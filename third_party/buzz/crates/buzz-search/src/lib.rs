#![deny(unsafe_code)]
#![warn(missing_docs)]
//! Buzz search — community-scoped SQLite FTS5 full-text search over Buzz events.
//!
//! The index lives in a standalone `events_fts` FTS5 virtual table (see
//! `migrations/0001_initial_schema.sql` in `buzz-db`), kept in sync by
//! `AFTER INSERT/UPDATE/DELETE` triggers on `events`. Because the sync is a
//! database trigger rather than an application-level write path, every row
//! write *is* the index update — there is no separate indexer, no mpsc
//! queue, no reindex job, no consistency window to reason about. A client
//! cannot forge the FTS row out of sync with the content it signed.
//!
//! This crate is the **query** side. Indexing is the trigger-maintained
//! `events_fts` table — owned by `buzz-db`'s migration. The relay refetches
//! canonical events through `buzz-db`'s scoped fetcher and runs access
//! checks per hit; search is never the access boundary (conformance row 50).
//!
//! ## Multi-tenant fence
//!
//! Every [`SearchQuery`] carries a [`CommunityId`]. There is no construction
//! path through this crate that omits it, and every SQL execution binds
//! `community_id = $ctx` as the first predicate. A query bound to community
//! A cannot return events stored under community B, by construction.

/// Search error types.
pub mod error;
/// Search query execution.
pub mod query;
/// Embedded LanceDB-backed semantic (vector) search — see module docs.
pub mod vector;

pub use buzz_core::CommunityId;
pub use error::SearchError;
pub use query::{search, ChannelScope, SearchHit, SearchMode, SearchQuery, SearchResult};
pub use vector::{
    EmbeddingGenerator, VectorRow, VectorSearchError, VectorSearchHit, VectorSearchQuery,
    VectorSearchService,
};

use sqlx::SqlitePool;

/// Thin handle around a `SqlitePool` for community-scoped FTS.
///
/// Holds nothing the pool itself doesn't already own. The whole purpose of
/// this type is a stable injection point for the relay's `AppState`.
#[derive(Debug, Clone)]
pub struct SearchService {
    pool: SqlitePool,
}

impl SearchService {
    /// Build a search service over an existing SQLite pool.
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }

    /// Execute a community-scoped FTS query.
    pub async fn search(&self, query: &SearchQuery) -> Result<SearchResult, SearchError> {
        query::search(&self.pool, query).await
    }
}
