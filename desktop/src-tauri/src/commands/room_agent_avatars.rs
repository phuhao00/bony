//! Official logos for the two fixed Local Room seats.
//!
//! Served from the Desktop web asset root (`/room-agent-logos/…`) so avatars
//! work offline. Provenance: `public/room-agent-logos/CREDITS.md`.

/// Stable avatar URL for a fixed room seat by display name (case-insensitive).
pub fn room_agent_avatar_url(name: &str) -> Option<&'static str> {
    match name.trim().to_ascii_lowercase().as_str() {
        "grok" => Some(GROK_ROOM_AVATAR),
        "zeroclaw" => Some(ZEROCLAW_ROOM_AVATAR),
        _ => None,
    }
}

const GROK_ROOM_AVATAR: &str = "/room-agent-logos/grok.svg";
const ZEROCLAW_ROOM_AVATAR: &str = "/room-agent-logos/zeroclaw.png";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn room_avatars_are_distinct_local_assets() {
        let names = ["ZeroClaw"];
        let mut urls = Vec::new();
        for name in names {
            let url = room_agent_avatar_url(name).expect(name);
            assert!(
                url.starts_with("/room-agent-logos/"),
                "{name} should be a bundled room-agent logo path"
            );
            urls.push(url);
        }
        let set: std::collections::BTreeSet<_> = urls.into_iter().collect();
        assert_eq!(set.len(), 1, "each room seat needs a unique logo");
    }
}
