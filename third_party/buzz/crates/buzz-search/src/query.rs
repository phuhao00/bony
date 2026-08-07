//! NIP-50 search query against SQLite FTS5, community-scoped.
//!
//! The relay never trusts a hit by itself: this layer returns canonical event
//! ids ordered by relevance, the relay refetches `StoredEvent`s through
//! buzz-db's `(community_id, event_id)` scoped fetcher, and runs the access
//! predicate (`search_hit_accepted` in `bridge.rs`) per hit. Search is never
//! the access boundary — it cannot widen visibility.
//!
//! See conformance row 50.

use buzz_core::CommunityId;
use sqlx::{QueryBuilder, Row, Sqlite, SqlitePool};
use uuid::Uuid;

use crate::error::SearchError;

/// Channel-scope filter for a community-scoped FTS query.
///
/// Four variants, 1-to-1 with the legacy `(accessible_channels: &[Uuid],
/// include_global: bool)` matrix from the Typesense relay:
///
/// | accessible | include_global | `ChannelScope` |
/// |---|---|---|
/// | non-empty  | true  | `ChannelsOrChannelLess(accessible)` |
/// | non-empty  | false | `Channels(accessible)`              |
/// | empty      | true  | `ChannelLessOnly`                   |
/// | empty      | false | (don't call — caller short-circuits to EOSE) |
///
/// `ChannelLessOnly` is the variant that the old `Option<Vec<Uuid>>` +
/// `bool` 2x2 could not express unambiguously: with empty accessible
/// channels and `include_global=true`, both `Some(vec![]) + true` and
/// `None + true` would broaden to all community channels rather than
/// restrict to channel-less events. The enum closes that hole at the
/// type level.
///
/// Empty-vec edge cases are intentionally not special-cased:
/// `Channels(vec![])` emits `channel_id IN ()` which is false-for-all-rows
/// (zero hits), and `ChannelsOrChannelLess(vec![])` emits `(channel_id IN ()
/// OR channel_id IS NULL)` which is equivalent to `ChannelLessOnly`.
#[derive(Debug, Clone)]
pub enum ChannelScope {
    /// No channel constraint. Matches every event in the community.
    Any,
    /// Restrict to `channel_id IS NULL` events only — what the legacy
    /// Typesense `channel_id:=__global__` sentinel meant.
    ChannelLessOnly,
    /// Restrict to events whose `channel_id` is in this list.
    Channels(Vec<Uuid>),
    /// Restrict to events whose `channel_id` is in this list, OR are
    /// channel-less (`channel_id IS NULL`).
    ChannelsOrChannelLess(Vec<Uuid>),
}

/// Search matching semantics.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SearchMode {
    /// Standard NIP-50-ish word/lexeme search: every whitespace-delimited
    /// token in the query must appear in the document (AND semantics),
    /// matching the old `websearch_to_tsquery` behavior for plain (no
    /// operator) input.
    FullText,
    /// Prefix-match the trailing normalized query token (`pro` matches `project`).
    ///
    /// Intended for bounded typeahead surfaces such as the desktop topbar. The
    /// relay still refetches and re-authorizes every hit; this mode changes only
    /// the candidate FTS5 query, not the access boundary.
    Prefix,
}

/// A community-scoped FTS query.
///
/// The community is REQUIRED at the type level — there is no construction path
/// that omits it. This is the search-side expression of conformance row zero:
/// every search call carries the server-resolved tenant, never client input.
#[derive(Debug, Clone)]
pub struct SearchQuery {
    /// Server-resolved community. Required.
    pub community: CommunityId,
    /// NIP-50 search text. Empty string is rejected by `search()` early
    /// (no hits, no SQL roundtrip).
    pub q: String,
    /// How to scope hits by channel. See [`ChannelScope`] — the four variants
    /// are 1-to-1 with the legacy `(accessible_channels, include_global)`
    /// matrix, and `ChannelLessOnly` closes the gap where "empty accessible
    /// channels + include global" used to silently broaden to all channels.
    pub channel_scope: ChannelScope,
    /// NIP-01 kinds filter. None = no kind constraint.
    pub kinds: Option<Vec<i32>>,
    /// NIP-01 authors filter (32-byte pubkeys). None = no author constraint.
    pub authors: Option<Vec<Vec<u8>>>,
    /// NIP-01 since (Unix seconds). Inclusive lower bound on created_at.
    pub since: Option<i64>,
    /// NIP-01 until (Unix seconds). Inclusive upper bound on created_at.
    pub until: Option<i64>,
    /// 1-indexed page number.
    pub page: u32,
    /// Page size. Clamped at 500 internally.
    pub per_page: u32,
    /// Matching semantics for the search text.
    pub mode: SearchMode,
}

/// A single FTS hit. The relay refetches the canonical `StoredEvent` and
/// re-authorizes; this struct is just enough to drive that fetch and preserve
/// relevance ordering.
#[derive(Debug, Clone)]
pub struct SearchHit {
    /// 32-byte event id.
    pub event_id: [u8; 32],
    /// Nostr kind.
    pub kind: i32,
    /// 32-byte pubkey of author.
    pub pubkey: [u8; 32],
    /// Optional channel UUID. `None` = channel-less event.
    pub channel_id: Option<Uuid>,
    /// Unix seconds.
    pub created_at: i64,
    /// Relevance score, higher = better (negated FTS5 `bm25()`, whose raw
    /// sign convention is the opposite — see `search()` below).
    pub rank: f32,
}

/// Result of a search.
#[derive(Debug, Clone)]
pub struct SearchResult {
    /// Hits on this page, ordered by relevance then created_at desc.
    pub hits: Vec<SearchHit>,
    /// 1-indexed page returned.
    pub page: u32,
}

const PER_PAGE_MAX: u32 = 500;
const PER_PAGE_DEFAULT: u32 = 100;
/// Hard cap on search text handed to the FTS5 tokenizer. This keeps a single
/// request from spending unbounded parser CPU/memory while still allowing far
/// longer queries than the desktop UI normally emits.
const SEARCH_TEXT_MAX_CHARS: usize = 4096;
/// Search pages are currently server-generated (WS uses 1..=MAX_SEARCH_PAGES,
/// bridge uses page 1), but clamp here too so a future caller cannot accidentally
/// wire untrusted input into a multi-trillion-row OFFSET.
const PAGE_MAX: u32 = 1000;

/// Escapes a single token as an FTS5 string literal (double-quoted): doubles
/// embedded `"` the way SQL string literals double embedded `'`. Wrapping
/// every token in quotes means FTS5's query-syntax operators (`AND`, `OR`,
/// `NOT`, `NEAR`, `col:`, `^`, `*`) inside raw user input are treated as
/// literal text, never parsed as operators — this is the injection boundary,
/// not a convenience.
fn quote_fts5_token(token: &str) -> String {
    let mut out = String::with_capacity(token.len() + 2);
    out.push('"');
    for ch in token.chars() {
        if ch == '"' {
            out.push('"');
        }
        out.push(ch);
    }
    out.push('"');
    out
}

/// A whitespace-delimited token carries no signal for the `unicode61`
/// tokenizer (and no signal for Postgres `to_tsquery`/`websearch_to_tsquery`
/// either) unless it contains at least one letter or digit. Quoting a
/// punctuation-only token (e.g. a lone `'` or `&`) still produces a
/// syntactically valid FTS5 phrase, but the tokenizer reduces it to zero
/// tokens, so an `AND`-ed chain that includes it can never match anything —
/// this must be filtered out here rather than sent to FTS5, mirroring the
/// old Postgres text-search parser silently dropping non-lexeme "words".
fn has_tokenizable_char(token: &str) -> bool {
    token.chars().any(|c| c.is_alphanumeric())
}

/// Builds the FTS5 `MATCH` query string for the search text, per [`SearchMode`].
///
/// `FullText` ANDs every whitespace-delimited token that carries at least one
/// letter/digit (quoted, so punctuation and FTS5 operators in the input are
/// literal — see [`has_tokenizable_char`] for why punctuation-only tokens are
/// dropped instead of quoted). `Prefix` keeps every completed token an exact
/// (quoted) match and turns only the trailing token into a prefix match —
/// FTS5 prefix syntax (`term*`) does not accept a quoted operand, so the
/// trailing token is restricted to word characters instead of being quoted.
fn build_match_query(mode: SearchMode, search_text: &str) -> Option<String> {
    let tokens: Vec<&str> = search_text.split_whitespace().collect();
    if tokens.is_empty() {
        return None;
    }

    match mode {
        SearchMode::FullText => {
            let parts: Vec<String> = tokens
                .iter()
                .filter(|t| has_tokenizable_char(t))
                .map(|t| quote_fts5_token(t))
                .collect();
            if parts.is_empty() {
                None
            } else {
                Some(parts.join(" AND "))
            }
        }
        SearchMode::Prefix => {
            let mut parts: Vec<String> = tokens[..tokens.len() - 1]
                .iter()
                .filter(|t| has_tokenizable_char(t))
                .map(|t| quote_fts5_token(t))
                .collect();
            let last = tokens[tokens.len() - 1];
            let prefix: String = last
                .chars()
                .filter(|c| c.is_alphanumeric() || *c == '_')
                .collect();
            if prefix.is_empty() {
                if parts.is_empty() {
                    return None;
                }
            } else {
                parts.push(format!("{prefix}*"));
            }
            if parts.is_empty() {
                None
            } else {
                Some(parts.join(" AND "))
            }
        }
    }
}

fn normalized_search_text(q: &str) -> Option<String> {
    let trimmed = q.trim();
    if trimmed.is_empty() {
        return None;
    }

    let mut cleaned = String::with_capacity(trimmed.len().min(SEARCH_TEXT_MAX_CHARS));
    for ch in trimmed.chars().take(SEARCH_TEXT_MAX_CHARS) {
        cleaned.push(if ch == '\0' { ' ' } else { ch });
    }

    let cleaned = cleaned.trim();
    if cleaned.is_empty() {
        None
    } else {
        Some(cleaned.to_string())
    }
}

fn push_hex_in_list(qb: &mut QueryBuilder<Sqlite>, values: &[Vec<u8>]) {
    qb.push("(");
    let mut sep = qb.separated(", ");
    for v in values {
        sep.push_bind(hex::encode(v));
    }
    sep.push_unseparated(")");
}

/// Execute a community-scoped FTS query.
///
/// SQL shape (always):
/// ```sql
/// SELECT event_id, kind, pubkey, channel_id, created_at, bm25(events_fts) AS rank
/// FROM events_fts
/// WHERE events_fts MATCH <mode-specific query>
///   AND community_id = $ctx
///   [+ channel scope, kinds, authors, since, until]
/// ORDER BY rank ASC, created_at DESC, event_id
/// LIMIT $per_page OFFSET (($page - 1) * $per_page)
/// ```
///
/// `community_id = $ctx` is a plain (UNINDEXED-column) predicate ANDed onto
/// the FTS5 `MATCH` — it is never expressible through the search text itself,
/// so there is no code path through this function that omits it.
///
/// `bm25()` returns a *smaller* (more negative) value for a *better* match;
/// [`SearchHit::rank`] negates it so callers keep the "higher = better"
/// convention the previous Postgres `ts_rank_cd`-based implementation used.
pub async fn search(pool: &SqlitePool, query: &SearchQuery) -> Result<SearchResult, SearchError> {
    let Some(search_text) = normalized_search_text(&query.q) else {
        return Ok(SearchResult {
            hits: Vec::new(),
            page: query.page.clamp(1, PAGE_MAX),
        });
    };

    let Some(match_query) = build_match_query(query.mode, &search_text) else {
        return Ok(SearchResult {
            hits: Vec::new(),
            page: query.page.clamp(1, PAGE_MAX),
        });
    };

    let per_page = query.per_page.clamp(1, PER_PAGE_MAX);
    let per_page_actual = if query.per_page == 0 {
        PER_PAGE_DEFAULT
    } else {
        per_page
    };
    let page = query.page.clamp(1, PAGE_MAX);
    let offset = ((page - 1) as i64) * (per_page_actual as i64);

    let mut qb: QueryBuilder<Sqlite> = QueryBuilder::new(
        "SELECT event_id, kind, pubkey, channel_id, created_at, bm25(events_fts) AS rank \
         FROM events_fts WHERE events_fts MATCH ",
    );
    qb.push_bind(match_query);
    qb.push(" AND community_id = ");
    qb.push_bind(query.community.as_uuid().to_string());

    // Channel scope — see `ChannelScope` doc for the four-case mapping. The
    // emitted SQL fragments are the SQLite `IN (...)` equivalents of the
    // legacy Postgres `= ANY(...)` shapes for the three carry-over cases;
    // `ChannelLessOnly` is the fence the old 2-tuple shape could not express.
    match &query.channel_scope {
        ChannelScope::Any => {
            // No channel constraint.
        }
        ChannelScope::ChannelLessOnly => {
            qb.push(" AND channel_id IS NULL");
        }
        ChannelScope::Channels(ids) => {
            qb.push(" AND channel_id IN (");
            let mut sep = qb.separated(", ");
            for id in ids {
                sep.push_bind(id.to_string());
            }
            sep.push_unseparated(")");
        }
        ChannelScope::ChannelsOrChannelLess(ids) => {
            qb.push(" AND (channel_id IN (");
            let mut sep = qb.separated(", ");
            for id in ids {
                sep.push_bind(id.to_string());
            }
            sep.push_unseparated(") OR channel_id IS NULL)");
        }
    }

    if let Some(ref kinds) = query.kinds {
        if !kinds.is_empty() {
            qb.push(" AND kind IN (");
            let mut sep = qb.separated(", ");
            for k in kinds {
                sep.push_bind(*k);
            }
            sep.push_unseparated(")");
        }
    }

    if let Some(ref authors) = query.authors {
        if !authors.is_empty() {
            qb.push(" AND pubkey IN ");
            push_hex_in_list(&mut qb, authors);
        }
    }

    if let Some(since) = query.since {
        qb.push(" AND created_at >= ");
        qb.push_bind(unix_seconds_to_rfc3339(since));
    }

    if let Some(until) = query.until {
        qb.push(" AND created_at <= ");
        qb.push_bind(unix_seconds_to_rfc3339(until));
    }

    qb.push(" ORDER BY rank ASC, created_at DESC, event_id LIMIT ");
    qb.push_bind(per_page_actual as i64);
    qb.push(" OFFSET ");
    qb.push_bind(offset);

    let rows = qb.build().fetch_all(pool).await?;

    let mut hits = Vec::with_capacity(rows.len());
    for row in rows {
        let id_hex: String = row.try_get("event_id")?;
        let pk_hex: String = row.try_get("pubkey")?;
        let id = decode_hex_32(&id_hex, "event_id")?;
        let pubkey = decode_hex_32(&pk_hex, "pubkey")?;
        let channel_id: Option<String> = row.try_get("channel_id")?;
        let channel_id = channel_id
            .map(|s| {
                Uuid::parse_str(&s).map_err(|e| {
                    sqlx::Error::Decode(format!("channel_id column is not a UUID: {e}").into())
                })
            })
            .transpose()?;
        let created_at_str: String = row.try_get("created_at")?;
        let created_at = chrono::DateTime::parse_from_rfc3339(&created_at_str)
            .map_err(|e| {
                sqlx::Error::Decode(format!("created_at column is not RFC3339: {e}").into())
            })?
            .timestamp();
        let bm25_rank: f64 = row.try_get("rank")?;

        hits.push(SearchHit {
            event_id: id,
            kind: row.try_get("kind")?,
            pubkey,
            channel_id,
            created_at,
            rank: -bm25_rank as f32,
        });
    }

    Ok(SearchResult { hits, page })
}

fn decode_hex_32(s: &str, column: &str) -> Result<[u8; 32], sqlx::Error> {
    let bytes = hex::decode(s)
        .map_err(|e| sqlx::Error::Decode(format!("{column} column is not hex: {e}").into()))?;
    bytes.try_into().map_err(|v: Vec<u8>| {
        sqlx::Error::Decode(format!("{column} column is {} bytes, expected 32", v.len()).into())
    })
}

fn unix_seconds_to_rfc3339(secs: i64) -> String {
    chrono::DateTime::from_timestamp(secs, 0)
        .unwrap_or_default()
        .to_rfc3339_opts(chrono::SecondsFormat::Millis, true)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalized_search_text_trims_and_rejects_empty() {
        assert_eq!(
            normalized_search_text("  hello  ").as_deref(),
            Some("hello")
        );
        assert!(normalized_search_text("   ").is_none());
    }

    #[test]
    fn normalized_search_text_replaces_nul_bytes() {
        assert_eq!(
            normalized_search_text("foo\0bar").as_deref(),
            Some("foo bar")
        );
    }

    #[test]
    fn normalized_search_text_caps_length() {
        let long = "x".repeat(SEARCH_TEXT_MAX_CHARS + 10);
        let cleaned = normalized_search_text(&long).expect("non-empty");
        assert_eq!(cleaned.chars().count(), SEARCH_TEXT_MAX_CHARS);
    }

    #[test]
    fn quote_fts5_token_escapes_embedded_quotes() {
        assert_eq!(quote_fts5_token("hello"), "\"hello\"");
        assert_eq!(quote_fts5_token("a\"b"), "\"a\"\"b\"");
    }

    #[test]
    fn build_match_query_full_text_ands_quoted_tokens() {
        let q = build_match_query(SearchMode::FullText, "hello world").unwrap();
        assert_eq!(q, "\"hello\" AND \"world\"");
    }

    #[test]
    fn build_match_query_full_text_neutralizes_operators_in_input() {
        // A raw `OR`/`NOT`/`NEAR`/column-filter token must stay literal text,
        // not be parsed as an FTS5 query operator.
        let q = build_match_query(SearchMode::FullText, "foo OR bar").unwrap();
        assert_eq!(q, "\"foo\" AND \"OR\" AND \"bar\"");
    }

    #[test]
    fn build_match_query_prefix_only_suffixes_trailing_token() {
        let q = build_match_query(SearchMode::Prefix, "hello wor").unwrap();
        assert_eq!(q, "\"hello\" AND wor*");
    }

    #[test]
    fn build_match_query_prefix_strips_punctuation_from_trailing_token() {
        let q = build_match_query(SearchMode::Prefix, "wor!!").unwrap();
        assert_eq!(q, "wor*");
    }

    #[test]
    fn build_match_query_rejects_all_whitespace() {
        assert!(build_match_query(SearchMode::FullText, "   ").is_none());
    }

    #[test]
    fn build_match_query_full_text_drops_punctuation_only_tokens() {
        // Punctuation-only tokens tokenize to nothing under `unicode61`; an
        // AND-chain that included them as quoted phrases could never match.
        let q = build_match_query(SearchMode::FullText, "alpha ' : & beta").unwrap();
        assert_eq!(q, "\"alpha\" AND \"beta\"");
    }

    #[test]
    fn build_match_query_rejects_all_punctuation() {
        assert!(build_match_query(SearchMode::FullText, "' : & | ( ) !").is_none());
    }

    #[test]
    fn build_match_query_prefix_drops_punctuation_only_middle_tokens() {
        let q = build_match_query(SearchMode::Prefix, "alpha ' beta be").unwrap();
        assert_eq!(q, "\"alpha\" AND \"beta\" AND be*");
    }

    // ── Integration tests against a real in-memory SQLite DB ────────────────
    // These exercise the actual `events_fts` triggers from `buzz-db`'s
    // migration, not just the query-builder logic above.

    async fn test_pool() -> SqlitePool {
        let pool = SqlitePool::connect("sqlite::memory:")
            .await
            .expect("open in-memory sqlite pool");
        buzz_db::migration::run_migrations(&pool)
            .await
            .expect("run migrations");
        pool
    }

    async fn insert_community(pool: &SqlitePool, id: Uuid, host: &str) {
        sqlx::query("INSERT INTO communities (id, host) VALUES (?, ?)")
            .bind(id.to_string())
            .bind(host)
            .execute(pool)
            .await
            .expect("insert community");
    }

    #[allow(clippy::too_many_arguments)]
    async fn insert_event(
        pool: &SqlitePool,
        community_id: Uuid,
        event_id: [u8; 32],
        pubkey: [u8; 32],
        kind: i32,
        content: &str,
        channel_id: Option<Uuid>,
        created_at: chrono::DateTime<chrono::Utc>,
    ) {
        sqlx::query(
            "INSERT INTO events (community_id, id, pubkey, created_at, kind, tags, content, sig, channel_id) \
             VALUES (?, ?, ?, ?, ?, '[]', ?, ?, ?)",
        )
        .bind(community_id.to_string())
        .bind(event_id.to_vec())
        .bind(pubkey.to_vec())
        .bind(created_at)
        .bind(kind)
        .bind(content)
        .bind(vec![0u8; 64])
        .bind(channel_id.map(|c| c.to_string()))
        .execute(pool)
        .await
        .expect("insert event");
    }

    fn id32(byte: u8) -> [u8; 32] {
        let mut a = [0u8; 32];
        a[0] = byte;
        a
    }

    #[tokio::test]
    async fn search_finds_inserted_event_by_content() {
        let pool = test_pool().await;
        let community = Uuid::new_v4();
        insert_community(&pool, community, "example.test").await;
        insert_event(
            &pool,
            community,
            id32(1),
            id32(0xAA),
            1,
            "hello from the buzz relay",
            None,
            chrono::Utc::now(),
        )
        .await;

        let result = search(
            &pool,
            &SearchQuery {
                community: CommunityId::from_uuid(community),
                q: "buzz relay".to_string(),
                channel_scope: ChannelScope::Any,
                kinds: None,
                authors: None,
                since: None,
                until: None,
                page: 1,
                per_page: 10,
                mode: SearchMode::FullText,
            },
        )
        .await
        .expect("search succeeds");

        assert_eq!(result.hits.len(), 1);
        assert_eq!(result.hits[0].event_id, id32(1));
        assert_eq!(result.hits[0].pubkey, id32(0xAA));
    }

    #[tokio::test]
    async fn search_is_scoped_to_community() {
        let pool = test_pool().await;
        let community_a = Uuid::new_v4();
        let community_b = Uuid::new_v4();
        insert_community(&pool, community_a, "a.test").await;
        insert_community(&pool, community_b, "b.test").await;
        insert_event(
            &pool,
            community_b,
            id32(2),
            id32(0xBB),
            1,
            "secret content in community b",
            None,
            chrono::Utc::now(),
        )
        .await;

        let result = search(
            &pool,
            &SearchQuery {
                community: CommunityId::from_uuid(community_a),
                q: "secret".to_string(),
                channel_scope: ChannelScope::Any,
                kinds: None,
                authors: None,
                since: None,
                until: None,
                page: 1,
                per_page: 10,
                mode: SearchMode::FullText,
            },
        )
        .await
        .expect("search succeeds");

        assert!(
            result.hits.is_empty(),
            "search text matching another community's content must not leak across communities"
        );
    }

    #[tokio::test]
    async fn soft_deleted_events_drop_out_of_search() {
        let pool = test_pool().await;
        let community = Uuid::new_v4();
        insert_community(&pool, community, "example.test").await;
        insert_event(
            &pool,
            community,
            id32(3),
            id32(0xCC),
            1,
            "ephemeral note about widgets",
            None,
            chrono::Utc::now(),
        )
        .await;

        sqlx::query("UPDATE events SET deleted_at = ? WHERE id = ?")
            .bind(chrono::Utc::now())
            .bind(id32(3).to_vec())
            .execute(&pool)
            .await
            .expect("soft delete");

        let result = search(
            &pool,
            &SearchQuery {
                community: CommunityId::from_uuid(community),
                q: "widgets".to_string(),
                channel_scope: ChannelScope::Any,
                kinds: None,
                authors: None,
                since: None,
                until: None,
                page: 1,
                per_page: 10,
                mode: SearchMode::FullText,
            },
        )
        .await
        .expect("search succeeds");

        assert!(result.hits.is_empty(), "soft-deleted events must not be searchable");
    }

    #[tokio::test]
    async fn prefix_mode_matches_typeahead_token() {
        let pool = test_pool().await;
        let community = Uuid::new_v4();
        insert_community(&pool, community, "example.test").await;
        insert_event(
            &pool,
            community,
            id32(4),
            id32(0xDD),
            1,
            "project kickoff notes",
            None,
            chrono::Utc::now(),
        )
        .await;

        let result = search(
            &pool,
            &SearchQuery {
                community: CommunityId::from_uuid(community),
                q: "proj".to_string(),
                channel_scope: ChannelScope::Any,
                kinds: None,
                authors: None,
                since: None,
                until: None,
                page: 1,
                per_page: 10,
                mode: SearchMode::Prefix,
            },
        )
        .await
        .expect("search succeeds");

        assert_eq!(result.hits.len(), 1);
        assert_eq!(result.hits[0].event_id, id32(4));
    }

    #[tokio::test]
    async fn channel_scope_filters_hits() {
        let pool = test_pool().await;
        let community = Uuid::new_v4();
        insert_community(&pool, community, "example.test").await;
        let channel_a = Uuid::new_v4();
        let channel_b = Uuid::new_v4();
        sqlx::query(
            "INSERT INTO channels (community_id, id, name, created_by) VALUES (?, ?, 'a', ?)",
        )
        .bind(community.to_string())
        .bind(channel_a.to_string())
        .bind(id32(0xEE).to_vec())
        .execute(&pool)
        .await
        .expect("insert channel a");
        sqlx::query(
            "INSERT INTO channels (community_id, id, name, created_by) VALUES (?, ?, 'b', ?)",
        )
        .bind(community.to_string())
        .bind(channel_b.to_string())
        .bind(id32(0xEE).to_vec())
        .execute(&pool)
        .await
        .expect("insert channel b");

        insert_event(
            &pool,
            community,
            id32(5),
            id32(0xEE),
            1,
            "message in channel a about rockets",
            Some(channel_a),
            chrono::Utc::now(),
        )
        .await;
        insert_event(
            &pool,
            community,
            id32(6),
            id32(0xEE),
            1,
            "message in channel b about rockets",
            Some(channel_b),
            chrono::Utc::now(),
        )
        .await;

        let result = search(
            &pool,
            &SearchQuery {
                community: CommunityId::from_uuid(community),
                q: "rockets".to_string(),
                channel_scope: ChannelScope::Channels(vec![channel_a]),
                kinds: None,
                authors: None,
                since: None,
                until: None,
                page: 1,
                per_page: 10,
                mode: SearchMode::FullText,
            },
        )
        .await
        .expect("search succeeds");

        assert_eq!(result.hits.len(), 1);
        assert_eq!(result.hits[0].event_id, id32(5));
    }

    #[tokio::test]
    async fn moderation_content_edit_updates_search_index() {
        let pool = test_pool().await;
        let community = Uuid::new_v4();
        insert_community(&pool, community, "example.test").await;
        insert_event(
            &pool,
            community,
            id32(7),
            id32(0xFF),
            1,
            "original wording here",
            None,
            chrono::Utc::now(),
        )
        .await;

        sqlx::query("UPDATE events SET content = ? WHERE id = ?")
            .bind("redacted placeholder text")
            .bind(id32(7).to_vec())
            .execute(&pool)
            .await
            .expect("content update");

        let stale = search(
            &pool,
            &SearchQuery {
                community: CommunityId::from_uuid(community),
                q: "original wording".to_string(),
                channel_scope: ChannelScope::Any,
                kinds: None,
                authors: None,
                since: None,
                until: None,
                page: 1,
                per_page: 10,
                mode: SearchMode::FullText,
            },
        )
        .await
        .expect("search succeeds");
        assert!(stale.hits.is_empty(), "old content must no longer match");

        let fresh = search(
            &pool,
            &SearchQuery {
                community: CommunityId::from_uuid(community),
                q: "redacted placeholder".to_string(),
                channel_scope: ChannelScope::Any,
                kinds: None,
                authors: None,
                since: None,
                until: None,
                page: 1,
                per_page: 10,
                mode: SearchMode::FullText,
            },
        )
        .await
        .expect("search succeeds");
        assert_eq!(fresh.hits.len(), 1);
        assert_eq!(fresh.hits[0].event_id, id32(7));
    }
}
