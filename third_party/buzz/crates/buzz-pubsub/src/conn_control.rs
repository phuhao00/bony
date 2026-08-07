//! Connection-control commands, fanned out to local subscribers in-process.
//!
//! A moderation action (a ban) must reach every live socket for the affected
//! pubkey. This module carries connection-control intents — today only
//! "disconnect this pubkey" — to every local consumer of
//! [`crate::PubSubManager::subscribe_conn_control`] via a `tokio::sync::broadcast`
//! channel, applied against [`crate::ConnectionManager`] on the same process.
//!
//! This is deliberately a **separate** channel from `cache_invalidation`: a
//! cache-key drop is a pure, idempotent hint (the DB is re-read on the next
//! access), whereas a disconnect is an imperative, non-idempotent action on a
//! live socket. Folding it into the cache-invalidation enum would break that
//! module's stated invariant ("a pure cache-key drop, never an evict payload").
//! The DB ban row remains the durable backstop: even if a disconnect message is
//! dropped, the next auth attempt is refused at the auth seam.

use buzz_core::CommunityId;

/// A connection-control command to apply locally.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConnControl {
    /// Disconnect every live socket bound to the carrying community.
    DisconnectCommunity,
    /// Disconnect every live connection authenticated as `pubkey` in the
    /// carrying community — live ban enforcement. `pubkey` is 32 raw bytes.
    /// `event_id` and `reason` reproduce the same NIP-01 `OK` frame the origin
    /// sent, so a member disconnected via this path learns why.
    DisconnectPubkey {
        /// Banned member's pubkey bytes.
        pubkey: Vec<u8>,
        /// Id echoed in the closing `OK` frame (the ban event's id on origin).
        event_id: String,
        /// Human-readable close reason for the `OK` frame.
        reason: String,
    },
}

/// A connection-control command scoped to the community it applies to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScopedConnControl {
    /// Community whose connections the command applies to.
    pub community_id: CommunityId,
    /// The tenant-local connection-control command.
    pub command: ConnControl,
}
