//! Tenant-scoped event routing topics.
//!
//! Topics are a routing/performance boundary, not an authorization boundary.
//! Tenant identity still comes from [`TenantContext`] on publish/retain
//! paths, and the relay re-checks access before local fan-out.

use buzz_core::{CommunityId, TenantContext};
use uuid::Uuid;

/// A tenant-local event routing scope.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum EventTopic {
    /// Events for one exact channel id.
    Channel(Uuid),
    /// Community-global events that are not exact-channel routed.
    Global,
}

/// A fully qualified event topic, including its server-resolved community.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct EventTopicKey {
    /// Server-resolved community id.
    pub community_id: CommunityId,
    /// Tenant-local routing scope.
    pub topic: EventTopic,
}

impl EventTopicKey {
    /// Build a topic key from a resolved tenant context.
    pub fn from_context(ctx: &TenantContext, topic: EventTopic) -> Self {
        Self {
            community_id: ctx.community(),
            topic,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ctx(id: u128, host: &str) -> TenantContext {
        TenantContext::resolved(CommunityId::from_uuid(Uuid::from_u128(id)), host)
    }

    #[test]
    fn same_channel_in_two_communities_has_different_keys() {
        let community_a = ctx(0xaaaa, "a.example");
        let community_b = ctx(0xbbbb, "b.example");
        let channel_id = Uuid::from_u128(0xcccc);

        assert_ne!(
            EventTopicKey::from_context(&community_a, EventTopic::Channel(channel_id)),
            EventTopicKey::from_context(&community_b, EventTopic::Channel(channel_id)),
        );
    }

    #[test]
    fn channel_and_global_topics_are_distinct_within_one_community() {
        let community = ctx(0xaaaa, "a.example");
        let channel_id = Uuid::from_u128(0xbbbb);

        assert_ne!(
            EventTopicKey::from_context(&community, EventTopic::Channel(channel_id)),
            EventTopicKey::from_context(&community, EventTopic::Global),
        );
    }
}
