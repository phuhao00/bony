//! Native, idempotent seeding of the local room agent (ZeroClaw)
//! and the shared "Local Room" channel.
//!
//! Replaces the old external-script chain
//! (`mint-agent-keys.ps1` → `register-room-agents.ps1` →
//! `start-<name>-agent.ps1`) with a single Rust command that goes
//! straight through the same [`create_managed_agent`] / [`create_channel`] /
//! [`add_channel_members`] paths a user clicking through the UI would use —
//! no hand-written `managed-agents.json`, no externally-launched `buzz-acp`
//! processes, no keyring writes past the OS credential-store size cap.
//!
//! Idempotent by name/channel-name: existing records are reused.
//! Legacy Grok / Unity / OpenMontage / DocSmith seats are archived, stopped, and
//! purged on reconcile. Safe to call on every Desktop launch — see the
//! post-community-init hook in `App.tsx`.

use std::collections::{BTreeMap, HashSet};

use serde::Serialize;
use tauri::{AppHandle, State};

use crate::app_state::AppState;
use crate::managed_agents::{
    load_managed_agents, save_managed_agents, BackendKind, CreateManagedAgentRequest,
    ManagedAgentRecord, RespondTo, MANAGED_AGENT_CAPABILITIES_ENV,
};
use crate::util::now_iso;

use super::{
    add_channel_members, create_channel, create_managed_agent, get_channels, remove_channel_member,
};
use super::room_agent_avatars;

const ROOM_CHANNEL_NAME: &str = "Local Room";
const ROOM_CHANNEL_DESCRIPTION: &str = "Local room stack agents";

const ZEROCLAW_PROMPT: &str =
    include_str!("../../prompts/zeroclaw-specialist.md");

/// One fixed room-agent seat. `name` is the idempotency key: a managed-agent
/// record whose name already matches (case-insensitively) is reused, so
/// calling `seed_room_agents` on every launch never mints duplicates.
struct RoomAgentSpec {
    name: &'static str,
    about: &'static str,
    capabilities: &'static [&'static str],
    /// Bare command resolved the same way any managed-agent harness is
    /// resolved (`resolve_command` — PATH + npm `.cmd` shims on Windows).
    /// `None` means "resolve dynamically" (only ZeroClaw, whose install path
    /// lives under the user profile rather than on PATH).
    agent_command: Option<&'static str>,
    agent_args: &'static [&'static str],
    /// Catalog MCP binary name, resolved the same way at spawn time. Empty =
    /// no MCP server (ZeroClaw ships its own tools).
    mcp_command: &'static str,
    system_prompt: &'static str,
    extra_env: &'static [(&'static str, &'static str)],
}

/// Absolute path to the ZeroClaw binary under the user's home directory.
/// Falls back to a bare `"zeroclaw"` (PATH lookup) if the home dir can't be
/// resolved — matches the previous `start-zeroclaw-agent.ps1` default.
fn zeroclaw_command() -> String {
    dirs::home_dir()
        .map(|home| {
            home.join(".bony-build")
                .join("zeroclaw")
                .join("target")
                .join("release")
                .join("zeroclaw.exe")
                .to_string_lossy()
                .into_owned()
        })
        .unwrap_or_else(|| "zeroclaw".to_string())
}

fn is_stripped_room_seat(name: &str) -> bool {
    matches!(
        name.trim().to_ascii_lowercase().as_str(),
        "grok" | "unity" | "openmontage" | "docsmith"
    )
}

fn room_agent_specs() -> [RoomAgentSpec; 1] {
    [
        RoomAgentSpec {
            name: "ZeroClaw",
            about: "ZeroClaw specialist",
            capabilities: &["research.web"],
            agent_command: None,
            agent_args: &["acp"],
            mcp_command: "",
            system_prompt: ZEROCLAW_PROMPT,
            extra_env: &[
                ("BUZZ_ACP_SUBSCRIBE", "mentions"),
                ("BUZZ_ACP_PERMISSION_MODE", "accept-edits"),
                ("BUZZ_ACP_AUTO_POST_REPLY", "true"),
                ("BUZZ_ACP_PROGRESS_POST", "true"),
                ("BUZZ_ACP_SUPPRESS_META_REPLIES", "true"),
                ("BUZZ_ACP_MULTIPLE_EVENT_HANDLING", "queue"),
                ("BUZZ_ACP_CONTEXT_MESSAGE_LIMIT", "4"),
                ("BUZZ_ACP_NO_MEMORY", "true"),
                // zeroclaw.exe ships native file_write/deliver_file tools it
                // will reach for on its own to "deliver" a long answer as an
                // attachment, even though the prompt says paste it inline.
                // deny both tools at the ACP layer.
                ("BUZZ_ACP_DENY_TOOLS", "deliver_file,file_write"),
            ],
        },
    ]
}

fn room_agent_system_prompt(spec: &RoomAgentSpec) -> String {
    format!(
        "{}\n\n(Local room agent: {} - {})",
        spec.system_prompt, spec.name, spec.about
    )
}

fn archive_stripped_room_seats(records: &mut [ManagedAgentRecord]) -> Vec<String> {
    let mut pubkeys = Vec::new();
    for record in records.iter_mut() {
        if record.pubkey.trim().is_empty() || !is_stripped_room_seat(&record.name) {
            continue;
        }
        pubkeys.push(record.pubkey.clone());
        record.is_active = false;
        record.start_on_app_launch = false;
        record.env_vars.remove(MANAGED_AGENT_CAPABILITIES_ENV);
        record.updated_at = now_iso();
    }
    pubkeys
}

fn purge_stripped_room_seats(records: &mut Vec<ManagedAgentRecord>) -> bool {
    let before = records.len();
    records.retain(|record| {
        record.pubkey.trim().is_empty() || !is_stripped_room_seat(&record.name)
    });
    records.len() != before
}

/// Reconcile live room seats (avatars, env, MCP) and archive stripped seats
/// so restore/spawn will not bring Grok / Unity / OpenMontage / DocSmith back.
fn reconcile_room_agent_contracts(
    app: &AppHandle,
    state: &AppState,
) -> Result<Vec<String>, String> {
    let _store_guard = state
        .managed_agents_store_lock
        .lock()
        .map_err(|error| error.to_string())?;
    let mut records = load_managed_agents(app)?;
    let mut changed = false;
    for record in &mut records {
        // Room seats are keyed by display name + live pubkey. Older seeds stamped
        // persona_id = pubkey, so avatar/env reconcile must not require
        // persona_id.is_none().
        let room_spec = room_agent_specs()
            .into_iter()
            .find(|spec| !record.pubkey.trim().is_empty() && record.name.eq_ignore_ascii_case(spec.name));
        if let Some(spec) = room_spec {
            if let Some(desired_avatar) = room_agent_avatars::room_agent_avatar_url(spec.name) {
                if record.avatar_url.as_deref() != Some(desired_avatar) {
                    record.avatar_url = Some(desired_avatar.to_string());
                    record.updated_at = now_iso();
                    changed = true;
                }
            }

            let mut desired_env: BTreeMap<String, String> = spec
                .extra_env
                .iter()
                .map(|(key, value)| ((*key).to_string(), (*value).to_string()))
                .collect();
            desired_env.insert(
                MANAGED_AGENT_CAPABILITIES_ENV.to_string(),
                spec.capabilities.join(","),
            );
            let mut env_changed = false;
            for (key, value) in desired_env {
                if record.env_vars.get(&key).map(String::as_str) != Some(value.as_str()) {
                    record.env_vars.insert(key, value);
                    env_changed = true;
                }
            }
            if env_changed {
                record.updated_at = now_iso();
                changed = true;
            }

            let desired_mcp = if spec.mcp_command.is_empty() {
                String::new()
            } else {
                spec.mcp_command.to_string()
            };
            if record.mcp_command != desired_mcp {
                record.mcp_command = desired_mcp;
                record.updated_at = now_iso();
                changed = true;
            }
        }
    }

    let stripped_pubkeys = archive_stripped_room_seats(&mut records);
    if !stripped_pubkeys.is_empty() {
        changed = true;
    }

    if changed {
        save_managed_agents(app, &records)?;
    }
    Ok(stripped_pubkeys)
}

fn purge_stripped_room_seat_store(app: &AppHandle, state: &AppState) -> Result<(), String> {
    let _store_guard = state
        .managed_agents_store_lock
        .lock()
        .map_err(|error| error.to_string())?;
    let mut records = load_managed_agents(app)?;
    if purge_stripped_room_seats(&mut records) {
        save_managed_agents(app, &records)?;
    }
    Ok(())
}

#[derive(Debug, Serialize)]
pub struct SeedRoomAgentsResult {
    pub channel_id: String,
    pub created_channel: bool,
    pub created_agents: Vec<String>,
    pub errors: Vec<String>,
}

/// Create any missing room-agent seats, ensure the shared "Local Room"
/// channel exists (reusing it by name rather than minting a duplicate), and
/// make sure every seat is a bot member. Errors on individual seats/steps are
/// collected rather than aborting the whole seed, so a single misconfigured
/// agent can't block the other seats from coming up.
#[tauri::command]
pub async fn seed_room_agents(
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<SeedRoomAgentsResult, String> {
    let mut errors = Vec::new();
    let mut created_agents = Vec::new();

    let stripped_pubkeys = match reconcile_room_agent_contracts(&app, &state) {
        Ok(pubkeys) => pubkeys,
        Err(error) => {
            errors.push(format!("reconcile room agent contracts: {error}"));
            Vec::new()
        }
    };
    for pubkey in &stripped_pubkeys {
        if let Err(error) = super::stop_managed_agent(pubkey.clone(), app.clone()).await {
            errors.push(format!("stop stripped seat {pubkey}: {error}"));
        }
    }
    if !stripped_pubkeys.is_empty() {
        if let Err(error) = purge_stripped_room_seat_store(&app, &state) {
            errors.push(format!("purge stripped room seats: {error}"));
        }
    }

    let existing_names: HashSet<String> = load_managed_agents(&app)
        .unwrap_or_default()
        .iter()
        // Only treat keyed (instantiated) seats as already seeded. Empty-key
        // projections (persona catalog / builtins named Grok etc.) must NOT
        // block minting a real local room agent.
        .filter(|record| !record.pubkey.trim().is_empty())
        .map(|record| record.name.to_lowercase())
        .collect();

    for spec in room_agent_specs() {
        if existing_names.contains(&spec.name.to_lowercase()) {
            continue;
        }

        let mut env_vars: BTreeMap<String, String> = spec
            .extra_env
            .iter()
            .map(|(key, value)| (key.to_string(), value.to_string()))
            .collect();
        env_vars.insert(
            MANAGED_AGENT_CAPABILITIES_ENV.to_string(),
            spec.capabilities.join(","),
        );

        let agent_command = match spec.agent_command {
            Some(command) => command.to_string(),
            None => zeroclaw_command(),
        };

        let request = CreateManagedAgentRequest {
            name: spec.name.to_string(),
            persona_id: None,
            team_id: None,
            relay_url: None,
            acp_command: None,
            agent_command: Some(agent_command),
            // Pin the harness so room seats never inherit the default
            // buzz-agent runtime catalog (which would trip NotReady on
            // missing provider/model and force setup-listener mode).
            harness_override: true,
            agent_args: spec.agent_args.iter().map(|arg| arg.to_string()).collect(),
            mcp_command: (!spec.mcp_command.is_empty()).then(|| spec.mcp_command.to_string()),
            turn_timeout_seconds: None,
            idle_timeout_seconds: None,
            max_turn_duration_seconds: None,
            // Room seats: one ACP child per agent. Default desktop parallelism
            // (10) forks ~10 grok/zeroclaw per seat and stalls Waking / relay.
            parallelism: Some(1),
            system_prompt: Some(room_agent_system_prompt(&spec)),
            avatar_url: room_agent_avatars::room_agent_avatar_url(spec.name)
                .map(str::to_string),
            model: None,
            provider: None,
            env_vars,
            spawn_after_create: true,
            start_on_app_launch: true,
            backend: BackendKind::Local,
            respond_to: Some(RespondTo::Anyone),
            respond_to_allowlist: Vec::new(),
            relay_mesh: None,
        };

        match create_managed_agent(request, app.clone(), state.clone()).await {
            Ok(_) => created_agents.push(spec.name.to_string()),
            Err(error) => errors.push(format!("{}: {error}", spec.name)),
        }
    }

    // ── Channel: reuse an existing "Local Room" by name; only mint one when
    // truly none exists. Archived twins are skipped so a retired duplicate
    // from the old script-based flow is never resurrected as canonical.
    let channels = get_channels(state.clone()).await.unwrap_or_default();
    let existing_channel = channels
        .iter()
        .find(|channel| channel.name == ROOM_CHANNEL_NAME && channel.archived_at.is_none());

    let (channel_id, created_channel, known_members): (String, bool, HashSet<String>) =
        match existing_channel {
            Some(channel) => (
                channel.id.clone(),
                false,
                channel
                    .member_pubkeys
                    .iter()
                    .map(|pubkey| pubkey.to_lowercase())
                    .collect(),
            ),
            None => match create_channel(
                ROOM_CHANNEL_NAME.to_string(),
                "stream".to_string(),
                "open".to_string(),
                Some(ROOM_CHANNEL_DESCRIPTION.to_string()),
                None,
                state.clone(),
            )
            .await
            {
                Ok(channel) => (channel.id, true, HashSet::new()),
                Err(error) => {
                    errors.push(format!("create_channel: {error}"));
                    return Ok(SeedRoomAgentsResult {
                        channel_id: String::new(),
                        created_channel: false,
                        created_agents,
                        errors,
                    });
                }
            },
        };

    // ── Membership: every fixed room-agent seat must be a bot member ───────
    // Also seed bot seats into the starter open channels humans actually type in
    // (welcome-everyone / general). Previously agents only joined "Local Room",
    // so messages elsewhere never reached ZeroClaw — look like "no feedback".
    let room_names: HashSet<String> = room_agent_specs()
        .iter()
        .map(|spec| spec.name.to_lowercase())
        .collect();
    let room_agent_pubkeys: Vec<String> = load_managed_agents(&app)
        .unwrap_or_default()
        .into_iter()
        .filter(|record| {
            !record.pubkey.trim().is_empty() && room_names.contains(&record.name.to_lowercase())
        })
        .map(|record| record.pubkey)
        .collect();

    // Channel names (case-insensitive) that must host the room stack bots.
    const SEED_CHANNEL_NAMES: &[&str] = &[ROOM_CHANNEL_NAME, "welcome-everyone", "general"];
    let seed_channel_ids: Vec<String> = {
        let mut ids = Vec::new();
        ids.push(channel_id.clone());
        for channel in &channels {
            if channel.archived_at.is_some() {
                continue;
            }
            let name = channel.name.to_lowercase();
            if SEED_CHANNEL_NAMES
                .iter()
                .any(|n| n.eq_ignore_ascii_case(&channel.name) || name == n.to_lowercase())
                && !ids.iter().any(|id| id == &channel.id)
            {
                ids.push(channel.id.clone());
            }
        }
        ids
    };

    for seed_channel_id in &seed_channel_ids {
        let known: HashSet<String> = channels
            .iter()
            .find(|c| &c.id == seed_channel_id)
            .map(|c| c.member_pubkeys.iter().map(|p| p.to_lowercase()).collect())
            .unwrap_or_else(|| {
                if seed_channel_id == &channel_id {
                    known_members.clone()
                } else {
                    HashSet::new()
                }
            });

        let missing: Vec<String> = room_agent_pubkeys
            .iter()
            .filter(|pk| !known.contains(&pk.to_lowercase()))
            .cloned()
            .collect();
        if missing.is_empty() {
            continue;
        }
        if let Err(error) = add_channel_members(
            seed_channel_id.clone(),
            missing,
            Some("bot".to_string()),
            state.clone(),
        )
        .await
        {
            errors.push(format!("add_channel_members({seed_channel_id}): {error}"));
        }
    }

    for seed_channel_id in &seed_channel_ids {
        for pubkey in &stripped_pubkeys {
            if let Err(error) =
                remove_channel_member(seed_channel_id.clone(), pubkey.clone(), state.clone()).await
            {
                errors.push(format!(
                    "remove_channel_member({seed_channel_id}, {pubkey}): {error}"
                ));
            }
        }
    }

    // Ensure the local human operator is an owner member of Local Room too
    // (cleans up after junk purges that accidentally stripped every non-bot seat).
    let owner_hex = match state.keys.lock() {
        Ok(keys) => keys.public_key().to_hex().to_lowercase(),
        Err(error) => {
            errors.push(format!("owner_pubkey: {error}"));
            String::new()
        }
    };
    if !owner_hex.is_empty() && !known_members.contains(&owner_hex) {
        if let Err(error) = add_channel_members(
            channel_id.clone(),
            vec![owner_hex],
            Some("owner".to_string()),
            state.clone(),
        )
        .await
        {
            errors.push(format!("add_owner_member: {error}"));
        }
    }

    // Refresh live-roster.json for route_list / route_pick (best-effort).
    if let Err(error) = crate::commands::list_managed_agents(app.clone()).await {
        errors.push(format!("refresh live roster: {error}"));
    }

    Ok(SeedRoomAgentsResult {
        channel_id,
        created_channel,
        created_agents,
        errors,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn room_seats_declare_zeroclaw_research_only() {
        let specs = room_agent_specs();
        let zeroclaw = specs.iter().find(|spec| spec.name == "ZeroClaw").unwrap();
        assert_eq!(zeroclaw.capabilities, &["research.web"]);
        assert!(!zeroclaw
            .capabilities
            .iter()
            .any(|capability| capability.starts_with("code.")));
    }

    #[test]
    fn stripped_specialists_are_not_seeded() {
        let specs = room_agent_specs();
        assert_eq!(specs.len(), 1);
        assert!(specs.iter().all(|spec| !is_stripped_room_seat(spec.name)));
        assert!(is_stripped_room_seat("Grok"));
        assert!(specs.iter().any(|spec| spec.name == "ZeroClaw"));
        assert!(!specs.iter().any(|spec| spec.name == "Grok"));
    }

    #[test]
    fn room_agent_avatar_urls_cover_all_seats() {
        for spec in room_agent_specs() {
            assert!(
                room_agent_avatars::room_agent_avatar_url(spec.name).is_some(),
                "missing logo for {}",
                spec.name
            );
        }
    }
}
