//! Embedded semantic (vector) search for Buzz search events, backed by
//! [LanceDB](https://github.com/lancedb/lancedb).
//!
//! This is the **semantic sibling** of [`crate::query`]'s SQLite FTS5
//! search: `query` answers "which events contain these words"; this module
//! answers "which events are semantically close to this vector". The two
//! retrieval paths are independent — this module does not read or write
//! `events_fts` state, and `query` does not know this module exists.
//! Merging/ranking FTS hits together with vector hits into one result list
//! is a product decision left to the caller; see the "non-goals" note on
//! [`VectorSearchService`].
//!
//! ## Scope note: no embedding inference here
//!
//! This module does **not** compute embeddings. Every insert/upsert call
//! takes an already-computed `Vec<f32>`; turning message text into that
//! vector (model choice, batching, caching) is left to callers through the
//! [`EmbeddingGenerator`] trait placeholder. That trait's shape —
//! `embed_batch(&[&str]) -> Vec<Vec<f32>>` plus an explicit
//! `dimensions(): usize` — mirrors `xai-grok-memory`'s
//! `embedding::EmbeddingProvider` (see
//! `crates/codegen/xai-grok-memory/src/embedding.rs`) so a future adapter
//! does not need a translation layer between the two crates' vector
//! representations.
//!
//! ## Multi-tenant fence
//!
//! Every [`VectorRow`] and [`VectorSearchQuery`] carries a
//! [`buzz_core::CommunityId`], same discipline as `query::SearchQuery`.
//! [`VectorSearchService::search`] always predicates on `community_id`
//! first; there is no code path that returns cross-community hits. As with
//! FTS, the relay is expected to refetch the canonical event and re-run
//! access checks per hit — this module is a candidate generator, not an
//! access boundary.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use arrow_array::{
    types::Float32Type, Array, FixedSizeBinaryArray, FixedSizeListArray, Float32Array,
    Int64Array, RecordBatch, RecordBatchIterator, StringArray,
};
use arrow_schema::{ArrowError, DataType, Field, Schema, SchemaRef};
use buzz_core::CommunityId;
use futures::TryStreamExt;
use lancedb::query::{ExecutableQuery, QueryBase};
use lancedb::{connect, Connection, DistanceType, Table};
use thiserror::Error;
use uuid::Uuid;

/// On-disk table name inside the LanceDB database directory.
const TABLE_NAME: &str = "buzz_search_vectors";

/// Errors from the vector search service.
///
/// Kept separate from [`crate::SearchError`] (the FTS error type): the two
/// retrieval paths have independent failure modes (embedded LanceDB/Arrow
/// vs. SQLite/sqlx) and are developed independently — see module docs.
#[derive(Debug, Error)]
pub enum VectorSearchError {
    /// Error from the LanceDB client (I/O, schema, query execution, ...).
    #[error("lancedb error: {0}")]
    Lance(#[from] lancedb::Error),
    /// Error building/reading an Arrow `RecordBatch`.
    #[error("arrow error: {0}")]
    Arrow(#[from] ArrowError),
    /// Caller supplied a vector whose length doesn't match the table's
    /// configured embedding dimension.
    #[error("vector dimension mismatch: table expects {expected}, got {actual}")]
    DimensionMismatch {
        /// Dimension the table/service was opened with.
        expected: usize,
        /// Length of the vector the caller actually supplied.
        actual: usize,
    },
    /// The database directory path is not valid UTF-8 (LanceDB's connection
    /// URI is string-based).
    #[error("database path is not valid UTF-8: {0}")]
    InvalidPath(PathBuf),
    /// A stored `channel_id` column value was not a parseable UUID —
    /// indicates on-disk data written by something other than this module.
    #[error("channel_id column value is not a valid UUID: {0}")]
    InvalidChannelId(String),
}

/// A single message embedding row, ready to insert/upsert into the vector
/// table.
#[derive(Debug, Clone)]
pub struct VectorRow {
    /// 32-byte Nostr event id — same identity as `query::SearchHit::event_id`.
    pub event_id: [u8; 32],
    /// Server-resolved community. Required — see module docs.
    pub community_id: CommunityId,
    /// Optional channel UUID. `None` = channel-less event.
    pub channel_id: Option<Uuid>,
    /// Dense embedding vector. Length must equal the dimension the table
    /// was opened/created with (see [`VectorSearchService::open`]).
    pub embedding: Vec<f32>,
    /// Optional raw content, kept for debugging/echo only. Never used as
    /// an access boundary and not required for the ANN search itself.
    pub content: Option<String>,
    /// Unix seconds, same convention as `query::SearchHit::created_at`.
    pub created_at: i64,
}

/// A community-scoped nearest-neighbor query.
#[derive(Debug, Clone)]
pub struct VectorSearchQuery {
    /// Server-resolved community. Required at the type level, mirroring
    /// `query::SearchQuery::community`.
    pub community: CommunityId,
    /// Optional channel filter. `None` = no channel constraint (search the
    /// whole community). `Some(id)` restricts to that channel.
    pub channel_id: Option<Uuid>,
    /// Query embedding. Must match the table's configured dimension.
    pub vector: Vec<f32>,
    /// How many nearest neighbors to return.
    pub top_k: usize,
}

/// A single nearest-neighbor hit.
///
/// Like `query::SearchHit`, this is just enough to drive a canonical
/// refetch — the relay still owns access-control re-checks on any event id
/// returned here.
#[derive(Debug, Clone)]
pub struct VectorSearchHit {
    /// 32-byte event id.
    pub event_id: [u8; 32],
    /// Optional channel UUID echoed back from the row.
    pub channel_id: Option<Uuid>,
    /// Unix seconds.
    pub created_at: i64,
    /// Distance to the query vector (lower = closer). Metric is cosine
    /// distance — see [`VectorSearchService::search`].
    pub distance: f32,
}

/// Placeholder for future text→vector integration.
///
/// Deliberately not implemented by this module: choosing/calling an
/// embedding model is a separate product decision (see module docs). A
/// concrete implementation should call whatever embedding backend is
/// chosen and hand the resulting vectors to
/// [`VectorSearchService::upsert_many`].
#[async_trait::async_trait]
pub trait EmbeddingGenerator: Send + Sync {
    /// Embed a batch of texts, returning one vector per input, in order.
    async fn embed_batch(&self, texts: &[&str]) -> Result<Vec<Vec<f32>>, VectorSearchError>;

    /// Dimensionality of vectors this generator produces. Must match the
    /// dimension the target [`VectorSearchService`] was opened with.
    fn dimensions(&self) -> usize;
}

fn table_schema(dimensions: usize) -> SchemaRef {
    Arc::new(Schema::new(vec![
        Field::new("event_id", DataType::FixedSizeBinary(32), false),
        Field::new("community_id", DataType::Utf8, false),
        Field::new("channel_id", DataType::Utf8, true),
        Field::new(
            "embedding",
            DataType::FixedSizeList(
                Arc::new(Field::new("item", DataType::Float32, true)),
                dimensions as i32,
            ),
            false,
        ),
        Field::new("content", DataType::Utf8, true),
        Field::new("created_at", DataType::Int64, false),
    ]))
}

fn embedding_dimensions(schema: &Schema) -> usize {
    match schema
        .field_with_name("embedding")
        .expect("table_schema always defines an embedding column")
        .data_type()
    {
        DataType::FixedSizeList(_, dim) => *dim as usize,
        other => unreachable!("embedding column schema drifted to {other:?}"),
    }
}

/// Escapes a value for embedding into a LanceDB SQL `only_if` filter
/// literal. Callers only ever pass UUID `Display` output through this, so
/// there is no real injection surface today, but filters are still built as
/// strings so this stays defensive.
fn escape_sql_literal(value: &str) -> String {
    value.replace('\'', "''")
}

/// Thin handle around a local, embedded LanceDB connection + table for
/// community-scoped semantic search over Buzz events.
///
/// Mirrors `SearchService`'s role for FTS: a stable injection point for the
/// relay's `AppState`, holding nothing the underlying connection/table
/// don't already own.
///
/// ## Non-goals (see task scope)
///
/// - Does not generate embeddings (see [`EmbeddingGenerator`]).
/// - Does not merge/rank results together with FTS hits from
///   [`crate::query::search`] — callers combining both retrieval paths own
///   that ranking decision.
/// - Does not build an ANN index (`create_index`) — flat/exact search is
///   used, which LanceDB itself documents as fine up to roughly hundreds of
///   thousands of vectors. Adding an `IVF_PQ`/`HNSW` index is a follow-up
///   once table sizes warrant it; the query path here would not need to
///   change, only an index creation step at startup.
pub struct VectorSearchService {
    #[allow(dead_code)] // kept alive for the table's lifetime; no direct use yet.
    connection: Connection,
    table: Table,
    dimensions: usize,
}

impl VectorSearchService {
    /// Open (creating if absent) a local, embedded LanceDB database at
    /// `dir` and the `buzz_search_vectors` table inside it.
    ///
    /// `dir` is a directory path — LanceDB manages its own file layout
    /// inside it, similar in spirit to opening a SQLite file. Callers
    /// typically pass something like `<data_dir>/buzz-search-vectors.lance`.
    ///
    /// `dimensions` is only used to create the table's `embedding` column
    /// on first open; on an existing table the on-disk schema wins and a
    /// caller-supplied `dimensions` that disagrees with it will simply
    /// surface as [`VectorSearchError::DimensionMismatch`] on the first
    /// insert/search rather than silently truncating vectors.
    pub async fn open(
        dir: impl AsRef<Path>,
        dimensions: usize,
    ) -> Result<Self, VectorSearchError> {
        let dir = dir.as_ref();
        let uri = dir
            .to_str()
            .ok_or_else(|| VectorSearchError::InvalidPath(dir.to_path_buf()))?;
        let connection = connect(uri).execute().await?;

        let table = match connection.open_table(TABLE_NAME).execute().await {
            Ok(table) => table,
            Err(lancedb::Error::TableNotFound { .. }) => {
                connection
                    .create_empty_table(TABLE_NAME, table_schema(dimensions))
                    .execute()
                    .await?
            }
            Err(err) => return Err(err.into()),
        };

        // The on-disk schema is authoritative for an existing table; a
        // freshly created table's schema is exactly what we just asked for.
        let dimensions = embedding_dimensions(table.schema().await?.as_ref());

        Ok(Self {
            connection,
            table,
            dimensions,
        })
    }

    /// The embedding dimension this service's table is configured with
    /// (read back from the on-disk schema, not just the caller's request).
    pub fn dimensions(&self) -> usize {
        self.dimensions
    }

    /// Insert or update one row, keyed by `event_id`. Uses LanceDB's
    /// `merge_insert` so re-embedding the same event (e.g. after an edit)
    /// replaces the prior vector rather than appending a duplicate.
    pub async fn upsert(&self, row: VectorRow) -> Result<(), VectorSearchError> {
        self.upsert_many(std::iter::once(row)).await
    }

    /// Batch form of [`Self::upsert`]. Prefer this for bulk backfills — one
    /// `merge_insert` call for N rows instead of N round trips.
    pub async fn upsert_many(
        &self,
        rows: impl IntoIterator<Item = VectorRow>,
    ) -> Result<(), VectorSearchError> {
        let rows: Vec<VectorRow> = rows.into_iter().collect();
        if rows.is_empty() {
            return Ok(());
        }
        for row in &rows {
            if row.embedding.len() != self.dimensions {
                return Err(VectorSearchError::DimensionMismatch {
                    expected: self.dimensions,
                    actual: row.embedding.len(),
                });
            }
        }

        let schema = table_schema(self.dimensions);
        let batch = rows_to_batch(&rows, &schema)?;
        let reader = RecordBatchIterator::new(vec![Ok(batch)], schema);

        let mut merge_insert = self.table.merge_insert(&["event_id"]);
        merge_insert
            .when_matched_update_all(None)
            .when_not_matched_insert_all();
        merge_insert.execute(Box::new(reader)).await?;
        Ok(())
    }

    /// Community-scoped (optionally channel-scoped) nearest-neighbor
    /// search. Returns up to `query.top_k` hits ordered by ascending
    /// cosine distance (closest first).
    ///
    /// `community_id = ...` is always the first predicate, mirroring
    /// `query::search`'s multi-tenant fence — see module docs.
    pub async fn search(
        &self,
        query: &VectorSearchQuery,
    ) -> Result<Vec<VectorSearchHit>, VectorSearchError> {
        if query.vector.len() != self.dimensions {
            return Err(VectorSearchError::DimensionMismatch {
                expected: self.dimensions,
                actual: query.vector.len(),
            });
        }

        let mut filter = format!(
            "community_id = '{}'",
            escape_sql_literal(&query.community.to_string())
        );
        if let Some(channel_id) = query.channel_id {
            filter.push_str(&format!(
                " AND channel_id = '{}'",
                escape_sql_literal(&channel_id.to_string())
            ));
        }

        let top_k = query.top_k.max(1);
        let batches: Vec<RecordBatch> = self
            .table
            .query()
            .only_if(filter)
            .nearest_to(query.vector.as_slice())?
            .distance_type(DistanceType::Cosine)
            .limit(top_k)
            .execute()
            .await?
            .try_collect()
            .await?;

        let mut hits = Vec::new();
        for batch in &batches {
            hits.extend(batch_to_hits(batch)?);
        }
        hits.sort_by(|a, b| a.distance.total_cmp(&b.distance));
        hits.truncate(top_k);
        Ok(hits)
    }
}

fn rows_to_batch(
    rows: &[VectorRow],
    schema: &SchemaRef,
) -> Result<RecordBatch, VectorSearchError> {
    let dimensions = embedding_dimensions(schema);

    let event_ids = FixedSizeBinaryArray::try_from_iter(rows.iter().map(|r| r.event_id))?;
    let community_ids =
        StringArray::from_iter_values(rows.iter().map(|r| r.community_id.to_string()));
    let channel_ids: StringArray = rows
        .iter()
        .map(|r| r.channel_id.map(|id| id.to_string()))
        .collect();
    let embeddings = FixedSizeListArray::from_iter_primitive::<Float32Type, _, _>(
        rows.iter()
            .map(|r| Some(r.embedding.iter().copied().map(Some).collect::<Vec<_>>())),
        dimensions as i32,
    );
    let contents: StringArray = rows.iter().map(|r| r.content.clone()).collect();
    let created_ats = Int64Array::from_iter_values(rows.iter().map(|r| r.created_at));

    Ok(RecordBatch::try_new(
        schema.clone(),
        vec![
            Arc::new(event_ids),
            Arc::new(community_ids),
            Arc::new(channel_ids),
            Arc::new(embeddings),
            Arc::new(contents),
            Arc::new(created_ats),
        ],
    )?)
}

fn batch_to_hits(batch: &RecordBatch) -> Result<Vec<VectorSearchHit>, VectorSearchError> {
    let event_ids = batch
        .column_by_name("event_id")
        .and_then(|c| c.as_any().downcast_ref::<FixedSizeBinaryArray>())
        .expect("event_id column has drifted from FixedSizeBinary(32) schema");
    let channel_ids = batch
        .column_by_name("channel_id")
        .and_then(|c| c.as_any().downcast_ref::<StringArray>())
        .expect("channel_id column has drifted from Utf8 schema");
    let created_ats = batch
        .column_by_name("created_at")
        .and_then(|c| c.as_any().downcast_ref::<Int64Array>())
        .expect("created_at column has drifted from Int64 schema");
    let distances = batch
        .column_by_name("_distance")
        .and_then(|c| c.as_any().downcast_ref::<Float32Array>())
        .expect("LanceDB vector query result missing _distance column");

    let mut hits = Vec::with_capacity(batch.num_rows());
    for i in 0..batch.num_rows() {
        let event_id: [u8; 32] = event_ids
            .value(i)
            .try_into()
            .expect("event_id column is not 32 bytes");
        let channel_id = if channel_ids.is_null(i) {
            None
        } else {
            let raw = channel_ids.value(i);
            Some(
                Uuid::parse_str(raw)
                    .map_err(|_| VectorSearchError::InvalidChannelId(raw.to_string()))?,
            )
        };
        hits.push(VectorSearchHit {
            event_id,
            channel_id,
            created_at: created_ats.value(i),
            distance: distances.value(i),
        });
    }
    Ok(hits)
}

#[cfg(test)]
mod tests {
    use super::*;

    const DIMENSIONS: usize = 4;

    fn event_id(byte: u8) -> [u8; 32] {
        let mut id = [0u8; 32];
        id[0] = byte;
        id
    }

    async fn open_test_service() -> (tempfile::TempDir, VectorSearchService) {
        let dir = tempfile::tempdir().expect("tempdir");
        let db_path = dir.path().join("vectors.lance");
        let service = VectorSearchService::open(&db_path, DIMENSIONS)
            .await
            .expect("open vector search service");
        (dir, service)
    }

    #[tokio::test]
    async fn open_creates_table_with_requested_dimensions() {
        let (_dir, service) = open_test_service().await;
        assert_eq!(service.dimensions(), DIMENSIONS);
    }

    #[tokio::test]
    async fn upsert_then_search_returns_nearest_neighbor_first() {
        let (_dir, service) = open_test_service().await;
        let community = CommunityId::from_uuid(Uuid::new_v4());

        // Three orthogonal-ish vectors so cosine distance clearly orders them.
        let rows = vec![
            VectorRow {
                event_id: event_id(1),
                community_id: community,
                channel_id: None,
                embedding: vec![1.0, 0.0, 0.0, 0.0],
                content: Some("alpha".into()),
                created_at: 100,
            },
            VectorRow {
                event_id: event_id(2),
                community_id: community,
                channel_id: None,
                embedding: vec![0.0, 1.0, 0.0, 0.0],
                content: Some("beta".into()),
                created_at: 200,
            },
            VectorRow {
                event_id: event_id(3),
                community_id: community,
                channel_id: None,
                embedding: vec![0.9, 0.1, 0.0, 0.0],
                content: Some("alpha-ish".into()),
                created_at: 300,
            },
        ];
        service.upsert_many(rows).await.expect("upsert rows");

        let hits = service
            .search(&VectorSearchQuery {
                community,
                channel_id: None,
                vector: vec![1.0, 0.0, 0.0, 0.0],
                top_k: 2,
            })
            .await
            .expect("search");

        assert_eq!(hits.len(), 2);
        assert_eq!(hits[0].event_id, event_id(1));
        assert_eq!(hits[1].event_id, event_id(3));
        assert!(hits[0].distance <= hits[1].distance);
    }

    #[tokio::test]
    async fn search_is_scoped_to_community() {
        let (_dir, service) = open_test_service().await;
        let community_a = CommunityId::from_uuid(Uuid::new_v4());
        let community_b = CommunityId::from_uuid(Uuid::new_v4());

        service
            .upsert(VectorRow {
                event_id: event_id(1),
                community_id: community_a,
                channel_id: None,
                embedding: vec![1.0, 0.0, 0.0, 0.0],
                content: None,
                created_at: 1,
            })
            .await
            .expect("upsert a");
        service
            .upsert(VectorRow {
                event_id: event_id(2),
                community_id: community_b,
                channel_id: None,
                embedding: vec![1.0, 0.0, 0.0, 0.0],
                content: None,
                created_at: 2,
            })
            .await
            .expect("upsert b");

        let hits = service
            .search(&VectorSearchQuery {
                community: community_a,
                channel_id: None,
                vector: vec![1.0, 0.0, 0.0, 0.0],
                top_k: 10,
            })
            .await
            .expect("search");

        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].event_id, event_id(1));
    }

    #[tokio::test]
    async fn upsert_replaces_prior_embedding_for_same_event_id() {
        let (_dir, service) = open_test_service().await;
        let community = CommunityId::from_uuid(Uuid::new_v4());
        let id = event_id(1);

        service
            .upsert(VectorRow {
                event_id: id,
                community_id: community,
                channel_id: None,
                embedding: vec![1.0, 0.0, 0.0, 0.0],
                content: Some("v1".into()),
                created_at: 1,
            })
            .await
            .expect("initial upsert");
        service
            .upsert(VectorRow {
                event_id: id,
                community_id: community,
                channel_id: None,
                embedding: vec![0.0, 0.0, 0.0, 1.0],
                content: Some("v2".into()),
                created_at: 2,
            })
            .await
            .expect("re-upsert same event_id");

        let hits = service
            .search(&VectorSearchQuery {
                community,
                channel_id: None,
                vector: vec![0.0, 0.0, 0.0, 1.0],
                top_k: 10,
            })
            .await
            .expect("search");

        // Exactly one row for this event_id (no duplicate from the second
        // upsert), and it reflects the latest embedding/created_at.
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].created_at, 2);
    }

    #[tokio::test]
    async fn search_rejects_mismatched_vector_dimension() {
        let (_dir, service) = open_test_service().await;
        let err = service
            .search(&VectorSearchQuery {
                community: CommunityId::from_uuid(Uuid::new_v4()),
                channel_id: None,
                vector: vec![1.0, 0.0],
                top_k: 1,
            })
            .await
            .expect_err("dimension mismatch should error, not panic");
        assert!(matches!(
            err,
            VectorSearchError::DimensionMismatch {
                expected: DIMENSIONS,
                actual: 2
            }
        ));
    }
}
