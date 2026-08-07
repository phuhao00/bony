//! Cache-key invalidation, fanned out to local subscribers in-process.
//!
//! The relay keeps in-memory (moka) membership / accessible-channels /
//! visibility caches. This module carries a cache-key drop from the write
//! path to every local consumer of
//! [`crate::PubSubManager::subscribe_cache_invalidations`] via a
//! `tokio::sync::broadcast` channel — no serialization, no network hop.
//!
//! The message is a pure cache-key drop — never an "evict these
//! subscriptions" payload. The per-event access gate
//! (`filter_fanout_by_access`) is the universal delivery-enforcement point,
//! so dropping the stale key is sufficient: the next read re-fetches
//! authoritative state from the DB.

use buzz_core::CommunityId;
use uuid::Uuid;

/// A cache-key drop to apply locally. Each variant mirrors exactly one of the
/// relay's local `invalidate_*` operations. The community is carried by
/// [`ScopedCacheInvalidation`], not by the tenant-local operation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CacheInvalidation {
    /// Drop the `(channel_id, pubkey)` membership entry and the user's
    /// accessible-channels entry. Mirrors `invalidate_membership`.
    Membership {
        /// Channel whose membership changed.
        channel_id: Uuid,
        /// Affected member's pubkey bytes.
        pubkey: Vec<u8>,
    },
    /// Drop every user's accessible-channels entry. Mirrors
    /// `invalidate_all_accessible_channels` (e.g. a new open channel).
    AccessibleAll,
    /// Drop the cached visibility for a single channel. Mirrors
    /// `invalidate_channel_visibility` (e.g. an open→private flip).
    Visibility {
        /// Channel whose visibility changed.
        channel_id: Uuid,
    },
    /// Drop all membership / accessible / visibility caches. Mirrors
    /// `invalidate_channel_deleted`.
    ChannelDeleted,
}

/// A cache invalidation scoped to the community whose cache entries it drops.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScopedCacheInvalidation {
    /// Community whose local cache key should be dropped.
    pub community_id: CommunityId,
    /// Tenant-local cache invalidation operation.
    pub invalidation: CacheInvalidation,
}
