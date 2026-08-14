//! Persist a home-anchored live agent roster for the coordinator MCP
//! (`buzz-dev-mcp` `route_list` / `route_pick`).
//!
//! Single writer: Desktop. Path mirrors buzz-dev-mcp defaults:
//! `<home>/.bony-build/room-memory/live-roster.json`
//! (override with `BONY_ROOM_ROSTER_PATH` or `BONY_ROOM_MEMORY_DIR`).

use chrono::Utc;
use serde::Serialize;
use std::fs;
use std::path::PathBuf;

use super::types::ManagedAgentSummary;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct RosterAgentWire {
    name: String,
    pubkey: String,
    capabilities: Vec<String>,
    status: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct LiveRosterWire {
    updated_at: String,
    agents: Vec<RosterAgentWire>,
}

/// Rewrite the live roster from current managed-agent summaries.
/// Best-effort: failures are logged via the returned error for callers to
/// surface or ignore — never block agent start on roster I/O.
pub fn write_live_roster(summaries: &[ManagedAgentSummary]) -> Result<PathBuf, String> {
    let path = live_roster_path();
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|e| format!("create roster dir: {e}"))?;
    }
    let agents: Vec<RosterAgentWire> = summaries
        .iter()
        .filter(|summary| !summary.capabilities.is_empty())
        .map(|summary| RosterAgentWire {
            name: summary.name.clone(),
            pubkey: summary.pubkey.clone(),
            capabilities: summary.capabilities.clone(),
            status: summary.status.clone(),
        })
        .collect();
    let roster = LiveRosterWire {
        updated_at: Utc::now().to_rfc3339(),
        agents,
    };
    let raw =
        serde_json::to_string_pretty(&roster).map_err(|e| format!("serialize roster: {e}"))?;
    fs::write(&path, raw).map_err(|e| format!("write roster {}: {e}", path.display()))?;
    Ok(path)
}

pub fn live_roster_path() -> PathBuf {
    if let Ok(p) = std::env::var("BONY_ROOM_ROSTER_PATH") {
        if !p.trim().is_empty() {
            return PathBuf::from(p);
        }
    }
    room_memory_dir().join("live-roster.json")
}

pub(crate) fn room_memory_dir() -> PathBuf {
    if let Ok(p) = std::env::var("BONY_ROOM_MEMORY_DIR") {
        if !p.trim().is_empty() {
            return PathBuf::from(p);
        }
    }
    if let Some(home) = std::env::var_os("USERPROFILE").or_else(|| std::env::var_os("HOME")) {
        return PathBuf::from(home).join(".bony-build").join("room-memory");
    }
    PathBuf::from(".bony-build").join("room-memory")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;
    use std::sync::Mutex;

    static ENV_LOCK: Mutex<()> = Mutex::new(());

    fn summary(name: &str, caps: &[&str], status: &str) -> ManagedAgentSummary {
        ManagedAgentSummary {
            pubkey: format!("pk-{name}"),
            name: name.into(),
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
            capabilities: caps.iter().map(|c| (*c).to_string()).collect(),
            persona_out_of_date: false,
            persona_orphaned: false,
            needs_restart: false,
            restart_diff: Vec::new(),
            env_vars: BTreeMap::new(),
            backend: crate::managed_agents::types::BackendKind::Local,
            backend_agent_id: None,
            status: status.into(),
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

    #[test]
    fn write_live_roster_skips_empty_capabilities() {
        let _guard = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let dir = std::env::temp_dir().join(format!("bony-live-roster-{}", std::process::id()));
        let path = dir.join("live-roster.json");
        std::env::set_var("BONY_ROOM_ROSTER_PATH", &path);
        let agents = vec![
            summary("Grok", &["coordination.route"], "running"),
            summary("Anon", &[], "running"),
        ];
        let result = write_live_roster(&agents);
        std::env::remove_var("BONY_ROOM_ROSTER_PATH");
        result.expect("write");
        let raw = fs::read_to_string(&path).expect("read");
        assert!(raw.contains("Grok"));
        assert!(!raw.contains("Anon"));
        let _ = fs::remove_dir_all(&dir);
    }
}
