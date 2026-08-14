//! Shared `#[cfg(test)]` helper for tests that need a real (not `connect_lazy`)
//! database: an in-memory SQLite pool with `buzz-db`'s migrations applied.
//!
//! Every module under `buzz-relay` used to eagerly `sqlx::PgPool::connect`
//! against a hardcoded local Postgres URL and `.ok()?`-skip the test if that
//! server wasn't reachable — a real external dependency for CI/dev. SQLite
//! removes that dependency entirely: an in-memory database with migrations
//! applied is always available, so tests that used to be conditionally
//! skipped now always run.

use sqlx::SqlitePool;

/// Opens a fresh in-memory SQLite pool with the full `buzz-db` schema applied.
///
/// Capped at 1 connection: a plain `sqlite::memory:` URL gives every pooled
/// connection its own private empty database, so a pool of size > 1 would
/// split reads/writes across unrelated in-memory databases.
pub(crate) async fn sqlite_test_pool() -> SqlitePool {
    let pool = sqlx::sqlite::SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await
        .expect("open in-memory sqlite pool");
    buzz_db::migration::run_migrations(&pool)
        .await
        .expect("run buzz-db migrations");
    pool
}
