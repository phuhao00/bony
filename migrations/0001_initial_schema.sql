-- Buzz initial SQLite schema — multi-tenant, single-instance deployment.
--
-- This is a from-scratch SQLite translation of the Postgres desired-state
-- schema (schema/schema.sql). There is no legacy Postgres data to preserve —
-- this deployment target is a fresh local/desktop SQLite database, so the
-- previous 27-version Postgres migration history was replaced by this one
-- consolidated migration rather than replayed statement-by-statement.
--
-- Governing contract: docs/multi-tenant-conformance.md. Every table below
-- keeps the same tenant-isolation shape as the Postgres schema: every
-- scoped row carries an immutable `community_id`, and every
-- UNIQUE/PRIMARY KEY/FK on a scoped table leads with it.
--
-- ── Cross-engine mapping decisions (kept consistent across every module) ──
--   uuid          -> TEXT (canonical lowercase hyphenated string; generated
--                    in Rust with `uuid::Uuid::new_v4()`, never DB-side —
--                    SQLite has no server-side random-UUID generator).
--   bytea         -> BLOB
--   timestamptz   -> TEXT (RFC3339 UTC; `chrono::DateTime<Utc>` maps to this
--                    natively via sqlx's sqlite+chrono integration).
--   jsonb         -> TEXT (JSON text; queries use SQLite's `json_extract` /
--                    `json_each` instead of `->`/`->>`/`jsonb_array_elements`).
--   enum types    -> TEXT + CHECK(value IN (...)) inline on the column.
--   BOOLEAN       -> INTEGER (SQLite has no native boolean storage class;
--                    the `BOOLEAN`/`TRUE`/`FALSE` spellings below are kept
--                    for readability — SQLite 3.23+ accepts them as INTEGER
--                    affinity + literals 1/0).
--   octet_length()-> length() (SQLite's length() on a BLOB returns bytes).
--   IDENTITY cols -> INTEGER PRIMARY KEY AUTOINCREMENT (rowid alias).
--
-- ── Removed entirely (multi-pod / multi-replica infrastructure, not needed
--    for a single-instance deployment) ──
--   - Monthly table partitioning (`events`, `delivery_log`): plain tables
--     with the same indexes; buzz-db's `partition.rs` module is deleted.
--   - The replica freshness fence (the replica-heartbeat table, the commit-time
--     `created_at` floor guard, `pg_stat_activity` probing): buzz-db's
--     `replica_fence.rs` module is deleted; `Db` no longer supports a
--     second read-replica pool.
--   - Per-community/per-channel Postgres session-advisory-lock functions:
--     SQLite already serializes writers at the connection level for a
--     single-instance deployment, so the lock-then-check dance these existed
--     to avoid starving is unnecessary. The triggers below do the equivalent
--     insert/update directly.
--   - `search_tsv` (Postgres `tsvector` generated column + GIN index): full
--     text search moves to a SQLite FTS5 external-content table
--     (`events_fts`), owned by buzz-search — see that crate's migration
--     when it lands. This migration keeps `events` search-column-free.
--
-- Composite-key covering indexes lose Postgres's `INCLUDE (...)` clause —
-- SQLite has no covering-index syntax — but scale in a single-instance
-- desktop deployment does not need it.

PRAGMA foreign_keys = ON;

-- ── Communities ───────────────────────────────────────────────────────────
-- Operator-global: the tenant registry itself; `id` IS the community key.

CREATE TABLE communities (
    id              TEXT PRIMARY KEY,
    host            TEXT NOT NULL,
    signing_key     BLOB,
    icon            TEXT,
    created_at      TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    archived_at     TEXT,
    CHECK (id <> '00000000-0000-0000-0000-000000000000')
);

CREATE UNIQUE INDEX idx_communities_host ON communities (lower(host));

-- ── Channels ──────────────────────────────────────────────────────────────
-- `community_id` immutable after insert (trigger below; no UPDATE path).
-- PK is (community_id, id): the same channel UUID may legitimately exist in
-- two communities.

CREATE TABLE channels (
    id               TEXT NOT NULL,
    community_id     TEXT NOT NULL REFERENCES communities(id),
    name             TEXT NOT NULL,
    channel_type     TEXT NOT NULL DEFAULT 'stream'
                         CHECK (channel_type IN ('stream', 'forum', 'dm', 'workflow')),
    visibility       TEXT NOT NULL DEFAULT 'open'
                         CHECK (visibility IN ('open', 'private')),
    description      TEXT,
    canvas           TEXT,
    created_by       BLOB NOT NULL,
    created_at       TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    updated_at       TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    archived_at      TEXT,
    deleted_at       TEXT,
    nip29_group_id   TEXT,
    topic_required   BOOLEAN NOT NULL DEFAULT FALSE,
    max_members      INTEGER,
    topic            TEXT,
    topic_set_by     BLOB,
    topic_set_at     TEXT,
    purpose          TEXT,
    purpose_set_by   BLOB,
    purpose_set_at   TEXT,
    participant_hash BLOB,
    ttl_seconds      INTEGER,
    ttl_deadline     TEXT,
    PRIMARY KEY (community_id, id),
    CHECK (id <> '00000000-0000-0000-0000-000000000000')
);

CREATE UNIQUE INDEX idx_channels_nip29_group ON channels (community_id, nip29_group_id)
    WHERE nip29_group_id IS NOT NULL;
CREATE UNIQUE INDEX idx_channels_dm_hash ON channels (community_id, participant_hash)
    WHERE participant_hash IS NOT NULL;
CREATE INDEX idx_channels_community_type ON channels (community_id, channel_type);
CREATE INDEX idx_channels_community_visibility ON channels (community_id, visibility);
CREATE INDEX idx_channels_created_by ON channels (community_id, created_by);
CREATE INDEX idx_channels_ttl_expiry ON channels (ttl_deadline)
    WHERE ttl_seconds IS NOT NULL AND archived_at IS NULL AND deleted_at IS NULL;
-- Tenant-independent channel-id -> community lookups carry no community_id
-- predicate, so no community_id-leading index can serve them. Not UNIQUE —
-- the same channel id may exist under more than one community.
CREATE INDEX idx_channels_id_live ON channels (id)
    WHERE deleted_at IS NULL;

CREATE TRIGGER trg_channels_community_id_immutable
BEFORE UPDATE ON channels
FOR EACH ROW WHEN NEW.community_id IS NOT OLD.community_id
BEGIN
    SELECT RAISE(ABORT, 'channels.community_id is immutable (channel cannot be re-tenanted)');
END;

-- ── Channel members ───────────────────────────────────────────────────────

CREATE TABLE channel_members (
    community_id TEXT NOT NULL REFERENCES communities(id),
    channel_id  TEXT NOT NULL,
    pubkey      BLOB NOT NULL,
    role        TEXT NOT NULL DEFAULT 'member'
                    CHECK (role IN ('owner', 'admin', 'member', 'guest', 'bot')),
    joined_at   TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    invited_by  BLOB,
    removed_at  TEXT,
    removed_by  BLOB,
    hidden_at   TEXT,
    PRIMARY KEY (community_id, channel_id, pubkey),
    FOREIGN KEY (community_id, channel_id)
        REFERENCES channels (community_id, id) ON DELETE CASCADE
);

CREATE INDEX idx_channel_members_pubkey ON channel_members (community_id, pubkey)
    WHERE removed_at IS NULL;

-- ── Users ─────────────────────────────────────────────────────────────────

CREATE TABLE users (
    community_id        TEXT NOT NULL REFERENCES communities(id),
    pubkey              BLOB NOT NULL,
    nip05_handle        TEXT,
    display_name        TEXT,
    avatar_url          TEXT,
    about               TEXT,
    agent_type          TEXT,
    capabilities        TEXT,
    okta_user_id        TEXT,
    created_at          TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    updated_at          TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    deactivated_at      TEXT,
    metadata_event_id   BLOB,
    agent_owner_pubkey  BLOB,
    channel_add_policy  TEXT NOT NULL DEFAULT 'anyone'
                            CHECK (channel_add_policy IN ('anyone', 'owner_only', 'nobody')),
    PRIMARY KEY (community_id, pubkey),
    CHECK (length(pubkey) = 32),
    FOREIGN KEY (community_id, agent_owner_pubkey)
        REFERENCES users (community_id, pubkey) ON DELETE SET NULL
);

CREATE UNIQUE INDEX idx_users_nip05 ON users (community_id, lower(nip05_handle))
    WHERE nip05_handle IS NOT NULL;
CREATE UNIQUE INDEX idx_users_okta ON users (community_id, okta_user_id)
    WHERE okta_user_id IS NOT NULL;

-- ── Events (single table — no monthly partitioning) ──────────────────────
-- Cross-community dedup: the same signed event may exist in two
-- communities; (community_id, created_at, id) dedupes within one, allows
-- across. Full-text search (`search_tsv` in Postgres) moves to an FTS5
-- external-content table owned by buzz-search, not this table.

CREATE TABLE events (
    community_id TEXT NOT NULL REFERENCES communities(id),
    id          BLOB NOT NULL,
    pubkey      BLOB NOT NULL,
    created_at  TEXT NOT NULL,
    kind        INTEGER NOT NULL,
    tags        TEXT NOT NULL,
    content     TEXT NOT NULL,
    sig         BLOB NOT NULL,
    received_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    channel_id  TEXT,
    deleted_at  TEXT,
    d_tag       TEXT,
    not_before  INTEGER,
    delivered_at INTEGER,
    PRIMARY KEY (community_id, created_at, id)
);

CREATE INDEX idx_events_community_id ON events (community_id, id, created_at DESC);
CREATE INDEX idx_events_community_channel_created
    ON events (community_id, channel_id, created_at DESC, id);
CREATE INDEX idx_events_community_pubkey_kind_created
    ON events (community_id, pubkey, kind, created_at DESC, id);
CREATE INDEX idx_events_community_kind_created
    ON events (community_id, kind, created_at DESC, id);
CREATE INDEX idx_events_community_deleted ON events (community_id, deleted_at);
CREATE INDEX idx_events_addressable
    ON events (community_id, kind, pubkey, channel_id, deleted_at);
CREATE INDEX idx_events_parameterized
    ON events (community_id, kind, pubkey, d_tag, created_at DESC, id)
    WHERE d_tag IS NOT NULL AND deleted_at IS NULL;
CREATE INDEX idx_events_not_before ON events (community_id, not_before)
    WHERE not_before IS NOT NULL AND deleted_at IS NULL AND delivered_at IS NULL;

-- ── Event mentions ────────────────────────────────────────────────────────

CREATE TABLE event_mentions (
    community_id        TEXT NOT NULL REFERENCES communities(id),
    pubkey_hex          TEXT NOT NULL,
    event_id            BLOB NOT NULL,
    event_created_at    TEXT NOT NULL,
    channel_id          TEXT,
    event_kind          INTEGER,
    PRIMARY KEY (community_id, pubkey_hex, event_id)
);

CREATE INDEX idx_event_mentions_pubkey_created
    ON event_mentions (community_id, pubkey_hex, event_created_at DESC);
CREATE INDEX idx_event_mentions_pubkey_kind_created
    ON event_mentions (community_id, pubkey_hex, event_kind, event_created_at DESC);

-- `event_mentions.event_id` has no declared foreign key to `events` (mention
-- writes are best-effort and run outside the event's own transaction — see
-- `insert_mentions` in `buzz-db`'s lib.rs), so a legacy writer that computed
-- mentions before an event was purged can still attempt this insert after
-- the event row is gone. Silently drop such orphaned mentions instead of
-- creating a dangling reference: `RAISE(IGNORE)` abandons just this row with
-- no error, so the caller sees a normal 0-row-affected insert.
CREATE TRIGGER event_mentions_skip_orphaned_insert
BEFORE INSERT ON event_mentions
WHEN NOT EXISTS (
    SELECT 1 FROM events WHERE community_id = NEW.community_id AND id = NEW.event_id
)
BEGIN
    SELECT RAISE(IGNORE);
END;

-- ── NIP-RS (kind:30078 read-state) durable ordering watermark ──────────────
-- Bounds read-state storage while preserving NIP-33 replacement ordering: the
-- payload itself has no historical product value, so superseded/deleted rows
-- are hard-deleted (see `events_purge_soft_deleted_nip_rs` below) and this
-- compact per-coordinate high-water mark is all that is kept to reject a
-- stale replay after its payload is gone. Maintained by `buzz-db`'s
-- `replace_parameterized_event` in the same transaction as the event write.
CREATE TABLE parameterized_event_watermarks (
    community_id  TEXT NOT NULL REFERENCES communities(id),
    kind          INTEGER NOT NULL,
    pubkey        BLOB NOT NULL,
    d_tag         TEXT NOT NULL,
    created_at    TEXT NOT NULL,
    event_id      BLOB NOT NULL,
    PRIMARY KEY (community_id, kind, pubkey, d_tag)
);

-- ── Subscriptions ─────────────────────────────────────────────────────────

CREATE TABLE subscriptions (
    community_id        TEXT NOT NULL REFERENCES communities(id),
    id                  TEXT NOT NULL,
    owner_pubkey        BLOB NOT NULL,
    filter_kinds        TEXT,
    filter_authors      TEXT,
    filter_channel_ids  TEXT,
    filter_since        TEXT,
    filter_until        TEXT,
    delivery_method     TEXT NOT NULL DEFAULT 'webhook'
                            CHECK (delivery_method IN ('webhook', 'websocket')),
    delivery_url        TEXT,
    status              TEXT NOT NULL DEFAULT 'active'
                            CHECK (status IN ('active', 'paused', 'deleted')),
    pause_reason        TEXT CHECK (pause_reason IS NULL OR pause_reason IN ('user', 'system', 'rate_limit')),
    delivered_count     INTEGER NOT NULL DEFAULT 0,
    error_count         INTEGER NOT NULL DEFAULT 0,
    created_at          TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    updated_at          TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    PRIMARY KEY (community_id, id),
    FOREIGN KEY (community_id, owner_pubkey) REFERENCES users (community_id, pubkey)
);

-- ── Delivery log (single table — no monthly partitioning) ────────────────

CREATE TABLE delivery_log (
    id              INTEGER PRIMARY KEY AUTOINCREMENT,
    community_id    TEXT NOT NULL REFERENCES communities(id),
    subscription_id TEXT,
    event_id        BLOB,
    method          TEXT CHECK (method IS NULL OR method IN ('webhook', 'websocket')),
    delivered_at    TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    success         BOOLEAN,
    http_status     INTEGER,
    error_message   TEXT,
    attempt_number  INTEGER DEFAULT 1
);

CREATE INDEX idx_delivery_log_community_sub ON delivery_log (community_id, subscription_id);
CREATE INDEX idx_delivery_log_delivered_at ON delivery_log (delivered_at);

-- ── Workflows ─────────────────────────────────────────────────────────────

CREATE TABLE workflows (
    community_id    TEXT NOT NULL REFERENCES communities(id),
    id              TEXT NOT NULL,
    name            TEXT NOT NULL,
    owner_pubkey    BLOB NOT NULL,
    channel_id      TEXT,
    definition      TEXT NOT NULL,
    definition_hash BLOB NOT NULL,
    status          TEXT NOT NULL DEFAULT 'active'
                        CHECK (status IN ('active', 'disabled', 'archived')),
    enabled         BOOLEAN NOT NULL DEFAULT TRUE,
    created_at      TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    updated_at      TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    PRIMARY KEY (community_id, id),
    FOREIGN KEY (community_id, owner_pubkey) REFERENCES users (community_id, pubkey),
    FOREIGN KEY (community_id, channel_id) REFERENCES channels (community_id, id)
);

CREATE INDEX idx_workflows_channel_active ON workflows (community_id, channel_id, status, enabled);
CREATE INDEX idx_workflows_enabled ON workflows (enabled, status) WHERE enabled;

-- ── Workflow runs ─────────────────────────────────────────────────────────

CREATE TABLE workflow_runs (
    community_id        TEXT NOT NULL REFERENCES communities(id),
    id                  TEXT NOT NULL,
    workflow_id         TEXT NOT NULL,
    status              TEXT NOT NULL DEFAULT 'pending'
                            CHECK (status IN ('pending', 'running', 'waiting_approval', 'completed', 'failed', 'cancelled')),
    trigger_event_id    BLOB,
    current_step        INTEGER NOT NULL DEFAULT 0,
    execution_trace     TEXT NOT NULL DEFAULT '[]',
    trigger_context     TEXT,
    started_at          TEXT,
    completed_at        TEXT,
    error_message       TEXT,
    created_at          TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    PRIMARY KEY (community_id, id),
    FOREIGN KEY (community_id, workflow_id)
        REFERENCES workflows (community_id, id) ON DELETE CASCADE
);

CREATE INDEX idx_workflow_runs_workflow ON workflow_runs (community_id, workflow_id);
CREATE INDEX idx_workflow_runs_status ON workflow_runs (community_id, status);

-- ── Workflow approvals ────────────────────────────────────────────────────

CREATE TABLE workflow_approvals (
    community_id    TEXT NOT NULL REFERENCES communities(id),
    token           BLOB NOT NULL,
    workflow_id     TEXT NOT NULL,
    run_id          TEXT NOT NULL,
    step_id         TEXT NOT NULL,
    step_index      INTEGER NOT NULL,
    approver_spec   TEXT NOT NULL,
    status          TEXT NOT NULL DEFAULT 'pending'
                        CHECK (status IN ('pending', 'granted', 'denied', 'expired')),
    approver_pubkey BLOB,
    note            TEXT,
    granted_at      TEXT,
    denied_at       TEXT,
    expires_at      TEXT NOT NULL,
    created_at      TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    PRIMARY KEY (community_id, token),
    FOREIGN KEY (community_id, workflow_id)
        REFERENCES workflows (community_id, id) ON DELETE CASCADE,
    FOREIGN KEY (community_id, run_id)
        REFERENCES workflow_runs (community_id, id) ON DELETE CASCADE
);

CREATE INDEX idx_workflow_approvals_workflow ON workflow_approvals (community_id, workflow_id);
CREATE INDEX idx_workflow_approvals_run ON workflow_approvals (community_id, run_id);
CREATE INDEX idx_workflow_approvals_status ON workflow_approvals (community_id, status);

-- ── Scheduled workflow fires (cron claim) ─────────────────────────────────
-- At-most-once cron fire claim: PRIMARY KEY (community_id, workflow_id,
-- scheduled_for) — only the insert that wins the claim creates the run.

CREATE TABLE scheduled_workflow_fires (
    community_id    TEXT NOT NULL REFERENCES communities(id),
    workflow_id     TEXT NOT NULL,
    scheduled_for   TEXT NOT NULL,
    claimed_at      TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    workflow_run_id TEXT,
    PRIMARY KEY (community_id, workflow_id, scheduled_for),
    FOREIGN KEY (community_id, workflow_id)
        REFERENCES workflows (community_id, id) ON DELETE CASCADE,
    FOREIGN KEY (community_id, workflow_run_id)
        REFERENCES workflow_runs (community_id, id) ON DELETE NO ACTION
);

CREATE INDEX idx_scheduled_fires_claimed_at ON scheduled_workflow_fires (claimed_at);

-- ── API tokens ────────────────────────────────────────────────────────────

CREATE TABLE api_tokens (
    community_id        TEXT NOT NULL REFERENCES communities(id),
    id                  TEXT NOT NULL,
    token_hash          BLOB NOT NULL,
    owner_pubkey        BLOB NOT NULL,
    name                TEXT NOT NULL,
    scopes              TEXT NOT NULL,
    channel_ids         TEXT,
    created_at          TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    expires_at          TEXT,
    last_used_at        TEXT,
    revoked_at          TEXT,
    revoked_by          BLOB,
    created_by_self_mint BOOLEAN NOT NULL DEFAULT FALSE,
    PRIMARY KEY (community_id, id),
    FOREIGN KEY (community_id, owner_pubkey) REFERENCES users (community_id, pubkey),
    CHECK (length(token_hash) = 32)
);

CREATE UNIQUE INDEX idx_api_tokens_hash ON api_tokens (community_id, token_hash);

-- ── Rate limit violations ─────────────────────────────────────────────────
-- Operator-global: deployment abuse/health, never tenant-observable.

CREATE TABLE rate_limit_violations (
    id              INTEGER PRIMARY KEY AUTOINCREMENT,
    community_id    TEXT,
    pubkey          BLOB,
    violation_at    TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    limit_type      TEXT,
    limit_value     INTEGER,
    actual_value    INTEGER,
    action_taken    TEXT
);

-- ── Thread metadata ───────────────────────────────────────────────────────

CREATE TABLE thread_metadata (
    community_id            TEXT NOT NULL REFERENCES communities(id),
    event_created_at        TEXT NOT NULL,
    event_id                BLOB NOT NULL,
    channel_id              TEXT NOT NULL,
    parent_event_id         BLOB,
    parent_event_created_at TEXT,
    root_event_id           BLOB,
    root_event_created_at   TEXT,
    depth                   INTEGER NOT NULL DEFAULT 0,
    reply_count             INTEGER NOT NULL DEFAULT 0,
    descendant_count        INTEGER NOT NULL DEFAULT 0,
    last_reply_at           TEXT,
    broadcast               BOOLEAN NOT NULL DEFAULT FALSE,
    PRIMARY KEY (community_id, event_created_at, event_id),
    FOREIGN KEY (community_id, channel_id) REFERENCES channels (community_id, id)
);

CREATE INDEX idx_thread_metadata_parent ON thread_metadata (community_id, parent_event_id);
CREATE INDEX idx_thread_metadata_root ON thread_metadata (community_id, root_event_id);
CREATE INDEX idx_thread_metadata_channel_depth
    ON thread_metadata (community_id, channel_id, depth, event_created_at);
CREATE INDEX idx_thread_metadata_event_id ON thread_metadata (community_id, event_id);

-- ── Reactions ─────────────────────────────────────────────────────────────

CREATE TABLE reactions (
    community_id        TEXT NOT NULL REFERENCES communities(id),
    event_created_at    TEXT NOT NULL,
    event_id            BLOB NOT NULL,
    pubkey              BLOB NOT NULL,
    emoji               TEXT NOT NULL,
    created_at          TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    removed_at          TEXT,
    reaction_event_id   BLOB,
    PRIMARY KEY (community_id, event_created_at, event_id, pubkey, emoji)
);

CREATE INDEX idx_reactions_event ON reactions (community_id, event_id, event_created_at);
CREATE INDEX idx_reactions_pubkey ON reactions (community_id, pubkey);
CREATE UNIQUE INDEX idx_reactions_source_event ON reactions (community_id, reaction_event_id)
    WHERE reaction_event_id IS NOT NULL;

-- ── Pubkey allowlist ──────────────────────────────────────────────────────

CREATE TABLE pubkey_allowlist (
    community_id TEXT NOT NULL REFERENCES communities(id),
    pubkey      BLOB NOT NULL,
    added_by    BLOB,
    added_at    TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    note        TEXT,
    PRIMARY KEY (community_id, pubkey)
);

-- ── Relay members (NIP-43) ────────────────────────────────────────────────

CREATE TABLE relay_members (
    community_id TEXT NOT NULL REFERENCES communities(id),
    pubkey      TEXT NOT NULL,
    role        TEXT NOT NULL CHECK (role IN ('owner', 'admin', 'member')),
    added_by    TEXT,
    created_at  TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    updated_at  TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    PRIMARY KEY (community_id, pubkey)
);

CREATE INDEX idx_relay_members_role ON relay_members (community_id, role);

-- ── Join policy acceptances ───────────────────────────────────────────────

CREATE TABLE join_policy_acceptances (
    community_id TEXT NOT NULL,
    pubkey TEXT NOT NULL,
    policy_version TEXT NOT NULL CHECK (length(policy_version) = 64),
    accepted_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    PRIMARY KEY (community_id, pubkey, policy_version),
    FOREIGN KEY (community_id, pubkey)
        REFERENCES relay_members (community_id, pubkey) ON DELETE CASCADE
);

-- ── Relay invites (use-limited invite links) ──────────────────────────────

CREATE TABLE relay_invites (
    community_id TEXT        NOT NULL REFERENCES communities(id),
    id           TEXT        NOT NULL,
    token_hash   BLOB        NOT NULL CHECK (length(token_hash) = 32),
    role         TEXT        NOT NULL DEFAULT 'member' CHECK (role = 'member'),
    max_uses     INTEGER     CHECK (max_uses BETWEEN 1 AND 10000),
    use_count    INTEGER     NOT NULL DEFAULT 0 CHECK (use_count >= 0),
    expires_at   TEXT        NOT NULL,
    created_by   TEXT        NOT NULL,
    created_at   TEXT        NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    PRIMARY KEY (community_id, id),
    UNIQUE (community_id, token_hash),
    CHECK (max_uses IS NULL OR use_count <= max_uses)
);

CREATE INDEX relay_invites_expires_at_idx ON relay_invites (expires_at);

-- ── Archived identities (NIP-IA) ──────────────────────────────────────────

CREATE TABLE archived_identities (
    community_id      TEXT NOT NULL REFERENCES communities(id),
    pubkey            TEXT NOT NULL,
    consent_path      TEXT NOT NULL CHECK (consent_path IN ('self', 'owner', 'admin')),
    actor             TEXT NOT NULL,
    reason            TEXT,
    replaced_by       TEXT,
    request_event_id  TEXT NOT NULL,
    archived_at       TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    PRIMARY KEY (community_id, pubkey)
);

-- ── Audit log ─────────────────────────────────────────────────────────────
-- Per-community hash chain: uniqueness (community_id, seq) and
-- (community_id, hash). One chain per tenant.

CREATE TABLE audit_log (
    community_id    TEXT NOT NULL REFERENCES communities(id),
    seq             INTEGER NOT NULL,
    hash            BLOB NOT NULL,
    prev_hash       BLOB,
    action          TEXT NOT NULL,
    actor_pubkey    BLOB,
    object_id       TEXT,
    detail          TEXT,
    created_at      TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    PRIMARY KEY (community_id, seq)
);

CREATE UNIQUE INDEX idx_audit_log_hash ON audit_log (community_id, hash);

-- ── NIP-56 reports (kind:1984 ingest) ─────────────────────────────────────

CREATE TABLE moderation_reports (
    community_id        TEXT NOT NULL REFERENCES communities(id),
    id                  TEXT NOT NULL,
    report_event_id     BLOB NOT NULL CHECK (length(report_event_id) = 32),
    reporter_pubkey     BLOB NOT NULL CHECK (length(reporter_pubkey) = 32),
    target_kind         TEXT NOT NULL CHECK (target_kind IN ('event', 'pubkey', 'blob')),
    target_event_id     BLOB CHECK (target_event_id IS NULL OR length(target_event_id) = 32),
    target_pubkey       BLOB CHECK (target_pubkey IS NULL OR length(target_pubkey) = 32),
    target_blob_sha256  BLOB CHECK (target_blob_sha256 IS NULL OR length(target_blob_sha256) = 32),
    channel_id          TEXT,
    report_type         TEXT NOT NULL,
    note                TEXT,
    status              TEXT NOT NULL DEFAULT 'open'
                        CHECK (status IN ('open', 'resolved', 'dismissed', 'escalated')),
    resolved_by         BLOB,
    resolved_at         TEXT,
    action_id           TEXT,
    created_at          TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    PRIMARY KEY (community_id, id),
    CHECK (
        (target_kind = 'event'  AND target_event_id IS NOT NULL AND target_pubkey IS NULL     AND target_blob_sha256 IS NULL) OR
        (target_kind = 'pubkey' AND target_event_id IS NULL     AND target_pubkey IS NOT NULL AND target_blob_sha256 IS NULL) OR
        (target_kind = 'blob'   AND target_event_id IS NULL     AND target_pubkey IS NULL     AND target_blob_sha256 IS NOT NULL)
    ),
    FOREIGN KEY (community_id, channel_id) REFERENCES channels (community_id, id),
    FOREIGN KEY (community_id, action_id) REFERENCES moderation_actions (community_id, id)
);

CREATE INDEX idx_moderation_reports_status
    ON moderation_reports (community_id, status, created_at DESC);
CREATE INDEX idx_moderation_reports_target_event
    ON moderation_reports (community_id, target_event_id)
    WHERE target_event_id IS NOT NULL;
CREATE INDEX idx_moderation_reports_target_pubkey
    ON moderation_reports (community_id, target_pubkey)
    WHERE target_pubkey IS NOT NULL;
CREATE UNIQUE INDEX idx_moderation_reports_event
    ON moderation_reports (community_id, report_event_id);

-- ── Bans + timeouts (one restriction row per member) ──────────────────────

CREATE TABLE community_bans (
    community_id    TEXT NOT NULL REFERENCES communities(id),
    pubkey          BLOB NOT NULL CHECK (length(pubkey) = 32),
    banned          BOOLEAN NOT NULL DEFAULT FALSE,
    ban_expires_at  TEXT,
    ban_reason      TEXT,
    muted_until     TEXT,
    mute_reason     TEXT,
    actor_pubkey    BLOB NOT NULL CHECK (length(actor_pubkey) = 32),
    created_at      TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    updated_at      TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    PRIMARY KEY (community_id, pubkey)
);

-- ── Moderation audit ──────────────────────────────────────────────────────

CREATE TABLE moderation_actions (
    community_id    TEXT NOT NULL REFERENCES communities(id),
    id              TEXT NOT NULL,
    actor_pubkey    BLOB NOT NULL CHECK (length(actor_pubkey) = 32),
    action          TEXT NOT NULL CHECK (action IN (
                        'delete_message', 'kick', 'ban', 'unban',
                        'timeout', 'untimeout', 'dismiss_report', 'escalate',
                        'resolve:delete', 'resolve:kick', 'resolve:ban',
                        'resolve:timeout')),
    target_pubkey   BLOB CHECK (target_pubkey IS NULL OR length(target_pubkey) = 32),
    target_event_id BLOB CHECK (target_event_id IS NULL OR length(target_event_id) = 32),
    channel_id      TEXT,
    reason_code     TEXT,
    public_reason   TEXT,
    private_reason  TEXT,
    matched_principal TEXT CHECK (matched_principal IS NULL OR matched_principal IN ('self', 'owner')),
    created_at      TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    PRIMARY KEY (community_id, id),
    FOREIGN KEY (community_id, channel_id) REFERENCES channels (community_id, id)
);

CREATE INDEX idx_moderation_actions_created
    ON moderation_actions (community_id, created_at DESC);
CREATE INDEX idx_moderation_actions_target_pubkey
    ON moderation_actions (community_id, target_pubkey)
    WHERE target_pubkey IS NOT NULL;

-- ── Lint allowlist registry ───────────────────────────────────────────────
-- Explicit registry of tables deliberately operator-global (NOT
-- tenant-scoped). Kept as a DB table (not a hard-coded list in the linter)
-- so the registry stays next to the schema it governs.

CREATE TABLE _operator_global_tables (
    table_name  TEXT PRIMARY KEY,
    reason      TEXT NOT NULL
);

INSERT INTO _operator_global_tables (table_name, reason) VALUES
    ('communities',           'the tenant registry itself; id IS the community key'),
    ('rate_limit_violations', 'deployment abuse/health; never tenant-observable; community_id is an attribution label only'),
    ('_operator_global_tables', 'the registry table itself');

-- ── Push leases + durable wake outbox (NIP-PL) ────────────────────────────
-- Every key is led by community_id. No advisory lock needed: SQLite
-- serializes writers for a single-instance deployment.

CREATE TABLE push_leases (
    community_id TEXT NOT NULL REFERENCES communities(id),
    author BLOB NOT NULL CHECK (length(author) = 32),
    installation_id TEXT NOT NULL CHECK (length(installation_id) BETWEEN 1 AND 64),
    source_event_id BLOB NOT NULL CHECK (length(source_event_id) = 32),
    source_created_at INTEGER NOT NULL,
    generation INTEGER NOT NULL CHECK (generation > 0),
    active BOOLEAN NOT NULL,
    endpoint_enabled BOOLEAN NOT NULL DEFAULT TRUE,
    app_profile TEXT,
    endpoint_hash BLOB CHECK (endpoint_hash IS NULL OR length(endpoint_hash) = 32),
    endpoint_grant TEXT,
    max_class TEXT CHECK (max_class IS NULL OR max_class IN ('silent','default','time_sensitive','urgent')),
    subscriptions TEXT,
    expires_at INTEGER NOT NULL,
    updated_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    PRIMARY KEY (community_id, author, installation_id),
    UNIQUE (community_id, source_event_id),
    CHECK ((active AND app_profile IS NOT NULL AND endpoint_hash IS NOT NULL AND endpoint_grant IS NOT NULL AND max_class IS NOT NULL AND subscriptions IS NOT NULL)
        OR (NOT active AND app_profile IS NULL AND endpoint_hash IS NULL AND endpoint_grant IS NULL AND max_class IS NULL AND subscriptions IS NULL))
);
CREATE UNIQUE INDEX push_leases_endpoint_unique
    ON push_leases (community_id, author, app_profile, endpoint_hash)
    WHERE active;
CREATE INDEX push_leases_expiry ON push_leases (community_id, expires_at) WHERE active;

CREATE TABLE push_wake_outbox (
    community_id TEXT NOT NULL REFERENCES communities(id),
    id TEXT NOT NULL,
    author BLOB NOT NULL CHECK (length(author) = 32),
    installation_id TEXT NOT NULL,
    lease_generation INTEGER NOT NULL CHECK (lease_generation > 0),
    endpoint_hash BLOB NOT NULL CHECK (length(endpoint_hash) = 32),
    event_id BLOB NOT NULL CHECK (length(event_id) = 32),
    class TEXT NOT NULL CHECK (class IN ('silent','default','time_sensitive','urgent')),
    expires_at INTEGER NOT NULL,
    state TEXT NOT NULL DEFAULT 'pending' CHECK (state IN ('pending','sending','delivered','failed')),
    attempts INTEGER NOT NULL DEFAULT 0 CHECK (attempts >= 0),
    next_attempt_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    lease_until TEXT,
    claim_id TEXT,
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    PRIMARY KEY (community_id, id),
    FOREIGN KEY (community_id, author, installation_id)
        REFERENCES push_leases (community_id, author, installation_id),
    UNIQUE (community_id, endpoint_hash, event_id)
);
CREATE INDEX push_wake_outbox_due
    ON push_wake_outbox (community_id, next_attempt_at) WHERE state = 'pending';
CREATE INDEX push_wake_outbox_recovery
    ON push_wake_outbox (community_id, lease_until) WHERE state = 'sending';

-- Durable event-to-push matching follower. The trigger runs in the event
-- insert transaction, so every accepted persistent event has a crash-safe
-- match job and rejected/rolled-back events never do.
CREATE TABLE push_match_queue (
    community_id TEXT NOT NULL REFERENCES communities(id),
    event_id BLOB NOT NULL CHECK (length(event_id) = 32),
    state TEXT NOT NULL DEFAULT 'pending' CHECK (state IN ('pending','matching')),
    attempts INTEGER NOT NULL DEFAULT 0 CHECK (attempts >= 0),
    next_attempt_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    lease_until TEXT,
    claim_id TEXT,
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    PRIMARY KEY (community_id, event_id)
);
CREATE INDEX push_match_queue_due
    ON push_match_queue (next_attempt_at, created_at) WHERE state = 'pending';
CREATE INDEX push_match_queue_recovery
    ON push_match_queue (lease_until) WHERE state = 'matching';

-- T1b push gate: enqueue only when the community has an active,
-- endpoint-enabled, unexpired lease. No advisory lock: SQLite serializes
-- writers for a single-instance deployment, so the plain EXISTS check
-- inside the same insert transaction is race-free.
CREATE TRIGGER events_enqueue_push_match
AFTER INSERT ON events
WHEN NEW.kind IN (7, 9, 1059, 40007, 46010)
BEGIN
    INSERT INTO push_match_queue (community_id, event_id)
    SELECT NEW.community_id, NEW.id
    WHERE EXISTS (
        SELECT 1 FROM push_leases
        WHERE community_id = NEW.community_id
          AND active
          AND endpoint_enabled
          AND expires_at > CAST(strftime('%s', 'now') AS INTEGER)
    )
    ON CONFLICT DO NOTHING;
END;

-- Channel TTL refresh: runs in the same transaction that makes a
-- channel-scoped event durable. Kind 9007 creates the channel and
-- initializes its own deadline, so it is excluded here.
CREATE TRIGGER events_refresh_channel_ttl
AFTER INSERT ON events
WHEN NEW.channel_id IS NOT NULL AND NEW.kind <> 9007
BEGIN
    UPDATE channels
    SET ttl_deadline = strftime('%Y-%m-%dT%H:%M:%fZ', 'now', '+' || ttl_seconds || ' seconds')
    WHERE community_id = NEW.community_id
      AND id = NEW.channel_id
      AND ttl_seconds IS NOT NULL
      AND archived_at IS NULL
      AND deleted_at IS NULL;
END;

-- NIP-RS payloads have no historical product value. Enforce physical removal
-- whenever a kind:30078 read-state row transitions to soft-deleted — whether
-- through `replace_parameterized_event`'s own hard-delete path (already a
-- physical DELETE, so this trigger simply never fires for it) or any other
-- code path that only knows the generic NIP-09 soft-delete convention (e.g. a
-- future generic deletion handler). This is the SQLite equivalent of the
-- Postgres `purge_soft_deleted_nip_rs` trigger, minus the mixed-relay-version
-- GUC opt-in gate: single-instance SQLite has exactly one writer binary, so
-- there is no legacy-writer case to fence separately from the corrected one.
--
-- The tag-shape checks below (exactly one `d` tag, exactly one bare
-- `["t","read-state"]` tag) intentionally mirror `buzz-db`'s Rust
-- `is_nip_rs` classification used by `replace_parameterized_event` — unlike
-- the looser `events_reject_live_nip_rs_hard_delete` guard below (which
-- fails closed on *any* row shaped like a read-state coordinate,
-- regardless of tag-count conformance), a nonconforming event with
-- duplicate discriminator tags is not a real NIP-RS coordinate and must
-- keep its full replacement history instead of being auto-purged.
CREATE TRIGGER events_purge_soft_deleted_nip_rs
AFTER UPDATE OF deleted_at ON events
WHEN OLD.deleted_at IS NULL
 AND NEW.deleted_at IS NOT NULL
 AND NEW.kind = 30078
 AND NEW.d_tag GLOB 'read-state:[0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f]'
 AND (SELECT count(*) FROM json_each(NEW.tags) t WHERE json_extract(t.value, '$[0]') = 'd') = 1
 AND (
     SELECT count(*) FROM json_each(NEW.tags) t
     WHERE json_extract(t.value, '$[0]') = 't' AND json_extract(t.value, '$[1]') = 'read-state'
       AND json_array_length(t.value) = 2
 ) = 1
BEGIN
    DELETE FROM events
    WHERE community_id = NEW.community_id AND created_at = NEW.created_at AND id = NEW.id;

    DELETE FROM event_mentions
    WHERE community_id = NEW.community_id AND event_id = NEW.id;
END;

-- Defends the NIP-RS ordering watermark against a legacy/buggy writer that
-- inserts directly into `events` instead of going through
-- `replace_parameterized_event` (which checks `parameterized_event_watermarks`
-- itself before ever issuing SQL, so a well-behaved caller never reaches
-- these triggers). Uses the same loose kind/d_tag/tag-shape match as
-- `events_reject_live_nip_rs_hard_delete` below rather than the stricter
-- `is_nip_rs`-equivalent shape check on the purge trigger above: this is a
-- defensive backstop, not a purge-eligibility test, so it should fail
-- closed for anything shaped like a read-state coordinate.
--
-- An exact replay of the current watermark (equal `created_at`/`id`) is a
-- routine idempotent redelivery and is silently absorbed as a no-op
-- (`RAISE(IGNORE)`); a replay strictly older than the watermark is a real
-- ordering violation and is rejected loudly (`RAISE(ABORT)`) so the legacy
-- caller can observe and log it.
CREATE TRIGGER events_ignore_nip_rs_exact_watermark_replay
BEFORE INSERT ON events
WHEN NEW.kind = 30078
 AND NEW.d_tag GLOB 'read-state:[0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f]'
 AND EXISTS (
     SELECT 1 FROM json_each(NEW.tags) t
     WHERE json_extract(t.value, '$[0]') = 't' AND json_extract(t.value, '$[1]') = 'read-state'
 )
 AND EXISTS (
     SELECT 1 FROM parameterized_event_watermarks w
     WHERE w.community_id = NEW.community_id AND w.kind = NEW.kind
       AND w.pubkey = NEW.pubkey AND w.d_tag = NEW.d_tag
       AND w.created_at = NEW.created_at AND w.event_id = NEW.id
 )
BEGIN
    SELECT RAISE(IGNORE);
END;

CREATE TRIGGER events_reject_nip_rs_stale_watermark_replay
BEFORE INSERT ON events
WHEN NEW.kind = 30078
 AND NEW.d_tag GLOB 'read-state:[0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f]'
 AND EXISTS (
     SELECT 1 FROM json_each(NEW.tags) t
     WHERE json_extract(t.value, '$[0]') = 't' AND json_extract(t.value, '$[1]') = 'read-state'
 )
 AND EXISTS (
     SELECT 1 FROM parameterized_event_watermarks w
     WHERE w.community_id = NEW.community_id AND w.kind = NEW.kind
       AND w.pubkey = NEW.pubkey AND w.d_tag = NEW.d_tag
       AND (w.created_at > NEW.created_at
            OR (w.created_at = NEW.created_at AND w.event_id > NEW.id))
 )
BEGIN
    SELECT RAISE(ABORT, 'stale NIP-RS replay dominated by ordering watermark');
END;

-- Buzz-mesh member-status (kind:30003, d_tag `buzz-mesh-member-status:*`)
-- follows the same "no historical value, hard-delete on supersede" rule as
-- NIP-RS above.
CREATE TRIGGER events_purge_soft_deleted_mesh_status
AFTER UPDATE OF deleted_at ON events
WHEN OLD.deleted_at IS NULL
 AND NEW.deleted_at IS NOT NULL
 AND NEW.kind = 30003
 AND NEW.d_tag LIKE 'buzz-mesh-member-status:%'
 AND EXISTS (
     SELECT 1 FROM json_each(NEW.tags) t
     WHERE json_extract(t.value, '$[0]') = 'k' AND json_extract(t.value, '$[1]') = 'buzz-mesh-status'
 )
BEGIN
    DELETE FROM events
    WHERE community_id = NEW.community_id AND created_at = NEW.created_at AND id = NEW.id;

    DELETE FROM event_mentions
    WHERE community_id = NEW.community_id AND event_id = NEW.id;
END;

-- Fail closed against a direct hard `DELETE` of a still-live row at either
-- purge-eligible coordinate above. The only sanctioned way to remove such a
-- row is the NIP-09 soft-delete convention (`UPDATE ... SET deleted_at =
-- ...`), which the two triggers above then convert into the same physical
-- purge. By the time that conversion's own internal `DELETE` runs, the row's
-- `deleted_at` is already non-NULL, so `OLD.deleted_at IS NULL` below no
-- longer holds and these guards do not block it — only a delete that skips
-- the soft-delete step entirely is rejected.
CREATE TRIGGER events_reject_live_nip_rs_hard_delete
BEFORE DELETE ON events
WHEN OLD.deleted_at IS NULL
 AND OLD.kind = 30078
 AND OLD.d_tag GLOB 'read-state:[0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f]'
 AND EXISTS (
     SELECT 1 FROM json_each(OLD.tags) t
     WHERE json_extract(t.value, '$[0]') = 't' AND json_extract(t.value, '$[1]') = 'read-state'
 )
BEGIN
    SELECT RAISE(ABORT, 'live NIP-RS event must be soft-deleted, not hard-deleted directly');
END;

CREATE TRIGGER events_reject_live_mesh_status_hard_delete
BEFORE DELETE ON events
WHEN OLD.deleted_at IS NULL
 AND OLD.kind = 30003
 AND OLD.d_tag LIKE 'buzz-mesh-member-status:%'
 AND EXISTS (
     SELECT 1 FROM json_each(OLD.tags) t
     WHERE json_extract(t.value, '$[0]') = 'k' AND json_extract(t.value, '$[1]') = 'buzz-mesh-status'
 )
BEGIN
    SELECT RAISE(ABORT, 'live buzz-mesh-status event must be soft-deleted, not hard-deleted directly');
END;

-- ── Full-text search (SQLite FTS5, replaces Postgres tsvector/GIN) ────────
-- `events` has a composite PRIMARY KEY, not a bare INTEGER one, so it cannot
-- be safely wired as an FTS5 "external content" table (a `VACUUM` may
-- renumber the rowids of tables without an INTEGER PRIMARY KEY alias, which
-- would desync an external-content index). This is instead a standalone
-- FTS5 table that duplicates the small set of columns `buzz-search` needs,
-- kept in sync by the triggers below. `event_id`/`pubkey` are stored as
-- lowercase hex (FTS5 columns are dynamically typed, but hex text keeps
-- comparisons simple and avoids relying on BLOB storage through the FTS
-- shadow tables). All non-`content` columns are UNINDEXED: they exist only
-- to let the relay filter/scope hits, never to be matched against search
-- text — `community_id` scoping in particular must never be satisfiable by
-- crafted search input (conformance row 50).
CREATE VIRTUAL TABLE events_fts USING fts5(
    content,
    community_id UNINDEXED,
    event_id UNINDEXED,
    kind UNINDEXED,
    pubkey UNINDEXED,
    channel_id UNINDEXED,
    created_at UNINDEXED,
    tokenize = 'unicode61 remove_diacritics 2'
);

-- Mirrors `events.rowid` onto `events_fts.rowid` so UPDATE/DELETE below can
-- address the matching FTS row directly instead of scanning the shadow
-- table. Every write to `events` re-derives the FTS row from scratch
-- (delete-then-maybe-reinsert) so INSERT, content edits (moderation
-- redaction), soft-delete (NIP-09), and hard-delete (purge triggers above)
-- all stay correct through one pair of triggers instead of several
-- conditionally-overlapping ones.
--
-- Privacy skip-set (kept byte-for-byte in sync with the old Postgres
-- `search_tsv GENERATED ALWAYS AS (CASE WHEN kind IN (...) THEN NULL ...)`
-- column, and with the Rust-side `AUTHOR_ONLY_KINDS`/`P_GATED_KINDS`
-- constants in `buzz-core::kind`): 1059 = gift wrap (NIP-17 ciphertext),
-- 30300/30350 = author-only (event reminder / push lease), 30622 = DM
-- visibility snapshot, 44100/44101 = p-gated membership notices, 44200 =
-- p-gated agent turn metric. These kinds must never be discoverable through
-- NIP-50 full-text search — excluding them at the storage layer (never
-- inserted into `events_fts`) is a defense-in-depth backstop below the
-- filter-level `#p`/author gate, drift-checked by
-- `crates/buzz-search/tests/fts_integration.rs`.
CREATE TRIGGER events_fts_insert AFTER INSERT ON events
WHEN NEW.deleted_at IS NULL
    AND NEW.kind NOT IN (1059, 30300, 30350, 30622, 44100, 44101, 44200)
BEGIN
    INSERT INTO events_fts (rowid, content, community_id, event_id, kind, pubkey, channel_id, created_at)
    VALUES (NEW.rowid, NEW.content, NEW.community_id, lower(hex(NEW.id)), NEW.kind, lower(hex(NEW.pubkey)), NEW.channel_id, NEW.created_at);
END;

CREATE TRIGGER events_fts_update AFTER UPDATE ON events
BEGIN
    DELETE FROM events_fts WHERE rowid = OLD.rowid;
    INSERT INTO events_fts (rowid, content, community_id, event_id, kind, pubkey, channel_id, created_at)
    SELECT NEW.rowid, NEW.content, NEW.community_id, lower(hex(NEW.id)), NEW.kind, lower(hex(NEW.pubkey)), NEW.channel_id, NEW.created_at
    WHERE NEW.deleted_at IS NULL
      AND NEW.kind NOT IN (1059, 30300, 30350, 30622, 44100, 44101, 44200);
END;

CREATE TRIGGER events_fts_delete AFTER DELETE ON events
BEGIN
    DELETE FROM events_fts WHERE rowid = OLD.rowid;
END;

-- Durable, deployment-global authority for the public NIP-PL push gateway.
-- Intentionally outside relay community tenancy: installations delegate to
-- relay signing keys and may authorize multiple relay deployments.
CREATE TABLE push_gateway_challenges (
    id TEXT PRIMARY KEY,
    challenge_hash BLOB NOT NULL CHECK (length(challenge_hash) = 32),
    expires_at TEXT NOT NULL,
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
);
CREATE INDEX push_gateway_challenges_expiry ON push_gateway_challenges (expires_at);

CREATE TABLE push_gateway_installations (
    id TEXT PRIMARY KEY,
    app_attest_key_id BLOB NOT NULL UNIQUE CHECK (length(app_attest_key_id) BETWEEN 1 AND 128),
    app_attest_public_key BLOB NOT NULL CHECK (length(app_attest_public_key) BETWEEN 33 AND 256),
    assertion_counter INTEGER NOT NULL CHECK (assertion_counter BETWEEN 0 AND 4294967295),
    app_profile TEXT NOT NULL CHECK (app_profile IN ('buzz-ios-production','buzz-ios-sandbox')),
    token_ciphertext BLOB NOT NULL CHECK (length(token_ciphertext) BETWEEN 1 AND 2048),
    token_fingerprint BLOB NOT NULL CHECK (length(token_fingerprint) = 32),
    endpoint_epoch INTEGER NOT NULL CHECK (endpoint_epoch > 0),
    expires_at TEXT NOT NULL,
    revoked_at TEXT,
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    updated_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    UNIQUE (app_profile, token_fingerprint)
);
CREATE INDEX push_gateway_installations_expiry ON push_gateway_installations (expires_at) WHERE revoked_at IS NULL;

CREATE TABLE push_gateway_delegations (
    id TEXT PRIMARY KEY,
    installation_id TEXT NOT NULL REFERENCES push_gateway_installations(id),
    relay_pubkey BLOB NOT NULL CHECK (length(relay_pubkey) = 32),
    endpoint_epoch INTEGER NOT NULL CHECK (endpoint_epoch > 0),
    generation INTEGER NOT NULL CHECK (generation > 0),
    not_before TEXT NOT NULL,
    expires_at TEXT NOT NULL,
    revoked_at TEXT,
    updated_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    UNIQUE (installation_id, relay_pubkey),
    CHECK (not_before < expires_at)
);
CREATE INDEX push_gateway_delegations_expiry ON push_gateway_delegations (expires_at) WHERE revoked_at IS NULL;

CREATE TABLE push_gateway_endpoint_quotas (
    token_fingerprint BLOB PRIMARY KEY CHECK (length(token_fingerprint) = 32),
    window_started_at TEXT NOT NULL,
    admitted INTEGER NOT NULL CHECK (admitted >= 0),
    updated_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
);
CREATE INDEX push_gateway_endpoint_quotas_updated ON push_gateway_endpoint_quotas (updated_at);

CREATE TABLE push_gateway_delivery_auth_replays (
    relay_pubkey BLOB NOT NULL CHECK (length(relay_pubkey) = 32),
    auth_event_id BLOB NOT NULL CHECK (length(auth_event_id) = 32),
    expires_at TEXT NOT NULL,
    PRIMARY KEY (relay_pubkey, auth_event_id)
);
CREATE INDEX push_gateway_delivery_auth_replays_expiry ON push_gateway_delivery_auth_replays (expires_at);

CREATE TABLE push_gateway_delivery_request_replays (
    relay_pubkey BLOB NOT NULL CHECK (length(relay_pubkey) = 32),
    request_id TEXT NOT NULL,
    expires_at TEXT NOT NULL,
    PRIMARY KEY (relay_pubkey, request_id)
);
CREATE INDEX push_gateway_delivery_request_replays_expiry ON push_gateway_delivery_request_replays (expires_at);

INSERT INTO _operator_global_tables (table_name, reason) VALUES
    ('push_gateway_challenges', 'public gateway one-time challenges span relay communities'),
    ('push_gateway_installations', 'public gateway installation authority spans relay communities'),
    ('push_gateway_delegations', 'public gateway relay delegations span relay communities'),
    ('push_gateway_endpoint_quotas', 'public gateway endpoint abuse ceilings span relay communities'),
    ('push_gateway_delivery_auth_replays', 'public gateway signed-event replay admission spans relay communities'),
    ('push_gateway_delivery_request_replays', 'public gateway stable request-id admission spans relay communities');

-- ── Community archival ────────────────────────────────────────────────────
-- Folded into `communities.archived_at` above (was migration 0016).

-- ── Product feedback (deployment-private sidecar) ────────────────────────
-- Retains community_id as provenance only — idempotent deployment-wide by
-- signed event_id, never a community moderation concern.

CREATE TABLE product_feedback (
    id                  TEXT PRIMARY KEY,
    community_id        TEXT NOT NULL,
    event_id            BLOB NOT NULL CHECK (length(event_id) = 32),
    submitter_pubkey    BLOB NOT NULL CHECK (length(submitter_pubkey) = 32),
    category            TEXT,
    body                TEXT NOT NULL,
    tags                TEXT NOT NULL,
    event_created_at    TEXT NOT NULL,
    received_at         TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
);
CREATE UNIQUE INDEX idx_product_feedback_event_id ON product_feedback (event_id);
CREATE INDEX idx_product_feedback_received_at ON product_feedback (received_at DESC);

INSERT INTO _operator_global_tables (table_name, reason) VALUES
    ('product_feedback', 'deployment product inbox; a deployment-private sidecar, not tenant-observable');

-- ── Git repo name registry (NIP-34 kind:30617) ────────────────────────────
-- The relay holds no persistent per-repo filesystem state (see
-- `git_repo.rs`); repo-name uniqueness within a community is the one shared
-- state need, enforced atomically by the primary key.
CREATE TABLE git_repo_names (
    community_id  TEXT NOT NULL REFERENCES communities(id),
    repo_id       TEXT NOT NULL,
    owner_pubkey  TEXT NOT NULL,
    created_at    TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    PRIMARY KEY (community_id, repo_id)
);
CREATE INDEX idx_git_repo_names_owner ON git_repo_names (community_id, owner_pubkey);
