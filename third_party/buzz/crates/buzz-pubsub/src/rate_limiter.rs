//! In-process fixed-window rate limiter.
//!
//! Implements the [`RateLimiter`] trait from `buzz-auth`. Backed by a
//! `DashMap<String, (count, window_ends_at)>` — single-instance equivalent of
//! the previous Redis `INCR` + `EXPIRE` script: a window resets lazily the
//! first time it is observed to have expired.
//!
//! ⚠️ Fixed windows allow up to 2× burst at boundaries. Upgrade to sliding
//! window or token bucket for strict limiting.

use std::net::IpAddr;
use std::time::{Duration, Instant};

use buzz_auth::{
    error::AuthError,
    rate_limit::{LimitType, RateLimitResult, RateLimiter},
};
use buzz_core::TenantContext;
use dashmap::DashMap;
use nostr::PublicKey;

struct Window {
    count: u64,
    ends_at: Instant,
}

fn hit(windows: &DashMap<String, Window>, key: &str, window_secs: u64, limit: u64) -> RateLimitResult {
    let now = Instant::now();

    let mut entry = windows
        .entry(key.to_string())
        .or_insert_with(|| Window {
            count: 0,
            // Starts already-expired so the first hit below always opens a
            // fresh window, even for `window_secs == 0`.
            ends_at: now,
        });

    if now >= entry.ends_at {
        entry.count = 0;
        entry.ends_at = now + Duration::from_secs(window_secs);
    }

    entry.count += 1;
    let count = entry.count;
    let reset_in_secs = entry.ends_at.saturating_duration_since(now).as_secs();

    if count <= limit {
        RateLimitResult::allowed(count, limit, reset_in_secs)
    } else {
        RateLimitResult::denied(count, limit, reset_in_secs)
    }
}

/// In-process rate limiter using fixed-window counters.
///
/// Pubkey keys are community-scoped via `&TenantContext`. IP keys remain
/// operator-global. Equivalent isolation to the prior Redis-backed limiter,
/// minus cross-process sharing — irrelevant for a single-instance relay.
#[derive(Default)]
pub struct InMemoryRateLimiter {
    windows: DashMap<String, Window>,
}

impl InMemoryRateLimiter {
    /// Creates a new, empty in-process rate limiter.
    pub fn new() -> Self {
        Self::default()
    }
}

impl RateLimiter for InMemoryRateLimiter {
    async fn check_and_increment(
        &self,
        ctx: &TenantContext,
        pubkey: &PublicKey,
        limit_type: LimitType,
        window_secs: u64,
        limit: u64,
    ) -> Result<RateLimitResult, AuthError> {
        let key = buzz_auth::rate_limit::rate_limit_key(ctx, pubkey, &limit_type);
        Ok(hit(&self.windows, &key, window_secs, limit))
    }

    async fn check_ip_connection(
        &self,
        ip: &IpAddr,
        window_secs: u64,
        limit: u64,
    ) -> Result<RateLimitResult, AuthError> {
        let key = buzz_auth::rate_limit::ip_rate_limit_key(ip);
        Ok(hit(&self.windows, &key, window_secs, limit))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use buzz_core::CommunityId;
    use nostr::Keys;
    use uuid::Uuid;

    fn ctx(id: u128, host: &str) -> TenantContext {
        TenantContext::resolved(CommunityId::from_uuid(Uuid::from_u128(id)), host)
    }

    #[tokio::test]
    async fn allows_up_to_limit_then_denies() {
        let limiter = InMemoryRateLimiter::new();
        let ctx = ctx(0xaaaa, "a.example");
        let pubkey = Keys::generate().public_key();

        for _ in 0..3 {
            let result = limiter
                .check_and_increment(&ctx, &pubkey, LimitType::Messages, 60, 3)
                .await
                .unwrap();
            assert!(result.allowed);
        }

        let denied = limiter
            .check_and_increment(&ctx, &pubkey, LimitType::Messages, 60, 3)
            .await
            .unwrap();
        assert!(!denied.allowed);
    }

    #[tokio::test]
    async fn window_resets_after_expiry() {
        let limiter = InMemoryRateLimiter::new();
        let ctx = ctx(0xaaaa, "a.example");
        let pubkey = Keys::generate().public_key();

        let first = limiter
            .check_and_increment(&ctx, &pubkey, LimitType::Messages, 0, 1)
            .await
            .unwrap();
        assert!(first.allowed);

        tokio::time::sleep(Duration::from_millis(10)).await;

        let second = limiter
            .check_and_increment(&ctx, &pubkey, LimitType::Messages, 0, 1)
            .await
            .unwrap();
        assert!(second.allowed, "window should have reset after expiry");
    }

    #[tokio::test]
    async fn communities_are_isolated() {
        let limiter = InMemoryRateLimiter::new();
        let ctx_a = ctx(0xaaaa, "a.example");
        let ctx_b = ctx(0xbbbb, "b.example");
        let pubkey = Keys::generate().public_key();

        let a = limiter
            .check_and_increment(&ctx_a, &pubkey, LimitType::Messages, 60, 1)
            .await
            .unwrap();
        let b = limiter
            .check_and_increment(&ctx_b, &pubkey, LimitType::Messages, 60, 1)
            .await
            .unwrap();
        assert!(a.allowed);
        assert!(b.allowed);
    }
}
