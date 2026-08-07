//! SQLite authority store (single-instance/desktop deployment).
//!
//! Every mutating method that reads state and then conditionally writes it
//! (the compare-and-swap methods: `upsert_delegation`, `authorize_delivery`)
//! opens its transaction with `BEGIN IMMEDIATE` rather than a plain `BEGIN`.
//! SQLite's default deferred `BEGIN` only takes a lock on the first
//! statement, so two concurrent read-then-write transactions can both
//! acquire a SHARED (read) lock and then race for the RESERVED (write) lock
//! — the loser blocks, but by the time it wakes it has already made its
//! CAS decision against stale data. `BEGIN IMMEDIATE` acquires RESERVED
//! up front, so a second transaction of this shape cannot even start until
//! the first fully commits or rolls back, giving the same "lock, read,
//! compare, write" serialization the historical PostgreSQL implementation
//! got from `SELECT ... FOR UPDATE`. Single-statement CAS updates (a bare
//! `UPDATE ... WHERE <guard>`) don't need this — a single statement is
//! already atomic.
use crate::authority::*;
use crate::model::AppProfile;
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use sqlx::{Row, SqlitePool};
use uuid::Uuid;

#[derive(Clone)]
pub struct SqliteAuthorityStore {
    pool: SqlitePool,
}

impl SqliteAuthorityStore {
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }

    /// Applies the authority schema migration.
    ///
    /// SQLite has no role/privilege-separation concept, so unlike the
    /// historical PostgreSQL implementation (`apply_migrations_and_grants`,
    /// which also carved out a least-privilege runtime role via
    /// GRANT/REVOKE) this only runs the schema migration — a single-instance
    /// desktop deployment has exactly one process holding the database file,
    /// not a fleet of runtime pods that must be denied DDL.
    pub async fn apply_migrations(pool: &SqlitePool) -> Result<(), Box<dyn std::error::Error>> {
        sqlx::migrate!("./migrations").run(pool).await?;
        Ok(())
    }
}
fn at(ts: i64) -> Result<DateTime<Utc>, AuthorityError> {
    DateTime::from_timestamp(ts, 0).ok_or(AuthorityError::Rejected)
}
fn ts(v: DateTime<Utc>) -> i64 {
    v.timestamp()
}
fn profile(v: &str) -> Result<AppProfile, AuthorityError> {
    match v {
        "buzz-ios-production" => Ok(AppProfile::BuzzIosProduction),
        "buzz-ios-sandbox" => Ok(AppProfile::BuzzIosSandbox),
        _ => Err(AuthorityError::Unavailable),
    }
}
fn db(_: sqlx::Error) -> AuthorityError {
    AuthorityError::Unavailable
}
fn bytes32(v: Vec<u8>) -> Result<[u8; 32], AuthorityError> {
    v.try_into().map_err(|_| AuthorityError::Unavailable)
}
fn uuid_col(id: String) -> Result<Uuid, AuthorityError> {
    Uuid::parse_str(&id).map_err(|_| AuthorityError::Unavailable)
}

#[async_trait]
impl AuthorityStore for SqliteAuthorityStore {
    async fn ready(&self) -> Result<(), AuthorityError> {
        const TABLES: [&str; 6] = [
            "push_gateway_challenges",
            "push_gateway_installations",
            "push_gateway_delegations",
            "push_gateway_endpoint_quotas",
            "push_gateway_delivery_auth_replays",
            "push_gateway_delivery_request_replays",
        ];
        let present: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM sqlite_master WHERE type = 'table' AND name IN (?, ?, ?, ?, ?, ?)",
        )
        .bind(TABLES[0])
        .bind(TABLES[1])
        .bind(TABLES[2])
        .bind(TABLES[3])
        .bind(TABLES[4])
        .bind(TABLES[5])
        .fetch_one(&self.pool)
        .await
        .map_err(db)?;
        if present != TABLES.len() as i64 {
            return Err(AuthorityError::Unavailable);
        }
        Ok(())
    }

    async fn put_challenge(&self, c: Challenge) -> Result<(), AuthorityError> {
        use sha2::{Digest, Sha256};
        sqlx::query("INSERT INTO push_gateway_challenges(id,challenge_hash,expires_at) VALUES(?,?,?)")
            .bind(c.id.to_string())
            .bind(Sha256::digest(c.value).to_vec())
            .bind(at(c.expires_at)?)
            .execute(&self.pool)
            .await
            .map_err(db)?;
        Ok(())
    }
    async fn consume_challenge(
        &self,
        id: Uuid,
        value: [u8; 32],
        now: i64,
    ) -> Result<(), AuthorityError> {
        use sha2::{Digest, Sha256};
        let result = sqlx::query(
            "DELETE FROM push_gateway_challenges WHERE id=? AND challenge_hash=? AND expires_at >= ?",
        )
        .bind(id.to_string())
        .bind(Sha256::digest(value).to_vec())
        .bind(at(now)?)
        .execute(&self.pool)
        .await
        .map_err(db)?;
        if result.rows_affected() != 1 {
            return Err(AuthorityError::Rejected);
        }
        Ok(())
    }
    async fn create_installation(&self, n: NewInstallation) -> Result<(), AuthorityError> {
        let result = sqlx::query(
            "INSERT INTO push_gateway_installations(id,app_attest_key_id,app_attest_public_key,assertion_counter,app_profile,token_ciphertext,token_fingerprint,endpoint_epoch,expires_at) \
             VALUES(?,?,?,?,?,?,?,?,?) ON CONFLICT DO NOTHING",
        )
        .bind(n.id.to_string())
        .bind(n.app_attest_key_id)
        .bind(n.app_attest_public_key)
        .bind(i64::from(n.assertion_counter))
        .bind(n.profile.as_str())
        .bind(n.token_ciphertext)
        .bind(n.token_fingerprint.to_vec())
        .bind(n.endpoint_epoch)
        .bind(at(n.expires_at)?)
        .execute(&self.pool)
        .await
        .map_err(db)?;
        if result.rows_affected() != 1 {
            return Err(AuthorityError::Rejected);
        }
        Ok(())
    }
    async fn installation(&self, id: Uuid, now: i64) -> Result<Installation, AuthorityError> {
        let r = sqlx::query(
            "SELECT * FROM push_gateway_installations WHERE id=? AND revoked_at IS NULL AND expires_at >= ?",
        )
        .bind(id.to_string())
        .bind(at(now)?)
        .fetch_optional(&self.pool)
        .await
        .map_err(db)?
        .ok_or(AuthorityError::Rejected)?;
        Ok(Installation {
            id,
            app_attest_key_id: r.try_get("app_attest_key_id").map_err(db)?,
            app_attest_public_key: r.try_get("app_attest_public_key").map_err(db)?,
            assertion_counter: u32::try_from(r.try_get::<i64, _>("assertion_counter").map_err(db)?)
                .map_err(|_| AuthorityError::Unavailable)?,
            profile: profile(r.try_get("app_profile").map_err(db)?)?,
            token_ciphertext: r.try_get("token_ciphertext").map_err(db)?,
            token_fingerprint: bytes32(r.try_get("token_fingerprint").map_err(db)?)?,
            endpoint_epoch: r.try_get("endpoint_epoch").map_err(db)?,
            expires_at: ts(r.try_get("expires_at").map_err(db)?),
            revoked: false,
        })
    }
    async fn advance_assertion_counter(
        &self,
        id: Uuid,
        previous: u32,
        next: u32,
    ) -> Result<(), AuthorityError> {
        if next <= previous {
            return Err(AuthorityError::Rejected);
        }
        let result = sqlx::query(
            "UPDATE push_gateway_installations SET assertion_counter=?,updated_at=? WHERE id=? AND assertion_counter=? AND revoked_at IS NULL",
        )
        .bind(i64::from(next))
        .bind(Utc::now())
        .bind(id.to_string())
        .bind(i64::from(previous))
        .execute(&self.pool)
        .await
        .map_err(db)?;
        if result.rows_affected() != 1 {
            return Err(AuthorityError::Rejected);
        }
        Ok(())
    }
    async fn upsert_delegation(&self, d: Delegation) -> Result<(), AuthorityError> {
        let mut tx = self
            .pool
            .begin_with("BEGIN IMMEDIATE")
            .await
            .map_err(db)?;
        let i = sqlx::query(
            "SELECT endpoint_epoch,expires_at,revoked_at FROM push_gateway_installations WHERE id=?",
        )
        .bind(d.installation_id.to_string())
        .fetch_optional(&mut *tx)
        .await
        .map_err(db)?
        .ok_or(AuthorityError::Rejected)?;
        if i.try_get::<Option<DateTime<Utc>>, _>("revoked_at")
            .map_err(db)?
            .is_some()
            || i.try_get::<i64, _>("endpoint_epoch").map_err(db)? != d.endpoint_epoch
            || at(d.expires_at)? > i.try_get::<DateTime<Utc>, _>("expires_at").map_err(db)?
        {
            return Err(AuthorityError::Rejected);
        }
        let relay = hex::decode(&d.relay_pubkey).map_err(|_| AuthorityError::Rejected)?;
        let result = sqlx::query(
            "INSERT INTO push_gateway_delegations(id,installation_id,relay_pubkey,endpoint_epoch,generation,not_before,expires_at,revoked_at) VALUES(?,?,?,?,?,?,?,NULL) \
             ON CONFLICT(installation_id,relay_pubkey) DO UPDATE SET \
               id=excluded.id,endpoint_epoch=excluded.endpoint_epoch,generation=excluded.generation, \
               not_before=excluded.not_before,expires_at=excluded.expires_at,revoked_at=NULL,updated_at=? \
             WHERE excluded.generation > push_gateway_delegations.generation",
        )
        .bind(d.id.to_string())
        .bind(d.installation_id.to_string())
        .bind(relay)
        .bind(d.endpoint_epoch)
        .bind(d.generation)
        .bind(at(d.not_before)?)
        .bind(at(d.expires_at)?)
        .bind(Utc::now())
        .execute(&mut *tx)
        .await
        .map_err(db)?;
        if result.rows_affected() != 1 {
            return Err(AuthorityError::Rejected);
        }
        tx.commit().await.map_err(db)?;
        Ok(())
    }
    async fn rotate_endpoint(
        &self,
        id: Uuid,
        expected: i64,
        new: i64,
        ciphertext: Vec<u8>,
        fingerprint: [u8; 32],
    ) -> Result<(), AuthorityError> {
        if new != expected.checked_add(1).ok_or(AuthorityError::Rejected)? {
            return Err(AuthorityError::Rejected);
        }
        let result = sqlx::query(
            "UPDATE push_gateway_installations SET endpoint_epoch=?,token_ciphertext=?,token_fingerprint=?,updated_at=? WHERE id=? AND endpoint_epoch=? AND revoked_at IS NULL",
        )
        .bind(new)
        .bind(ciphertext)
        .bind(fingerprint.to_vec())
        .bind(Utc::now())
        .bind(id.to_string())
        .bind(expected)
        .execute(&self.pool)
        .await
        .map_err(db)?;
        if result.rows_affected() != 1 {
            return Err(AuthorityError::Rejected);
        }
        Ok(())
    }
    async fn revoke_delegation(
        &self,
        id: Uuid,
        relay: &str,
        generation: i64,
    ) -> Result<(), AuthorityError> {
        let relay = hex::decode(relay).map_err(|_| AuthorityError::Rejected)?;
        let result = sqlx::query(
            "UPDATE push_gateway_delegations SET generation=?,revoked_at=?,updated_at=? WHERE installation_id=? AND relay_pubkey=? AND generation<?",
        )
        .bind(generation)
        .bind(Utc::now())
        .bind(Utc::now())
        .bind(id.to_string())
        .bind(relay)
        .bind(generation)
        .execute(&self.pool)
        .await
        .map_err(db)?;
        if result.rows_affected() != 1 {
            return Err(AuthorityError::Rejected);
        }
        Ok(())
    }
    async fn revoke_installation(
        &self,
        id: Uuid,
        expected: i64,
        new: i64,
    ) -> Result<(), AuthorityError> {
        if new != expected.checked_add(1).ok_or(AuthorityError::Rejected)? {
            return Err(AuthorityError::Rejected);
        }
        let result = sqlx::query(
            "UPDATE push_gateway_installations SET endpoint_epoch=?,revoked_at=?,updated_at=? WHERE id=? AND endpoint_epoch=? AND revoked_at IS NULL",
        )
        .bind(new)
        .bind(Utc::now())
        .bind(Utc::now())
        .bind(id.to_string())
        .bind(expected)
        .execute(&self.pool)
        .await
        .map_err(db)?;
        if result.rows_affected() != 1 {
            return Err(AuthorityError::Rejected);
        }
        Ok(())
    }
    async fn authorize_delivery(
        &self,
        did: Uuid,
        relay: &str,
        epoch: i64,
        generation: i64,
        event_id: &str,
        request_id: Uuid,
        request_expires_at: i64,
        quota_window_seconds: i64,
        quota_max_deliveries: i64,
        now: i64,
    ) -> Result<DeliveryPermit, AuthorityError> {
        let relay_bytes = hex::decode(relay).map_err(|_| AuthorityError::Rejected)?;
        let event_bytes = hex::decode(event_id).map_err(|_| AuthorityError::Rejected)?;
        if event_bytes.len() != 32 {
            return Err(AuthorityError::Rejected);
        }
        let mut tx = self
            .pool
            .begin_with("BEGIN IMMEDIATE")
            .await
            .map_err(db)?;
        // Every authority mutation locks installation before delegation. Keep
        // this order here to avoid delivery-vs-refresh deadlocks.
        let i = sqlx::query(
            "SELECT app_profile,token_ciphertext,token_fingerprint,endpoint_epoch,expires_at,revoked_at
             FROM push_gateway_installations
             WHERE id=(SELECT installation_id FROM push_gateway_delegations WHERE id=?)",
        )
        .bind(did.to_string())
        .fetch_optional(&mut *tx)
        .await
        .map_err(db)?
        .ok_or(AuthorityError::Rejected)?;
        if i.try_get::<Option<DateTime<Utc>>, _>("revoked_at")
            .map_err(db)?
            .is_some()
            || i.try_get::<i64, _>("endpoint_epoch").map_err(db)? != epoch
            || i.try_get::<DateTime<Utc>, _>("expires_at").map_err(db)? < at(now)?
        {
            return Err(AuthorityError::Rejected);
        }
        let d = sqlx::query(
            "SELECT installation_id,expires_at FROM push_gateway_delegations
             WHERE id=? AND relay_pubkey=? AND endpoint_epoch=? AND generation=?
               AND revoked_at IS NULL AND not_before<=? AND expires_at>=?",
        )
        .bind(did.to_string())
        .bind(&relay_bytes)
        .bind(epoch)
        .bind(generation)
        .bind(at(now)?)
        .bind(at(now)?)
        .fetch_optional(&mut *tx)
        .await
        .map_err(db)?
        .ok_or(AuthorityError::Rejected)?;
        let installation_id: String = d.try_get("installation_id").map_err(db)?;
        let installation_id = uuid_col(installation_id)?;
        let authority = DeliveryAuthority {
            delegation_id: did,
            installation_id,
            relay_pubkey: relay.to_owned(),
            profile: profile(i.try_get("app_profile").map_err(db)?)?,
            token_ciphertext: i.try_get("token_ciphertext").map_err(db)?,
            endpoint_epoch: epoch,
            generation,
            expires_at: ts(d.try_get("expires_at").map_err(db)?),
        };
        if request_expires_at < now || request_expires_at > authority.expires_at {
            return Err(AuthorityError::Rejected);
        }
        if quota_window_seconds < 1 || quota_max_deliveries < 1 {
            return Err(AuthorityError::Unavailable);
        }
        let fingerprint: Vec<u8> = i.try_get("token_fingerprint").map_err(db)?;
        let now_ts = at(now)?;
        let window_boundary = now_ts - chrono::Duration::seconds(quota_window_seconds);
        let quota = sqlx::query(
            "INSERT INTO push_gateway_endpoint_quotas(token_fingerprint,window_started_at,admitted) VALUES(?,?,1) \
             ON CONFLICT(token_fingerprint) DO UPDATE SET \
               window_started_at=CASE WHEN push_gateway_endpoint_quotas.window_started_at <= ? THEN ? ELSE push_gateway_endpoint_quotas.window_started_at END, \
               admitted=CASE WHEN push_gateway_endpoint_quotas.window_started_at <= ? THEN 1 ELSE push_gateway_endpoint_quotas.admitted + 1 END, \
               updated_at=? \
             WHERE push_gateway_endpoint_quotas.window_started_at <= ? OR push_gateway_endpoint_quotas.admitted < ?",
        )
        .bind(&fingerprint)
        .bind(now_ts)
        .bind(window_boundary)
        .bind(now_ts)
        .bind(window_boundary)
        .bind(now_ts)
        .bind(window_boundary)
        .bind(quota_max_deliveries)
        .execute(&mut *tx)
        .await
        .map_err(db)?;
        if quota.rows_affected() != 1 {
            return Err(AuthorityError::Rejected);
        }
        let auth_inserted = sqlx::query(
            "INSERT INTO push_gateway_delivery_auth_replays(relay_pubkey,auth_event_id,expires_at) VALUES(?,?,?) ON CONFLICT DO NOTHING",
        )
        .bind(&relay_bytes)
        .bind(event_bytes)
        .bind(at(request_expires_at)?)
        .execute(&mut *tx)
        .await
        .map_err(db)?;
        let request_inserted = sqlx::query(
            "INSERT INTO push_gateway_delivery_request_replays(relay_pubkey,request_id,expires_at) VALUES(?,?,?) ON CONFLICT DO NOTHING",
        )
        .bind(&relay_bytes)
        .bind(request_id.to_string())
        .bind(at(request_expires_at)?)
        .execute(&mut *tx)
        .await
        .map_err(db)?;
        if auth_inserted.rows_affected() != 1 || request_inserted.rows_affected() != 1 {
            return Err(AuthorityError::Rejected);
        }
        tx.commit().await.map_err(db)?;
        Ok(DeliveryPermit::new(authority, relay.to_owned(), request_id))
    }

    async fn finish_delivery(
        &self,
        permit: DeliveryPermit,
        disposition: DeliveryDisposition,
    ) -> Result<(), AuthorityError> {
        if disposition == DeliveryDisposition::Retryable {
            sqlx::query(
                "DELETE FROM push_gateway_delivery_request_replays WHERE relay_pubkey=? AND request_id=?",
            )
            .bind(hex::decode(permit.relay_pubkey).map_err(|_| AuthorityError::Unavailable)?)
            .bind(permit.request_id.to_string())
            .execute(&self.pool)
            .await
            .map_err(db)?;
        }
        Ok(())
    }

    async fn reap_expired(&self, now: i64) -> Result<(), AuthorityError> {
        let mut tx = self
            .pool
            .begin_with("BEGIN IMMEDIATE")
            .await
            .map_err(db)?;
        let now_ts = at(now)?;
        let one_day_ago = now_ts - chrono::Duration::days(1);
        sqlx::query("DELETE FROM push_gateway_challenges WHERE expires_at < ?")
            .bind(now_ts)
            .execute(&mut *tx)
            .await
            .map_err(db)?;
        sqlx::query("DELETE FROM push_gateway_delivery_auth_replays WHERE expires_at < ?")
            .bind(now_ts)
            .execute(&mut *tx)
            .await
            .map_err(db)?;
        sqlx::query("DELETE FROM push_gateway_delivery_request_replays WHERE expires_at < ?")
            .bind(now_ts)
            .execute(&mut *tx)
            .await
            .map_err(db)?;
        sqlx::query("DELETE FROM push_gateway_endpoint_quotas WHERE updated_at < ?")
            .bind(one_day_ago)
            .execute(&mut *tx)
            .await
            .map_err(db)?;
        // A parent may become retention-eligible before an otherwise-active
        // child. Parent eligibility must therefore reap every child first;
        // otherwise the installation delete violates the delegation FK and
        // rolls back all cleanup in this transaction.
        sqlx::query(
            "DELETE FROM push_gateway_delegations
             WHERE expires_at < ?
                OR revoked_at < ?
                OR EXISTS (
                    SELECT 1 FROM push_gateway_installations i
                    WHERE i.id = push_gateway_delegations.installation_id
                      AND (i.expires_at < ? OR i.revoked_at < ?)
                )",
        )
        .bind(now_ts)
        .bind(one_day_ago)
        .bind(now_ts)
        .bind(one_day_ago)
        .execute(&mut *tx)
        .await
        .map_err(db)?;
        sqlx::query(
            "DELETE FROM push_gateway_installations WHERE expires_at < ? OR revoked_at < ?",
        )
        .bind(now_ts)
        .bind(one_day_ago)
        .execute(&mut *tx)
        .await
        .map_err(db)?;
        tx.commit().await.map_err(db)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sqlx::sqlite::SqlitePoolOptions;
    use std::time::Duration;

    /// A file-backed (not `:memory:`) pool with `max_connections` real
    /// connections sharing one database — needed for the concurrency tests
    /// below. A bare `sqlite::memory:` URL gives every pooled connection its
    /// own private empty database, which would defeat the point of racing
    /// two connections against the same rows.
    async fn file_pool(max_connections: u32) -> (SqlitePool, tempfile::TempDir) {
        let dir = tempfile::tempdir().expect("create temp dir");
        let path = dir.path().join("push-gateway-test.db");
        let pool = SqlitePoolOptions::new()
            .max_connections(max_connections)
            .connect_with(
                sqlx::sqlite::SqliteConnectOptions::new()
                    .filename(&path)
                    .create_if_missing(true)
                    .busy_timeout(Duration::from_secs(10)),
            )
            .await
            .expect("open file-backed sqlite pool");
        SqliteAuthorityStore::apply_migrations(&pool)
            .await
            .expect("apply migrations");
        (pool, dir)
    }

    #[tokio::test]
    async fn readiness_requires_migrated_schema() {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .expect("open in-memory sqlite pool");
        let store = SqliteAuthorityStore::new(pool.clone());
        assert!(store.ready().await.is_err(), "unmigrated database is not ready");

        SqliteAuthorityStore::apply_migrations(&pool)
            .await
            .expect("apply migrations");
        assert!(store.ready().await.is_ok(), "migrated database is ready");
    }

    #[tokio::test]
    async fn reaper_deletes_active_child_of_retention_eligible_revoked_installation() {
        let (pool, _dir) = file_pool(1).await;
        let store = SqliteAuthorityStore::new(pool.clone());
        let now = Utc::now();
        let installation_id = Uuid::new_v4();
        sqlx::query(
            "INSERT INTO push_gateway_installations(id,app_attest_key_id,app_attest_public_key,assertion_counter,app_profile,token_ciphertext,token_fingerprint,endpoint_epoch,expires_at,revoked_at)
             VALUES (?,?,?,0,'buzz-ios-production',?,?,1,?,?)",
        )
        .bind(installation_id.to_string())
        .bind(vec![1u8])
        .bind(vec![2u8; 33])
        .bind(vec![3u8])
        .bind(vec![4u8; 32])
        .bind(now + chrono::Duration::days(30))
        .bind(now - chrono::Duration::days(2))
        .execute(&pool)
        .await
        .expect("insert retention-eligible revoked installation");
        sqlx::query(
            "INSERT INTO push_gateway_delegations(id,installation_id,relay_pubkey,endpoint_epoch,generation,not_before,expires_at,revoked_at)
             VALUES (?,?,?,1,1,?,?,NULL)",
        )
        .bind(Uuid::new_v4().to_string())
        .bind(installation_id.to_string())
        .bind(vec![5u8; 32])
        .bind(now - chrono::Duration::days(1))
        .bind(now + chrono::Duration::days(7))
        .execute(&pool)
        .await
        .expect("insert active future-expiring child delegation");

        store
            .reap_expired(now.timestamp())
            .await
            .expect("reaper must delete the child before its revoked parent");
        let delegations: i64 = sqlx::query_scalar("SELECT count(*) FROM push_gateway_delegations")
            .fetch_one(&pool)
            .await
            .expect("count delegations");
        let installations: i64 =
            sqlx::query_scalar("SELECT count(*) FROM push_gateway_installations")
                .fetch_one(&pool)
                .await
                .expect("count installations");
        assert_eq!(delegations, 0);
        assert_eq!(installations, 0);
    }

    const RELAY_HEX: &str = "11111111111111111111111111111111111111111111111111111111111111aa";
    const DELEGATION_ID: u128 = 2;

    // One installation + one live delegation that admits at now=1_000.
    async fn install_authority(pool: &SqlitePool) {
        let now = Utc::now();
        sqlx::query(
            "INSERT INTO push_gateway_installations(id,app_attest_key_id,app_attest_public_key,assertion_counter,app_profile,token_ciphertext,token_fingerprint,endpoint_epoch,expires_at)
             VALUES (?,?,?,0,'buzz-ios-production',?,?,1,?)",
        )
        .bind(Uuid::from_u128(1).to_string())
        .bind(vec![1u8])
        .bind(vec![2u8; 33])
        .bind(vec![3u8])
        .bind(vec![4u8; 32])
        .bind(now + chrono::Duration::days(30))
        .execute(pool)
        .await
        .expect("insert installation");
        sqlx::query(
            "INSERT INTO push_gateway_delegations(id,installation_id,relay_pubkey,endpoint_epoch,generation,not_before,expires_at,revoked_at)
             VALUES (?,?,?,1,1,?,?,NULL)",
        )
        .bind(Uuid::from_u128(DELEGATION_ID).to_string())
        .bind(Uuid::from_u128(1).to_string())
        .bind(hex::decode(RELAY_HEX).unwrap())
        .bind(now - chrono::Duration::days(1))
        .bind(now + chrono::Duration::days(7))
        .execute(pool)
        .await
        .expect("insert delegation");
    }

    fn admit<'a>(
        store: &'a SqliteAuthorityStore,
        event_hex: &'a str,
        request_id: Uuid,
    ) -> impl std::future::Future<Output = Result<DeliveryPermit, AuthorityError>> + 'a {
        admit_with_quota(store, event_hex, request_id, 10)
    }

    fn admit_with_quota<'a>(
        store: &'a SqliteAuthorityStore,
        event_hex: &'a str,
        request_id: Uuid,
        quota_max_deliveries: i64,
    ) -> impl std::future::Future<Output = Result<DeliveryPermit, AuthorityError>> + 'a {
        let now = Utc::now().timestamp();
        store.authorize_delivery(
            Uuid::from_u128(DELEGATION_ID),
            RELAY_HEX,
            1,
            1,
            event_hex,
            request_id,
            now + 300,
            60,
            quota_max_deliveries,
            now,
        )
    }

    // Two concurrent admissions colliding on the same (relay,request_id) PK must
    // admit exactly once; the loser rejects with its whole tx rolled back, so
    // quota is charged once and the auth-event fence is not consumed by the loser.
    #[tokio::test]
    async fn concurrent_same_request_id_admits_exactly_once() {
        let (pool, _dir) = file_pool(4).await;
        install_authority(&pool).await;
        let store = SqliteAuthorityStore::new(pool.clone());
        let request_id = Uuid::new_v4();
        let event_a = "22".repeat(32);
        let event_b = "33".repeat(32);

        let (a, b) = tokio::join!(
            admit(&store, &event_a, request_id),
            admit(&store, &event_b, request_id),
        );
        assert_eq!(
            [a.is_ok(), b.is_ok()].iter().filter(|ok| **ok).count(),
            1,
            "exactly one concurrent same-request_id admission may win"
        );

        let requests: i64 =
            sqlx::query_scalar("SELECT count(*) FROM push_gateway_delivery_request_replays")
                .fetch_one(&pool)
                .await
                .expect("count request replays");
        assert_eq!(requests, 1, "winner leaves exactly one request-id fence");
        let auth_events: i64 =
            sqlx::query_scalar("SELECT count(*) FROM push_gateway_delivery_auth_replays")
                .fetch_one(&pool)
                .await
                .expect("count auth replays");
        assert_eq!(auth_events, 1, "loser's auth-event insert rolled back");
        let admitted: i64 = sqlx::query_scalar("SELECT admitted FROM push_gateway_endpoint_quotas")
            .fetch_one(&pool)
            .await
            .expect("read quota");
        assert_eq!(admitted, 1, "loser's quota reservation rolled back");
    }

    // Red-team: quota ceiling under concurrency. Two admissions for the SAME
    // endpoint fingerprint but DISTINCT request_ids and DISTINCT auth events —
    // so neither replay PK fence can gate them; the only thing standing between
    // the caller and over-admission is the quota upsert's `WHERE ... admitted <
    // ?` predicate, now re-checked serially because `BEGIN IMMEDIATE` forces
    // the second transaction to start only after the first fully commits.
    #[tokio::test]
    async fn concurrent_admissions_never_over_admit_past_quota_ceiling() {
        let (pool, _dir) = file_pool(4).await;
        install_authority(&pool).await;
        let store = SqliteAuthorityStore::new(pool.clone());
        let event_a = "22".repeat(32);
        let event_b = "33".repeat(32);

        let (a, b) = tokio::join!(
            admit_with_quota(&store, &event_a, Uuid::new_v4(), 1),
            admit_with_quota(&store, &event_b, Uuid::new_v4(), 1),
        );
        assert_eq!(
            [a.is_ok(), b.is_ok()].iter().filter(|ok| **ok).count(),
            1,
            "quota ceiling of 1 admits exactly one of two concurrent attempts"
        );

        let admitted: i64 = sqlx::query_scalar("SELECT admitted FROM push_gateway_endpoint_quotas")
            .fetch_one(&pool)
            .await
            .expect("read quota");
        assert_eq!(
            admitted, 1,
            "persisted admitted counter must never exceed the ceiling under a race"
        );
        // The loser's whole tx rolled back: its distinct auth event is not fenced.
        let auth_events: i64 =
            sqlx::query_scalar("SELECT count(*) FROM push_gateway_delivery_auth_replays")
                .fetch_one(&pool)
                .await
                .expect("count auth replays");
        assert_eq!(auth_events, 1, "rejected admission consumes no auth fence");
        let requests: i64 =
            sqlx::query_scalar("SELECT count(*) FROM push_gateway_delivery_request_replays")
                .fetch_one(&pool)
                .await
                .expect("count request replays");
        assert_eq!(requests, 1, "rejected admission consumes no request fence");
    }

    // Red-team: Retryable release is unconditional (deletes the request-id row on
    // the pool, not inside a tx). Attack the window where a losing delivery's
    // release races a fresh admission that legitimately re-took the same
    // request_id — could the stale DELETE punch a hole in the live fence? It
    // cannot: the DELETE keys on (relay_pubkey, request_id) with no ownership
    // token, but the fence it would remove is exactly the one the retrying caller
    // is entitled to free, and any *subsequent* admission re-inserts its own row.
    // Concretely: admit R, Retryable-release R (fence gone), re-admit R (fresh
    // fence), then replay the SAME release a second time (a duplicated/late
    // finish) — it must delete the NOW-LIVE fence, and the next admission of R
    // must still be gated by whatever fence remains. This pins that a duplicated
    // Retryable finish is idempotent-safe and never leaves R permanently
    // un-fenceable while the delegation is live.
    #[tokio::test]
    async fn duplicated_retryable_release_does_not_permanently_unfence_request_id() {
        let (pool, _dir) = file_pool(2).await;
        install_authority(&pool).await;
        let store = SqliteAuthorityStore::new(pool.clone());
        let request_id = Uuid::new_v4();

        let permit = admit(&store, &"22".repeat(32), request_id)
            .await
            .expect("first admission");
        store
            .finish_delivery(permit, DeliveryDisposition::Retryable)
            .await
            .expect("retryable release frees the fence");
        // Re-admit: fresh fence for the same request_id.
        let permit2 = admit(&store, &"33".repeat(32), request_id)
            .await
            .expect("re-admit after release");
        // A duplicated/late Retryable finish for the same (relay, request_id)
        // deletes the now-live fence — this is the worst case for the
        // unconditional DELETE. It must not error, and R must remain re-admittable
        // (fence hole is transient, never permanent), which is the honest
        // NIP-PL §312 contract: a still-live endpoint gets a fresh job.
        store
            .finish_delivery(permit2, DeliveryDisposition::Retryable)
            .await
            .expect("duplicated retryable release is idempotent-safe");
        let admitted_again = admit(&store, &"44".repeat(32), request_id).await;
        assert!(
            admitted_again.is_ok(),
            "after any Retryable release the request_id is re-admittable, never permanently unfenceable"
        );
        // And a Terminal on that live permit re-burns it, closing the window.
        store
            .finish_delivery(admitted_again.unwrap(), DeliveryDisposition::Terminal)
            .await
            .expect("terminal finish");
        assert!(
            admit(&store, &"55".repeat(32), request_id).await.is_err(),
            "terminal keeps the fence burned after the release churn"
        );
    }

    // Retryable release must free the real request-id PK: after finish_delivery,
    // the same request_id re-admits with a fresh auth event; a Terminal finish
    // leaves it burned.
    #[tokio::test]
    async fn retryable_release_frees_request_id() {
        let (pool, _dir) = file_pool(2).await;
        install_authority(&pool).await;
        let store = SqliteAuthorityStore::new(pool.clone());
        let request_id = Uuid::new_v4();

        let permit = admit(&store, &"22".repeat(32), request_id)
            .await
            .expect("first admission");
        store
            .finish_delivery(permit, DeliveryDisposition::Retryable)
            .await
            .expect("retryable release");
        // Same request_id, fresh auth event: released PK admits again.
        let permit = admit(&store, &"33".repeat(32), request_id)
            .await
            .expect("retryable release frees the request-id PK");
        // Terminal now burns it: a further re-admit with the same request_id fails.
        store
            .finish_delivery(permit, DeliveryDisposition::Terminal)
            .await
            .expect("terminal finish");
        assert!(
            admit(&store, &"44".repeat(32), request_id).await.is_err(),
            "terminal outcome keeps the request-id fence burned"
        );
    }
}
