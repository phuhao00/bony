//! Live room roster + capability route pick for the coordinator MCP surface.
//!
//! Desktop writes `live-roster.json` next to the task-log; Grok reads it here
//! so routing can use declared capabilities instead of hard-coded display names.
//! Does not grant permissions — only ranks eligible seats.

use crate::shell::SharedState;
use rmcp::ErrorData;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};

const MAX_CAPABILITIES: usize = 32;
const MAX_CAPABILITY_LEN: usize = 64;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct RosterAgent {
    pub name: String,
    #[serde(default)]
    pub pubkey: String,
    #[serde(default)]
    pub capabilities: Vec<String>,
    #[serde(default)]
    pub status: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct LiveRoster {
    #[serde(default)]
    pub updated_at: String,
    #[serde(default)]
    pub agents: Vec<RosterAgent>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct RouteListParams {
    /// Capability id or namespace prefix (`research.web`, `code.`). Empty = list all declared.
    #[serde(default)]
    pub capability: Option<String>,
    /// When true (default), only `status=running` agents are returned.
    #[serde(default)]
    pub require_running: Option<bool>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct RoutePickParams {
    /// Required capability id or namespace prefix the assignee must declare.
    pub capability: String,
    /// User-explicit pin (agent display name). Wins when that agent is eligible.
    #[serde(default)]
    pub preferred_name: Option<String>,
    /// Soft preference names from memory (most preferred first). Only reorders
    /// eligible candidates — never bypasses capability / running checks.
    #[serde(default)]
    pub preference_names: Vec<String>,
    #[serde(default)]
    pub require_running: Option<bool>,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct RoutePick {
    pub name: String,
    pub pubkey: String,
    pub capabilities: Vec<String>,
    pub status: String,
    pub reason: String,
}

pub fn list(state: &SharedState, p: RouteListParams) -> Result<String, ErrorData> {
    let roster = load_roster(state)?;
    let require_running = p.require_running.unwrap_or(true);
    let capability = p.capability.as_deref().unwrap_or("").trim();
    let mut agents: Vec<&RosterAgent> = roster
        .agents
        .iter()
        .filter(|agent| !agent.capabilities.is_empty())
        .filter(|agent| !require_running || agent.status.eq_ignore_ascii_case("running"))
        .filter(|agent| {
            capability.is_empty() || capability_matches(&agent.capabilities, capability)
        })
        .collect();
    agents.sort_by(|left, right| {
        left.pubkey
            .to_ascii_lowercase()
            .cmp(&right.pubkey.to_ascii_lowercase())
            .then_with(|| {
                left.name
                    .to_ascii_lowercase()
                    .cmp(&right.name.to_ascii_lowercase())
            })
    });
    if agents.is_empty() {
        return Ok(format!(
            "no eligible agents{}{}",
            if capability.is_empty() {
                String::new()
            } else {
                format!(" for capability \"{capability}\"")
            },
            if require_running {
                " (running only)"
            } else {
                ""
            }
        ));
    }
    let mut out = format!("{} eligible agent(s):\n", agents.len());
    for agent in agents {
        out.push_str(&format!(
            "- {} | caps: {} | status: {} | pubkey: {}\n",
            agent.name,
            agent.capabilities.join(","),
            agent.status,
            if agent.pubkey.is_empty() {
                "-"
            } else {
                &agent.pubkey
            }
        ));
    }
    Ok(out)
}

pub fn pick(state: &SharedState, p: RoutePickParams) -> Result<String, ErrorData> {
    let capability = p.capability.trim();
    if capability.is_empty() {
        return Err(ErrorData::invalid_params(
            "capability must not be empty",
            None,
        ));
    }
    let roster = load_roster(state)?;
    let require_running = p.require_running.unwrap_or(true);
    match pick_route_agent(
        &roster.agents,
        capability,
        p.preferred_name.as_deref(),
        &p.preference_names,
        require_running,
    ) {
        Some(pick) => Ok(format!(
            "pick: @{} | reason: {} | caps: {} | status: {} | pubkey: {}",
            pick.name,
            pick.reason,
            pick.capabilities.join(","),
            pick.status,
            if pick.pubkey.is_empty() {
                "-"
            } else {
                &pick.pubkey
            }
        )),
        None => Ok(format!(
            "no eligible agent for capability \"{capability}\" — ask the user to choose, do not guess"
        )),
    }
}

/// Deterministic pick: explicit pin → preference-ranked eligible → pubkey tie-break.
pub fn pick_route_agent(
    agents: &[RosterAgent],
    capability: &str,
    preferred_name: Option<&str>,
    preference_names: &[String],
    require_running: bool,
) -> Option<RoutePick> {
    let eligible: Vec<&RosterAgent> = agents
        .iter()
        .filter(|agent| !agent.capabilities.is_empty())
        .filter(|agent| capability_matches(&agent.capabilities, capability))
        .filter(|agent| !require_running || agent.status.eq_ignore_ascii_case("running"))
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
                name: agent.name.clone(),
                pubkey: agent.pubkey.clone(),
                capabilities: agent.capabilities.clone(),
                status: agent.status.clone(),
                reason: "explicit pin".into(),
            });
        }
    }

    let mut ranked: Vec<(&RosterAgent, i32, bool)> = eligible
        .into_iter()
        .map(|agent| {
            let pref_score = preference_rank_score(agent, preference_names);
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
            .then_with(|| {
                left.0
                    .name
                    .to_ascii_lowercase()
                    .cmp(&right.0.name.to_ascii_lowercase())
            })
    });
    let (agent, pref_score, _) = ranked.into_iter().next()?;
    let reason = if pref_score > 0 {
        format!("capability match + memory preference score {pref_score}")
    } else {
        "capability match + deterministic tie-break".into()
    };
    Some(RoutePick {
        name: agent.name.clone(),
        pubkey: agent.pubkey.clone(),
        capabilities: agent.capabilities.clone(),
        status: agent.status.clone(),
        reason,
    })
}

fn preference_rank_score(agent: &RosterAgent, preference_names: &[String]) -> i32 {
    preference_names
        .iter()
        .enumerate()
        .filter(|(_, name)| agent.name.eq_ignore_ascii_case(name.trim()))
        .map(|(index, _)| (preference_names.len() - index) as i32)
        .sum()
}

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

pub(crate) fn load_roster(state: &SharedState) -> Result<LiveRoster, ErrorData> {
    let path = roster_path(state);
    match fs::read_to_string(&path) {
        Ok(raw) => serde_json::from_str(&raw).map_err(|e| {
            ErrorData::internal_error(format!("parse live roster {}: {e}", path.display()), None)
        }),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(LiveRoster {
            updated_at: String::new(),
            agents: Vec::new(),
        }),
        Err(e) => Err(ErrorData::internal_error(
            format!("open live roster {}: {e}", path.display()),
            None,
        )),
    }
}

/// Same home-anchored dir as task-log; override via `BONY_ROOM_ROSTER_PATH`.
pub fn roster_path(state: &SharedState) -> PathBuf {
    if let Ok(p) = std::env::var("BONY_ROOM_ROSTER_PATH") {
        if !p.trim().is_empty() {
            return PathBuf::from(p);
        }
    }
    room_memory_dir(state).join("live-roster.json")
}

pub fn room_memory_dir(state: &SharedState) -> PathBuf {
    if let Ok(p) = std::env::var("BONY_ROOM_MEMORY_DIR") {
        if !p.trim().is_empty() {
            return PathBuf::from(p);
        }
    }
    if let Some(home) = std::env::var_os("USERPROFILE").or_else(|| std::env::var_os("HOME")) {
        return PathBuf::from(home).join(".bony-build").join("room-memory");
    }
    state.cwd.join(".bony-build").join("room-memory")
}

/// Write helper used by unit tests (and mirrored by Desktop).
pub fn write_roster_file(path: &Path, roster: &LiveRoster) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|e| format!("create roster dir: {e}"))?;
    }
    let raw = serde_json::to_string_pretty(roster).map_err(|e| format!("serialize roster: {e}"))?;
    fs::write(path, raw).map_err(|e| format!("write roster: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    static ENV_LOCK: Mutex<()> = Mutex::new(());

    fn agent(name: &str, pubkey: &str, caps: &[&str], status: &str) -> RosterAgent {
        RosterAgent {
            name: name.into(),
            pubkey: pubkey.into(),
            capabilities: caps.iter().map(|c| (*c).to_string()).collect(),
            status: status.into(),
        }
    }

    #[test]
    fn pick_honors_explicit_pin_when_eligible() {
        let agents = vec![
            agent("ZeroClaw", "aa", &["research.web"], "running"),
            agent("AltResearch", "bb", &["research.web"], "running"),
        ];
        let pick = pick_route_agent(
            &agents,
            "research.web",
            Some("AltResearch"),
            &["ZeroClaw".into()],
            true,
        )
        .expect("pick");
        assert_eq!(pick.name, "AltResearch");
        assert_eq!(pick.reason, "explicit pin");
    }

    #[test]
    fn pick_uses_memory_preferences_then_pubkey() {
        let agents = vec![
            agent("ZeroClaw", "ff", &["research.web"], "running"),
            agent("Scout", "aa", &["research.web"], "running"),
        ];
        let pick = pick_route_agent(
            &agents,
            "research.web",
            None,
            &["Scout".into(), "ZeroClaw".into()],
            true,
        )
        .expect("pick");
        assert_eq!(pick.name, "Scout");
        assert!(pick.reason.contains("memory preference"));
    }

    #[test]
    fn pick_rejects_pin_outside_capability() {
        let agents = vec![
            agent("DocSmith", "aa", &["document.create"], "running"),
            agent("ZeroClaw", "bb", &["research.web"], "running"),
        ];
        let pick = pick_route_agent(&agents, "research.web", Some("DocSmith"), &[], true)
            .expect("fallback");
        assert_eq!(pick.name, "ZeroClaw");
        assert_ne!(pick.reason, "explicit pin");
    }

    #[test]
    fn roster_roundtrip_file() {
        let _guard = ENV_LOCK.lock().unwrap();
        let dir = std::env::temp_dir().join(format!("bony-roster-{}", std::process::id()));
        let path = dir.join("live-roster.json");
        let roster = LiveRoster {
            updated_at: "2026-08-12T00:00:00Z".into(),
            agents: vec![agent("Grok", "cc", &["coordination.route"], "running")],
        };
        write_roster_file(&path, &roster).expect("write");
        let loaded: LiveRoster = serde_json::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(loaded.agents[0].name, "Grok");
        let _ = fs::remove_dir_all(&dir);
    }
}
