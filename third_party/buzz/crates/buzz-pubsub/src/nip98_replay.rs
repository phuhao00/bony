//! In-process NIP-98 replay seen-set.
//!
//! Implements the [`Nip98ReplayGuard`] trait from `buzz-auth`. Backed by a
//! `DashMap<String, Instant>` of expiry timestamps — single-instance
//! equivalent of the previous Redis `SET NX EX`: `entry().or_insert_with()`
//! is the atomic set-if-absent, and an expired entry is treated as absent and
//! overwritten in place.

use std::time::{Duration, Instant};

use buzz_auth::{
    error::AuthError,
    nip98_replay::{
        nip98_replay_key_for_scope, Nip98ReplayGuard, DEFAULT_REPLAY_TTL_SECS, MAX_REPLAY_TTL_SECS,
    },
};
use dashmap::DashMap;
use nostr::EventId;

/// In-process NIP-98 replay seen-set.
///
/// Each `try_mark(ctx, event_id, ttl)` claims `buzz:{community}:nip98:{event_id_hex}`
/// with an expiry `Instant`. The first claim within the TTL window wins;
/// subsequent claims before expiry are rejected as replay.
#[derive(Default)]
pub struct InMemoryNip98ReplayGuard {
    seen: DashMap<String, Instant>,
}

impl InMemoryNip98ReplayGuard {
    /// Creates a new, empty in-process replay guard.
    pub fn new() -> Self {
        Self::default()
    }
}

impl Nip98ReplayGuard for InMemoryNip98ReplayGuard {
    fn try_mark_in_scope<'a>(
        &'a self,
        scope: &'a str,
        event_id: &'a EventId,
        ttl_secs: u64,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<bool, AuthError>> + Send + 'a>>
    {
        Box::pin(async move {
            // §5 gate floor + safety ceiling, same clamp as the previous
            // Redis-backed guard (a Redis `EX` arg limit no longer applies,
            // but the bounded-TTL contract is unchanged).
            let ttl = ttl_secs.clamp(DEFAULT_REPLAY_TTL_SECS, MAX_REPLAY_TTL_SECS);
            let key = nip98_replay_key_for_scope(scope, event_id);
            let now = Instant::now();
            let expires_at = now + Duration::from_secs(ttl);

            let mut claimed = false;
            self.seen
                .entry(key)
                .and_modify(|existing| {
                    if now >= *existing {
                        *existing = expires_at;
                        claimed = true;
                    }
                })
                .or_insert_with(|| {
                    claimed = true;
                    expires_at
                });

            Ok(claimed)
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nostr::{EventBuilder, Keys, Kind};

    fn fresh_event_id() -> EventId {
        EventBuilder::new(Kind::HttpAuth, "")
            .sign_with_keys(&Keys::generate())
            .expect("sign")
            .id
    }

    #[tokio::test]
    async fn first_claim_succeeds_replay_fails() {
        let guard = InMemoryNip98ReplayGuard::new();
        let eid = fresh_event_id();

        assert!(guard
            .try_mark_in_scope("scope-a", &eid, DEFAULT_REPLAY_TTL_SECS)
            .await
            .expect("first mark"));
        assert!(!guard
            .try_mark_in_scope("scope-a", &eid, DEFAULT_REPLAY_TTL_SECS)
            .await
            .expect("replay mark"));
    }

    #[tokio::test]
    async fn isolation_between_scopes() {
        let guard = InMemoryNip98ReplayGuard::new();
        let eid = fresh_event_id();

        assert!(guard
            .try_mark_in_scope("scope-a", &eid, DEFAULT_REPLAY_TTL_SECS)
            .await
            .expect("mark in A"));
        // Same event id under a different scope is still a first claim —
        // scopes are independent seen-sets.
        assert!(guard
            .try_mark_in_scope("scope-b", &eid, DEFAULT_REPLAY_TTL_SECS)
            .await
            .expect("mark in B"));
    }

    #[tokio::test]
    async fn sub_floor_ttl_is_lifted_to_default() {
        let guard = InMemoryNip98ReplayGuard::new();
        let eid = fresh_event_id();

        assert!(guard
            .try_mark_in_scope("scope-a", &eid, 30)
            .await
            .expect("mark"));
        assert!(!guard
            .try_mark_in_scope("scope-a", &eid, 30)
            .await
            .expect("replay"));
    }

    #[tokio::test]
    async fn above_ceiling_ttl_is_clamped() {
        let guard = InMemoryNip98ReplayGuard::new();
        let eid = fresh_event_id();

        assert!(guard
            .try_mark_in_scope("scope-a", &eid, u64::MAX)
            .await
            .expect("mark with extreme ttl must succeed via clamp"));
        assert!(!guard
            .try_mark_in_scope("scope-a", &eid, u64::MAX)
            .await
            .expect("replay with extreme ttl must succeed via clamp"));
    }

    #[tokio::test]
    async fn expired_entry_can_be_reclaimed() {
        let guard = InMemoryNip98ReplayGuard::new();
        let eid = fresh_event_id();

        assert!(guard
            .try_mark_in_scope("scope-a", &eid, 30)
            .await
            .expect("mark"));
        // Force expiry by writing a past timestamp directly.
        guard
            .seen
            .insert(nip98_replay_key_for_scope("scope-a", &eid), Instant::now());
        tokio::time::sleep(Duration::from_millis(5)).await;
        assert!(guard
            .try_mark_in_scope("scope-a", &eid, 30)
            .await
            .expect("reclaim after expiry"));
    }
}
