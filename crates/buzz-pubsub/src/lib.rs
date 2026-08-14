#![deny(unsafe_code)]
#![warn(missing_docs)]
//! `buzz-pubsub` — in-process event fan-out, presence, rate limiting, and
//! NIP-98 replay guard for a single-instance Buzz relay.
//!
//! # Architecture
//!
//! ```text
//! buzz-relay process
//!   └── PubSubManager
//!         ├── broadcast::channel(4096)  → ChannelEvent          (WS fan-out)
//!         ├── broadcast::channel(4096)  → ScopedCacheInvalidation (cache drops)
//!         ├── broadcast::channel(4096)  → ScopedConnControl      (live bans)
//!         ├── PresenceStore             (DashMap, TTL on read)
//!         ├── InMemoryRateLimiter       (DashMap, fixed window)
//!         └── InMemoryNip98ReplayGuard  (DashMap, TTL on read)
//! ```
//!
//! Everything here used to be backed by Redis so state could be shared
//! across pods. This deployment target is single-instance, so all of it now
//! lives in-process: no network hop, no serialization, no external service to
//! run. Lagged receivers still get `RecvError::Lagged` from the underlying
//! `tokio::sync::broadcast` channels.

/// Cache-key invalidation, fanned out to local subscribers in-process.
pub mod cache_invalidation;
/// Connection-control commands, fanned out to local subscribers in-process.
pub mod conn_control;
/// Error types for pub/sub operations.
pub mod error;
/// In-process NIP-98 replay seen-set.
pub mod nip98_replay;
pub use nip98_replay::InMemoryNip98ReplayGuard;
/// Online/offline presence tracking, in-process.
pub mod presence;
/// In-process fixed-window rate limiter.
pub mod rate_limiter;
pub use rate_limiter::InMemoryRateLimiter;
/// Tenant-scoped event routing topics.
pub mod topic;
pub use error::PubSubError;

use std::collections::HashMap;

use buzz_core::TenantContext;
use nostr::PublicKey;
use tokio::sync::{broadcast, Mutex};

use crate::cache_invalidation::{CacheInvalidation, ScopedCacheInvalidation};
use crate::conn_control::{ConnControl, ScopedConnControl};
use crate::presence::PresenceStore;
pub use crate::topic::{EventTopic, EventTopicKey};

/// A Nostr event routed to a scoped topic, broadcast to local subscribers.
#[derive(Debug, Clone)]
pub struct ChannelEvent {
    /// Server-resolved community that scoped the topic.
    pub community_id: buzz_core::CommunityId,
    /// Tenant-local routing scope for this event.
    pub topic: EventTopic,
    /// The Nostr event payload.
    pub event: nostr::Event,
}

/// Central pub/sub manager for a single-instance Buzz relay.
pub struct PubSubManager {
    /// Local desired topic refcounts. Retained purely for observability
    /// (`topic_refcount`) — every event still fans out on the single shared
    /// `broadcast_tx` regardless of refcount, since there is no remote
    /// subscription to open or close in-process.
    desired_topics: Mutex<HashMap<EventTopicKey, usize>>,
    broadcast_tx: broadcast::Sender<ChannelEvent>,
    cache_invalidation_tx: broadcast::Sender<ScopedCacheInvalidation>,
    conn_control_tx: broadcast::Sender<ScopedConnControl>,
    presence: PresenceStore,
}

impl Default for PubSubManager {
    fn default() -> Self {
        Self::new()
    }
}

impl PubSubManager {
    /// Creates a new, empty in-process `PubSubManager`.
    pub fn new() -> Self {
        let (broadcast_tx, _) = broadcast::channel(4096);
        let (cache_invalidation_tx, _) = broadcast::channel(4096);
        let (conn_control_tx, _) = broadcast::channel(4096);

        Self {
            desired_topics: Mutex::new(HashMap::new()),
            broadcast_tx,
            cache_invalidation_tx,
            conn_control_tx,
            presence: PresenceStore::new(),
        }
    }

    /// Returns a new broadcast receiver for locally-published channel events.
    pub fn subscribe_local(&self) -> broadcast::Receiver<ChannelEvent> {
        self.broadcast_tx.subscribe()
    }

    /// Retain local interest in an event topic.
    ///
    /// Bookkeeping only — every event fans out on the shared broadcast
    /// channel regardless of refcount, so this never gates delivery. Kept so
    /// callers that track topic lifetime for logging/metrics need no changes.
    pub async fn retain_topic(&self, ctx: &TenantContext, topic: EventTopic) {
        let topic_key = EventTopicKey::from_context(ctx, topic);
        let mut desired = self.desired_topics.lock().await;
        *desired.entry(topic_key).or_insert(0) += 1;
    }

    /// Release local interest in an event topic. See [`Self::retain_topic`].
    pub async fn release_topic(&self, ctx: &TenantContext, topic: EventTopic) {
        let topic_key = EventTopicKey::from_context(ctx, topic);
        let mut desired = self.desired_topics.lock().await;
        let Some(count) = desired.get_mut(&topic_key) else {
            tracing::warn!(?topic_key, "release_topic called for unretained topic");
            return;
        };
        *count -= 1;
        if *count == 0 {
            desired.remove(&topic_key);
        }
    }

    /// Current local desired refcount for tests and metrics.
    pub async fn topic_refcount(&self, ctx: &TenantContext, topic: EventTopic) -> usize {
        let topic_key = EventTopicKey::from_context(ctx, topic);
        self.desired_topics
            .lock()
            .await
            .get(&topic_key)
            .copied()
            .unwrap_or(0)
    }

    /// Returns a new broadcast receiver for cache-invalidation drops.
    pub fn subscribe_cache_invalidations(&self) -> broadcast::Receiver<ScopedCacheInvalidation> {
        self.cache_invalidation_tx.subscribe()
    }

    /// Returns a new broadcast receiver for connection-control commands.
    pub fn subscribe_conn_control(&self) -> broadcast::Receiver<ScopedConnControl> {
        self.conn_control_tx.subscribe()
    }

    /// Publish a cache-key drop to all local subscribers. Fire-and-forget at
    /// the call site: the local cache is already dropped synchronously; this
    /// only matters for the consumer loop that mirrors it onto derived
    /// caches. Returns the number of active local subscribers.
    pub async fn publish_cache_invalidation(
        &self,
        ctx: &TenantContext,
        invalidation: &CacheInvalidation,
    ) -> Result<i64, PubSubError> {
        let scoped = ScopedCacheInvalidation {
            community_id: ctx.community(),
            invalidation: invalidation.clone(),
        };
        let _ = self.cache_invalidation_tx.send(scoped);
        Ok(self.cache_invalidation_tx.receiver_count() as i64)
    }

    /// Publish a connection-control command to all local subscribers. Used
    /// for live ban enforcement: the DB ban row is the durable backstop, so a
    /// dropped send still refuses the next auth attempt.
    pub async fn publish_conn_control(
        &self,
        ctx: &TenantContext,
        command: &ConnControl,
    ) -> Result<i64, PubSubError> {
        let scoped = ScopedConnControl {
            community_id: ctx.community(),
            command: command.clone(),
        };
        let _ = self.conn_control_tx.send(scoped);
        Ok(self.conn_control_tx.receiver_count() as i64)
    }

    /// Publish an event to local subscribers. Returns the number of active
    /// local subscribers.
    ///
    /// Routing note (NIP-ER author-private reminders): `topic` is a routing
    /// label, not an isolation boundary — every event still flows through the
    /// single shared channel. The actual author-only delivery boundary is
    /// `filter_fanout_by_access` in the relay, applied by every consumer of
    /// [`Self::subscribe_local`].
    pub async fn publish_event(
        &self,
        ctx: &TenantContext,
        topic: EventTopic,
        event: &nostr::Event,
    ) -> Result<i64, PubSubError> {
        let channel_event = ChannelEvent {
            community_id: ctx.community(),
            topic,
            event: event.clone(),
        };
        let _ = self.broadcast_tx.send(channel_event);
        Ok(self.broadcast_tx.receiver_count() as i64)
    }

    /// Set presence with [`presence::PRESENCE_TTL_SECS`] TTL. Call on connect
    /// and every 60s heartbeat.
    pub async fn set_presence(&self, ctx: &TenantContext, pubkey: &PublicKey, status: &str) {
        self.presence.set(ctx, pubkey, status);
    }

    /// Remove presence for `pubkey`. Call on clean disconnect.
    pub async fn clear_presence(&self, ctx: &TenantContext, pubkey: &PublicKey) {
        self.presence.clear(ctx, pubkey);
    }

    /// Returns the current presence status for `pubkey`, or `None` if not set.
    pub async fn get_presence(&self, ctx: &TenantContext, pubkey: &PublicKey) -> Option<String> {
        self.presence.get(ctx, pubkey)
    }

    /// Returns presence statuses for multiple pubkeys as a `pubkey_hex → status` map.
    pub async fn get_presence_bulk(
        &self,
        ctx: &TenantContext,
        pubkeys: &[PublicKey],
    ) -> HashMap<String, String> {
        self.presence.get_bulk(ctx, pubkeys)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use buzz_core::{CommunityId, TenantContext};
    use nostr::{EventBuilder, Keys, Kind};
    use std::sync::Arc;
    use uuid::Uuid;

    fn make_manager() -> Arc<PubSubManager> {
        Arc::new(PubSubManager::new())
    }

    fn ctx(id: u128, host: &str) -> TenantContext {
        TenantContext::resolved(CommunityId::from_uuid(Uuid::from_u128(id)), host)
    }

    #[tokio::test]
    async fn publish_and_subscribe_roundtrip() {
        let manager = make_manager();
        let mut rx = manager.subscribe_local();

        let ctx = ctx(0xaaaa, "a.example");
        let channel_id = Uuid::new_v4();
        let keys = Keys::generate();
        let event = EventBuilder::new(Kind::TextNote, "hello pubsub")
            .tags([])
            .sign_with_keys(&keys)
            .expect("signing failed");
        let event_id = event.id;

        manager
            .retain_topic(&ctx, EventTopic::Channel(channel_id))
            .await;

        manager
            .publish_event(&ctx, EventTopic::Channel(channel_id), &event)
            .await
            .expect("publish failed");

        let received = tokio::time::timeout(tokio::time::Duration::from_secs(2), rx.recv())
            .await
            .expect("timeout")
            .expect("channel closed");

        assert_eq!(received.community_id, ctx.community());
        assert_eq!(received.topic, EventTopic::Channel(channel_id));
        assert_eq!(received.event.id, event_id);
    }

    #[tokio::test]
    async fn cache_invalidation_roundtrip() {
        let manager = make_manager();
        let mut rx = manager.subscribe_cache_invalidations();

        let channel_id = Uuid::new_v4();
        let pubkey = Keys::generate().public_key().to_bytes().to_vec();
        let sent = CacheInvalidation::Membership {
            channel_id,
            pubkey: pubkey.clone(),
        };

        let ctx = ctx(0xaaaa, "a.example");

        manager
            .publish_cache_invalidation(&ctx, &sent)
            .await
            .expect("publish failed");

        let received = tokio::time::timeout(tokio::time::Duration::from_secs(2), rx.recv())
            .await
            .expect("timeout")
            .expect("channel closed");

        assert_eq!(
            received,
            ScopedCacheInvalidation {
                community_id: ctx.community(),
                invalidation: sent,
            }
        );
    }

    #[tokio::test]
    async fn conn_control_roundtrip() {
        let manager = make_manager();
        let mut rx = manager.subscribe_conn_control();
        let ctx = ctx(0xaaaa, "a.example");

        manager
            .publish_conn_control(&ctx, &ConnControl::DisconnectCommunity)
            .await
            .expect("publish failed");

        let received = tokio::time::timeout(tokio::time::Duration::from_secs(2), rx.recv())
            .await
            .expect("timeout")
            .expect("channel closed");

        assert_eq!(received.community_id, ctx.community());
        assert_eq!(received.command, ConnControl::DisconnectCommunity);
    }

    #[tokio::test]
    async fn presence_set_get_clear_roundtrip() {
        let manager = make_manager();
        let pubkey = Keys::generate().public_key();
        let ctx = ctx(0xaaaa, "a.example");

        assert!(manager.get_presence(&ctx, &pubkey).await.is_none());

        manager.set_presence(&ctx, &pubkey, "online").await;
        assert_eq!(
            manager.get_presence(&ctx, &pubkey).await.as_deref(),
            Some("online")
        );

        manager.clear_presence(&ctx, &pubkey).await;
        assert!(manager.get_presence(&ctx, &pubkey).await.is_none());
    }

    #[tokio::test]
    async fn same_channel_id_in_two_communities_release_one_keeps_other_live() {
        let manager = make_manager();

        let ctx_a = ctx(0xaaaa, "a.example");
        let ctx_b = ctx(0xbbbb, "b.example");
        let channel_id = Uuid::from_u128(0xcccc);
        let topic = EventTopic::Channel(channel_id);

        manager.retain_topic(&ctx_a, topic).await;
        manager.retain_topic(&ctx_b, topic).await;

        assert_eq!(manager.topic_refcount(&ctx_a, topic).await, 1);
        assert_eq!(manager.topic_refcount(&ctx_b, topic).await, 1);

        manager.release_topic(&ctx_a, topic).await;
        assert_eq!(manager.topic_refcount(&ctx_a, topic).await, 0);
        assert_eq!(manager.topic_refcount(&ctx_b, topic).await, 1);

        manager.release_topic(&ctx_b, topic).await;
        assert_eq!(manager.topic_refcount(&ctx_b, topic).await, 0);
    }

    #[tokio::test]
    async fn retain_release_refcounts() {
        let manager = PubSubManager::new();
        let ctx = ctx(0xaaaa, "a.example");
        let topic = EventTopic::Channel(Uuid::from_u128(0xbbbb));

        assert_eq!(manager.topic_refcount(&ctx, topic).await, 0);

        manager.retain_topic(&ctx, topic).await;
        manager.retain_topic(&ctx, topic).await;
        assert_eq!(manager.topic_refcount(&ctx, topic).await, 2);

        manager.release_topic(&ctx, topic).await;
        assert_eq!(manager.topic_refcount(&ctx, topic).await, 1);

        manager.release_topic(&ctx, topic).await;
        assert_eq!(manager.topic_refcount(&ctx, topic).await, 0);
    }
}
