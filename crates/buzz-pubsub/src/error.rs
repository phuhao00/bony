use thiserror::Error;

/// Errors that can occur in pub/sub, presence, and rate-limiting operations.
#[derive(Debug, Error)]
pub enum PubSubError {
    /// The target broadcast channel has no active local subscribers.
    /// Harmless in practice — the relay keeps at least one internal
    /// subscriber alive for the process lifetime on every channel — but
    /// surfaced so callers can log it instead of silently dropping the event.
    #[error("no local subscribers for this event")]
    NoSubscribers,
}
