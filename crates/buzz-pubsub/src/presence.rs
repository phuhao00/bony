//! Presence tracking — online/away status with TTL, in-process.
//!
//! Backed by a `DashMap<(CommunityId, [u8; 32]), (String, Instant)>`. TTL is
//! checked lazily on read: an entry older than [`PRESENCE_TTL_SECS`] is
//! treated as absent and removed. TTL is 3x the 60s heartbeat interval so a
//! single missed heartbeat doesn't cause presence flap. Clean disconnect
//! removes the entry immediately.

use std::collections::HashMap;
use std::time::Instant;

use buzz_core::{CommunityId, TenantContext};
use dashmap::DashMap;
use nostr::PublicKey;

/// 3x the 60s heartbeat — single missed heartbeat won't cause presence flap.
pub const PRESENCE_TTL_SECS: u64 = 180;

type PresenceKey = (CommunityId, [u8; 32]);

/// In-process presence store, keyed by `(community, pubkey)`.
#[derive(Default)]
pub struct PresenceStore {
    entries: DashMap<PresenceKey, (String, Instant)>,
}

fn key(ctx: &TenantContext, pubkey: &PublicKey) -> PresenceKey {
    (ctx.community(), pubkey.to_bytes())
}

impl PresenceStore {
    /// Creates an empty presence store.
    pub fn new() -> Self {
        Self::default()
    }

    /// Sets presence status for `pubkey` with a [`PRESENCE_TTL_SECS`]-second TTL.
    pub fn set(&self, ctx: &TenantContext, pubkey: &PublicKey, status: &str) {
        self.entries
            .insert(key(ctx, pubkey), (status.to_string(), Instant::now()));
    }

    /// Removes the presence entry for `pubkey`. Call on clean disconnect.
    pub fn clear(&self, ctx: &TenantContext, pubkey: &PublicKey) {
        self.entries.remove(&key(ctx, pubkey));
    }

    /// Returns the current presence status for `pubkey`, or `None` if not set or expired.
    pub fn get(&self, ctx: &TenantContext, pubkey: &PublicKey) -> Option<String> {
        let k = key(ctx, pubkey);
        let entry = self.entries.get(&k)?;
        let (status, set_at) = entry.value();
        if set_at.elapsed().as_secs() > PRESENCE_TTL_SECS {
            drop(entry);
            self.entries.remove(&k);
            return None;
        }
        Some(status.clone())
    }

    /// Returns `pubkey_hex → status` for all currently-live entries among `pubkeys`.
    pub fn get_bulk(&self, ctx: &TenantContext, pubkeys: &[PublicKey]) -> HashMap<String, String> {
        pubkeys
            .iter()
            .filter_map(|pk| self.get(ctx, pk).map(|status| (pk.to_hex(), status)))
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use buzz_core::{CommunityId, TenantContext};
    use nostr::Keys;
    use uuid::Uuid;

    fn make_pubkey() -> PublicKey {
        Keys::generate().public_key()
    }

    fn ctx(id: u128, host: &str) -> TenantContext {
        TenantContext::resolved(CommunityId::from_uuid(Uuid::from_u128(id)), host)
    }

    #[test]
    fn presence_ttl_is_three_one_minute_heartbeat_windows() {
        assert_eq!(PRESENCE_TTL_SECS, 180);
        assert_eq!(PRESENCE_TTL_SECS, 3 * 60);
    }

    #[test]
    fn same_pubkey_in_two_communities_is_independent() {
        let store = PresenceStore::new();
        let pubkey = make_pubkey();
        let community_a = ctx(0xaaaa, "a.example");
        let community_b = ctx(0xbbbb, "b.example");

        store.set(&community_a, &pubkey, "online");
        assert_eq!(store.get(&community_a, &pubkey).as_deref(), Some("online"));
        assert_eq!(store.get(&community_b, &pubkey), None);
    }

    #[test]
    fn set_get_clear_roundtrip() {
        let store = PresenceStore::new();
        let pubkey = make_pubkey();
        let ctx = ctx(0xaaaa, "a.example");

        assert!(store.get(&ctx, &pubkey).is_none());

        store.set(&ctx, &pubkey, "online");
        assert_eq!(store.get(&ctx, &pubkey).as_deref(), Some("online"));

        store.set(&ctx, &pubkey, "away");
        assert_eq!(store.get(&ctx, &pubkey).as_deref(), Some("away"));

        store.clear(&ctx, &pubkey);
        assert!(store.get(&ctx, &pubkey).is_none());
    }

    #[test]
    fn bulk_returns_only_live_entries() {
        let store = PresenceStore::new();
        let pk1 = make_pubkey();
        let pk2 = make_pubkey();
        let pk3 = make_pubkey();
        let ctx = ctx(0xaaaa, "a.example");

        store.set(&ctx, &pk1, "online");
        store.set(&ctx, &pk2, "away");

        let result = store.get_bulk(&ctx, &[pk1, pk2, pk3]);

        assert_eq!(result.get(&pk1.to_hex()).map(|s| s.as_str()), Some("online"));
        assert_eq!(result.get(&pk2.to_hex()).map(|s| s.as_str()), Some("away"));
        assert!(!result.contains_key(&pk3.to_hex()));
    }
}
