//! Embedded SQLx migrations for Buzz (SQLite).
//!
//! Single-instance deployment target: one consolidated migration
//! (`migrations/0001_initial_schema.sql`) is the schema's sole source of
//! truth. There is no legacy Postgres data to replay forward — this is a
//! fresh SQLite database on every install — so the previous 27-version
//! Postgres migration history was replaced wholesale rather than translated
//! version-by-version. See the migration file's header comment for the
//! cross-engine mapping decisions (UUID -> TEXT, BYTEA -> BLOB, etc.) and the
//! multi-pod infrastructure (replica fence, table partitioning, advisory
//! locks) that was dropped along with Postgres.

use sqlx::SqlitePool;

use crate::Result;

static MIGRATOR: sqlx::migrate::Migrator = sqlx::migrate!("../../migrations");

/// Run all pending Buzz database migrations.
pub async fn run_migrations(pool: &SqlitePool) -> Result<()> {
    MIGRATOR.run(pool).await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn embedded_migrator_contains_the_consolidated_sqlite_schema() {
        let mut migrations: Vec<_> = MIGRATOR.iter().collect();
        migrations.sort_by_key(|migration| migration.version);

        assert_eq!(
            migrations.len(),
            1,
            "single-instance SQLite deployment carries exactly one consolidated migration"
        );
        assert_eq!(migrations[0].version, 1);
        let sql = migrations[0].sql.as_ref();

        for table in [
            "communities",
            "channels",
            "channel_members",
            "users",
            "events",
            "event_mentions",
            "parameterized_event_watermarks",
            "subscriptions",
            "delivery_log",
            "workflows",
            "workflow_runs",
            "workflow_approvals",
            "scheduled_workflow_fires",
            "api_tokens",
            "rate_limit_violations",
            "thread_metadata",
            "reactions",
            "pubkey_allowlist",
            "relay_members",
            "join_policy_acceptances",
            "relay_invites",
            "archived_identities",
            "audit_log",
            "moderation_reports",
            "community_bans",
            "moderation_actions",
            "_operator_global_tables",
            "push_leases",
            "push_wake_outbox",
            "push_match_queue",
            "push_gateway_challenges",
            "push_gateway_installations",
            "push_gateway_delegations",
            "push_gateway_endpoint_quotas",
            "push_gateway_delivery_auth_replays",
            "push_gateway_delivery_request_replays",
            "product_feedback",
            "git_repo_names",
        ] {
            assert!(
                sql.contains(&format!("CREATE TABLE {table}")),
                "consolidated migration must create table {table}"
            );
        }

        assert!(
            sql.contains("CREATE VIRTUAL TABLE events_fts USING fts5"),
            "consolidated migration must create the events_fts FTS5 index"
        );

        // No partitioning, no replica fence, no advisory locks, no
        // Postgres-only column types — this is the SQLite single-instance
        // schema, not a translated-in-place Postgres one.
        for forbidden in [
            "PARTITION BY",
            "PARTITION OF",
            "pg_advisory",
            "TSVECTOR",
            "JSONB",
            "BYTEA",
            "TIMESTAMPTZ",
            "gen_random_uuid",
            "replica_heartbeat",
        ] {
            assert!(
                !sql.contains(forbidden),
                "consolidated SQLite migration must not contain Postgres-only construct: {forbidden}"
            );
        }
    }

    #[tokio::test]
    async fn run_migrations_applies_consolidated_schema_on_fresh_database() {
        let pool = SqlitePool::connect("sqlite::memory:")
            .await
            .expect("open in-memory sqlite pool");

        run_migrations(&pool).await.expect("run migrations");

        for table in ["communities", "events", "events_fts", "channels", "scheduled_workflow_fires", "audit_log"] {
            let exists: Option<String> = sqlx::query_scalar(
                "SELECT name FROM sqlite_master WHERE type = 'table' AND name = ?",
            )
            .bind(table)
            .fetch_optional(&pool)
            .await
            .unwrap_or_else(|err| panic!("check table {table}: {err}"));
            assert!(exists.is_some(), "migration should create {table}");
        }

        // Running twice must be idempotent (sqlx tracks applied versions).
        run_migrations(&pool).await.expect("re-run is a no-op");
    }
}
