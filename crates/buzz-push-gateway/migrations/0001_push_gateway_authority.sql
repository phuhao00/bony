-- Durable, deployment-global authority for the public NIP-PL push gateway.
-- This state is intentionally outside relay community tenancy: installations
-- delegate to relay signing keys and may authorize multiple relay deployments.
--
-- SQLite translation (single-instance/desktop deployment; see
-- `migrations/0001_initial_schema.sql` at the repo root for the full set of
-- cross-engine mapping decisions this follows): uuid -> TEXT, bytea -> BLOB,
-- timestamptz -> TEXT (RFC3339 UTC), octet_length() -> length(),
-- now() -> strftime('%Y-%m-%dT%H:%M:%fZ', 'now').
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
