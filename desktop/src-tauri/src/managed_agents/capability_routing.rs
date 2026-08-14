//! Capability-aware route eligibility for managed agents.
//!
//! Single authoritative place for:
//! - parsing stable capability ids from the seeder env declaration;
//! - filtering a live roster by capability / readiness / automatic eligibility.
//!
//! Does **not** grant permissions — capability ids only describe what an agent
//! declares. Callers still apply ACP allow/deny and room policy.

use super::types::{ManagedAgentRecord, ManagedAgentSummary, MANAGED_AGENT_CAPABILITIES_ENV};

/// Max capability ids accepted from a single declaration (matches summary projection).
const MAX_CAPABILITIES: usize = 32;
const MAX_CAPABILITY_LEN: usize = 64;

/// Parse comma-separated capability ids with the same validation the summary
/// projection uses. Invalid tokens are dropped, not rejected.
pub fn parse_capability_ids(raw: &str) -> Vec<String> {
    raw.split(',')
        .map(str::trim)
        .filter(|value| {
            !value.is_empty()
                && value.len() <= MAX_CAPABILITY_LEN
                && value
                    .chars()
                    .any(|character| character.is_ascii_alphanumeric())
                && value.chars().all(|character| {
                    character.is_ascii_alphanumeric() || matches!(character, '.' | '_' | '-')
                })
        })
        .take(MAX_CAPABILITIES)
        .map(str::to_string)
        .collect()
}

/// Capabilities declared on a managed-agent record (env key written by room seeder
/// or user). Empty means "no declared capability" — still mentionable, not
/// auto-routable by capability match.
pub fn record_capabilities(record: &ManagedAgentRecord) -> Vec<String> {
    record
        .env_vars
        .get(MANAGED_AGENT_CAPABILITIES_ENV)
        .map(|value| parse_capability_ids(value))
        .unwrap_or_default()
}

/// True when `capabilities` contains an exact id or a matching namespace prefix
/// query (`code.` matches `code.repo.read`).
pub fn capability_matches(capabilities: &[String], query: &str) -> bool {
    let query = query.trim();
    if query.is_empty() {
        return false;
    }
    if query.ends_with('.') {
        return capabilities
            .iter()
            .any(|capability| capability.starts_with(query));
    }
    capabilities
        .iter()
        .any(|capability| capability == query || capability.starts_with(&format!("{query}.")))
}

/// One route-eligible row for coordinator / UI consumption.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RouteEligibleAgent {
    pub pubkey: String,
    pub name: String,
    pub capabilities: Vec<String>,
    pub status: String,
    pub automatic: bool,
}

/// Result of [`pick_route_agent`] — single assignee + why.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RoutePick {
    pub pubkey: String,
    pub name: String,
    pub capabilities: Vec<String>,
    pub status: String,
    pub reason: String,
}

/// Filter + deterministically order agents that declare `capability_query`.
///
/// Rules (aligned with `docs/buzz-room-agent-orchestration-plan.md` §3 / D2):
/// - must declare a matching capability (no name/prompt inference);
/// - when `require_running` is true, `status` must be `running`;
/// - agents with empty capability lists are never auto-selected;
/// - tie-break by lowercase pubkey for stable ordering.
pub fn select_route_eligible_agents(
    agents: &[ManagedAgentSummary],
    capability_query: &str,
    require_running: bool,
) -> Vec<RouteEligibleAgent> {
    let mut selected: Vec<RouteEligibleAgent> = agents
        .iter()
        .filter(|agent| !agent.capabilities.is_empty())
        .filter(|agent| capability_matches(&agent.capabilities, capability_query))
        .filter(|agent| !require_running || agent.status == "running")
        .map(|agent| RouteEligibleAgent {
            pubkey: agent.pubkey.clone(),
            name: agent.name.clone(),
            capabilities: agent.capabilities.clone(),
            status: agent.status.clone(),
            // Declared capability + running (when required) ⇒ eligible for
            // automatic capability routing. Explicit mention always remains
            // available regardless of this flag.
            automatic: true,
        })
        .collect();
    selected.sort_by(|left, right| {
        left.pubkey
            .to_ascii_lowercase()
            .cmp(&right.pubkey.to_ascii_lowercase())
    });
    selected
}

/// Deterministic single-agent pick (D3 + soft D4 preference ranking).
///
/// Order: explicit `preferred_name` pin (if eligible) → preference_names soft
/// score → exact capability match before prefix → lowercase pubkey tie-break.
/// Preference names never bypass capability / running filters.
pub fn pick_route_agent(
    agents: &[ManagedAgentSummary],
    capability: &str,
    preferred_name: Option<&str>,
    preference_names: &[String],
    require_running: bool,
) -> Option<RoutePick> {
    let eligible: Vec<&ManagedAgentSummary> = agents
        .iter()
        .filter(|agent| !agent.capabilities.is_empty())
        .filter(|agent| capability_matches(&agent.capabilities, capability))
        .filter(|agent| !require_running || agent.status == "running")
        .collect();
    if eligible.is_empty() {
        return None;
    }

    if let Some(pin) = preferred_name
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        if let Some(agent) = eligible
            .iter()
            .find(|agent| agent.name.eq_ignore_ascii_case(pin))
        {
            return Some(RoutePick {
                pubkey: agent.pubkey.clone(),
                name: agent.name.clone(),
                capabilities: agent.capabilities.clone(),
                status: agent.status.clone(),
                reason: "explicit pin".into(),
            });
        }
    }

    let mut ranked: Vec<(&ManagedAgentSummary, i32, bool)> = eligible
        .into_iter()
        .map(|agent| {
            let pref_score = preference_names
                .iter()
                .enumerate()
                .filter(|(_, name)| agent.name.eq_ignore_ascii_case(name.trim()))
                .map(|(index, _)| (preference_names.len() - index) as i32)
                .sum();
            let exact = agent.capabilities.iter().any(|cap| cap == capability);
            (agent, pref_score, exact)
        })
        .collect();
    ranked.sort_by(|left, right| {
        right
            .1
            .cmp(&left.1)
            .then_with(|| right.2.cmp(&left.2))
            .then_with(|| {
                left.0
                    .pubkey
                    .to_ascii_lowercase()
                    .cmp(&right.0.pubkey.to_ascii_lowercase())
            })
    });
    let (agent, pref_score, _) = ranked.into_iter().next()?;
    let reason = if pref_score > 0 {
        format!("capability match + memory preference score {pref_score}")
    } else {
        "capability match + deterministic tie-break".into()
    };
    Some(RoutePick {
        pubkey: agent.pubkey.clone(),
        name: agent.name.clone(),
        capabilities: agent.capabilities.clone(),
        status: agent.status.clone(),
        reason,
    })
}

/// Overlay local managed-agent capabilities onto relay directory rows that
/// still publish an empty `capabilities` array (pre-capability kind:10100).
pub fn overlay_local_capabilities(
    relay_agents: &mut [super::types::RelayAgentInfo],
    local_records: &[ManagedAgentRecord],
) {
    for agent in relay_agents.iter_mut() {
        if !agent.capabilities.is_empty() {
            continue;
        }
        let Some(record) = local_records
            .iter()
            .find(|record| record.pubkey.eq_ignore_ascii_case(&agent.pubkey))
        else {
            continue;
        };
        let caps = record_capabilities(record);
        if !caps.is_empty() {
            agent.capabilities = caps;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    fn summary(pubkey: &str, name: &str, caps: &[&str], status: &str) -> ManagedAgentSummary {
        ManagedAgentSummary {
            pubkey: pubkey.to_string(),
            name: name.to_string(),
            persona_id: None,
            runtime: None,
            team_id: None,
            relay_url: String::new(),
            acp_command: String::new(),
            agent_command: String::new(),
            agent_command_override: None,
            agent_args: Vec::new(),
            mcp_command: String::new(),
            turn_timeout_seconds: 0,
            idle_timeout_seconds: None,
            max_turn_duration_seconds: None,
            parallelism: 1,
            system_prompt: None,
            avatar_url: None,
            model: None,
            model_source: None,
            provider: None,
            capabilities: caps.iter().map(|value| (*value).to_string()).collect(),
            persona_out_of_date: false,
            persona_orphaned: false,
            needs_restart: false,
            restart_diff: Vec::new(),
            env_vars: BTreeMap::new(),
            backend: crate::managed_agents::types::BackendKind::Local,
            backend_agent_id: None,
            status: status.to_string(),
            pid: None,
            created_at: String::new(),
            updated_at: String::new(),
            last_started_at: None,
            last_stopped_at: None,
            last_exit_code: None,
            last_error: None,
            last_error_code: None,
            start_on_app_launch: false,
            auto_restart_on_config_change: true,
            log_path: String::new(),
            respond_to: crate::managed_agents::types::RespondTo::default(),
            respond_to_allowlist: Vec::new(),
        }
    }

    fn record_with_caps(pubkey: &str, caps: &str) -> ManagedAgentRecord {
        let mut record: ManagedAgentRecord = serde_json::from_str(
            r#"{
                "pubkey": "placeholder",
                "name": "test",
                "private_key_nsec": "nsec1fake",
                "relay_url": "",
                "acp_command": "buzz-acp",
                "agent_command": "buzz-agent",
                "agent_args": [],
                "mcp_command": "",
                "turn_timeout_seconds": 320,
                "system_prompt": null,
                "created_at": "2026-01-01T00:00:00Z",
                "updated_at": "2026-01-01T00:00:00Z",
                "last_started_at": null,
                "last_stopped_at": null,
                "last_exit_code": null,
                "last_error": null
            }"#,
        )
        .expect("record fixture");
        record.pubkey = pubkey.to_string();
        record
            .env_vars
            .insert(MANAGED_AGENT_CAPABILITIES_ENV.to_string(), caps.to_string());
        record
    }

    #[test]
    fn parse_capability_ids_drops_invalid_tokens() {
        assert_eq!(
            parse_capability_ids("code.repo.read, bad id!, research.web,,.."),
            vec!["code.repo.read".to_string(), "research.web".to_string()]
        );
    }

    #[test]
    fn capability_matches_exact_and_prefix() {
        let caps = vec![
            "code.repo.read".to_string(),
            "code.rust.change".to_string(),
            "research.web".to_string(),
        ];
        assert!(capability_matches(&caps, "code.repo.read"));
        assert!(capability_matches(&caps, "code."));
        assert!(capability_matches(&caps, "research"));
        assert!(!capability_matches(&caps, "unity."));
        assert!(!capability_matches(&caps, ""));
    }

    #[test]
    fn select_route_eligible_agents_filters_and_sorts() {
        let agents = vec![
            summary("ff", "B", &["code.repo.read"], "running"),
            summary("aa", "A", &["code.rust.change"], "running"),
            summary("bb", "Idle", &["code.repo.read"], "stopped"),
            summary("cc", "None", &[], "running"),
            summary("dd", "Research", &["research.web"], "running"),
        ];
        let selected = select_route_eligible_agents(&agents, "code.", true);
        assert_eq!(
            selected
                .iter()
                .map(|agent| agent.pubkey.as_str())
                .collect::<Vec<_>>(),
            vec!["aa", "ff"]
        );
        assert!(selected.iter().all(|agent| agent.automatic));
    }

    #[test]
    fn pick_route_agent_honors_pin_and_preferences() {
        let agents = vec![
            summary("ff", "ZeroClaw", &["research.web"], "running"),
            summary("aa", "Scout", &["research.web"], "running"),
        ];
        let pinned = pick_route_agent(
            &agents,
            "research.web",
            Some("Scout"),
            &["ZeroClaw".into()],
            true,
        )
        .expect("pin");
        assert_eq!(pinned.name, "Scout");
        assert_eq!(pinned.reason, "explicit pin");

        let preferred = pick_route_agent(
            &agents,
            "research.web",
            None,
            &["Scout".into(), "ZeroClaw".into()],
            true,
        )
        .expect("pref");
        assert_eq!(preferred.name, "Scout");
        assert!(preferred.reason.contains("memory preference"));
    }

    #[test]
    fn overlay_fills_empty_relay_capabilities_from_local_record() {
        let mut relay = vec![crate::managed_agents::types::RelayAgentInfo {
            pubkey: "ABCD".to_string(),
            name: "Grok".to_string(),
            agent_type: "agent".to_string(),
            channels: Vec::new(),
            channel_ids: Vec::new(),
            capabilities: Vec::new(),
            status: "online".to_string(),
            respond_to: None,
            respond_to_allowlist: Vec::new(),
        }];
        let record = record_with_caps("abcd", "coordination.route,code.repo.read");
        overlay_local_capabilities(&mut relay, std::slice::from_ref(&record));
        assert_eq!(
            relay[0].capabilities,
            vec![
                "coordination.route".to_string(),
                "code.repo.read".to_string()
            ]
        );
    }
}
