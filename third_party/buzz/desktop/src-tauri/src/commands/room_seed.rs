//! Native, idempotent seeding of the five local "room" agents (Grok,
//! ZeroClaw, Unity, OpenMontage, DocSmith) and their shared "Local Room"
//! channel.
//!
//! Replaces the old external-script chain
//! (`mint-agent-keys.ps1` → `register-room-agents.ps1` →
//! 5× `start-<name>-agent.ps1`) with a single Rust command that goes
//! straight through the same [`create_managed_agent`] / [`create_channel`] /
//! [`add_channel_members`] paths a user clicking through the UI would use —
//! no hand-written `managed-agents.json`, no externally-launched `buzz-acp`
//! processes, no keyring writes past the OS credential-store size cap.
//!
//! Idempotent by name/channel-name: a record or channel that already exists
//! is left untouched. Safe to call on every Desktop launch — see the
//! post-community-init hook in `App.tsx`.

use std::collections::{BTreeMap, HashSet};

use serde::Serialize;
use tauri::{AppHandle, State};

use crate::app_state::AppState;
use crate::managed_agents::{load_managed_agents, BackendKind, CreateManagedAgentRequest, RespondTo};

use super::{add_channel_members, create_channel, create_managed_agent, get_channels};

const ROOM_CHANNEL_NAME: &str = "Local Room";
const ROOM_CHANNEL_DESCRIPTION: &str = "Local room stack agents";

const GROK_PROMPT: &str =
    include_str!("../../../../../../scripts/buzz-room/prompts/grok-coordinator.md");
const ZEROCLAW_PROMPT: &str =
    include_str!("../../../../../../scripts/buzz-room/prompts/zeroclaw-specialist.md");
const UNITY_PROMPT: &str =
    include_str!("../../../../../../scripts/buzz-room/prompts/unity-specialist.md");
const OPENMONTAGE_PROMPT: &str =
    include_str!("../../../../../../scripts/buzz-room/prompts/openmontage-specialist.md");
const DOCSMITH_PROMPT: &str =
    include_str!("../../../../../../scripts/buzz-room/prompts/docsmith-specialist.md");

/// One fixed room-agent seat. `name` is the idempotency key: a managed-agent
/// record whose name already matches (case-insensitively) is left alone, so
/// calling `seed_room_agents` on every launch never mints duplicates.
struct RoomAgentSpec {
    name: &'static str,
    about: &'static str,
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

/// Working-directory root ZeroClaw's OpenMontage counterpart reads from.
fn openmontage_root() -> Option<String> {
    dirs::home_dir().map(|home| {
        home.join(".bony-build")
            .join("openmontage")
            .to_string_lossy()
            .into_owned()
    })
}

fn room_agent_specs() -> [RoomAgentSpec; 5] {
    const SPECIALIST_ENV: &[(&str, &str)] = &[
        ("BUZZ_ACP_SUBSCRIBE", "mentions"),
        ("BUZZ_ACP_PERMISSION_MODE", "accept-edits"),
        ("BUZZ_ACP_AUTO_POST_REPLY", "true"),
        ("BUZZ_ACP_PROGRESS_POST", "true"),
        ("BUZZ_ACP_SUPPRESS_META_REPLIES", "true"),
    ];

    [
        RoomAgentSpec {
            name: "Grok",
            about: "Room coordinator",
            agent_command: Some("grok"),
            agent_args: &["agent", "stdio"],
            mcp_command: "buzz-dev-mcp",
            system_prompt: GROK_PROMPT,
            extra_env: &[
                ("BUZZ_ACP_SUBSCRIBE", "all"),
                // Bare subscribe=all with empty kinds is a wildcard (reactions,
                // presence, control noise all become turns). Restrict to the
                // stream message kinds Mentions mode uses by default.
                ("BUZZ_ACP_KINDS", "9,46010,40007"),
                ("BUZZ_ACP_PERMISSION_MODE", "accept-edits"),
                ("BUZZ_ACP_AUTO_POST_REPLY", "true"),
                ("BUZZ_ACP_PROGRESS_POST", "true"),
                ("BUZZ_ACP_SUPPRESS_META_REPLIES", "true"),
                ("BUZZ_ACP_MULTIPLE_EVENT_HANDLING", "queue"),
                ("BUZZ_ACP_CONTEXT_MESSAGE_LIMIT", "6"),
                ("BUZZ_ACP_NO_MEMORY", "true"),
            ],
        },
        RoomAgentSpec {
            name: "ZeroClaw",
            about: "ZeroClaw specialist",
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
                // DocSmith (and every other room agent) has no access to that
                // attachment store, so the file is a dead end every time —
                // deny both tools at the ACP layer.
                ("BUZZ_ACP_DENY_TOOLS", "deliver_file,file_write"),
            ],
        },
        RoomAgentSpec {
            name: "Unity",
            about: "Unity specialist",
            agent_command: Some("grok"),
            agent_args: &["agent", "stdio"],
            mcp_command: "bony-room-tools-mcp",
            system_prompt: UNITY_PROMPT,
            extra_env: SPECIALIST_ENV,
        },
        RoomAgentSpec {
            name: "OpenMontage",
            about: "OpenMontage specialist",
            agent_command: Some("grok"),
            agent_args: &["agent", "stdio"],
            mcp_command: "bony-room-tools-mcp",
            system_prompt: OPENMONTAGE_PROMPT,
            extra_env: SPECIALIST_ENV,
        },
        RoomAgentSpec {
            name: "DocSmith",
            about: "Docs specialist (PDF/Word/Excel/PPT)",
            agent_command: Some("grok"),
            agent_args: &["agent", "stdio"],
            mcp_command: "bony-docs-tools-mcp",
            system_prompt: DOCSMITH_PROMPT,
            extra_env: SPECIALIST_ENV,
        },
    ]
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
/// agent can't block the other four from coming up.
#[tauri::command]
pub async fn seed_room_agents(
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<SeedRoomAgentsResult, String> {
    let mut errors = Vec::new();
    let mut created_agents = Vec::new();

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
        if spec.name == "OpenMontage" {
            if let Some(root) = openmontage_root() {
                env_vars.insert("OPENMONTAGE_ROOT".to_string(), root);
            }
        }

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
            system_prompt: Some(format!(
                "{}\n\n(Local room agent: {} — {})",
                spec.system_prompt, spec.name, spec.about
            )),
            avatar_url: None,
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
    // so messages elsewhere never reached Grok/ZeroClaw — look like "no feedback".
    let room_names: HashSet<String> = room_agent_specs()
        .iter()
        .map(|spec| spec.name.to_lowercase())
        .collect();
    let room_agent_pubkeys: Vec<String> = load_managed_agents(&app)
        .unwrap_or_default()
        .into_iter()
        .filter(|record| {
            !record.pubkey.trim().is_empty()
                && room_names.contains(&record.name.to_lowercase())
        })
        .map(|record| record.pubkey)
        .collect();

    // Channel names (case-insensitive) that must host the room stack bots.
    const SEED_CHANNEL_NAMES: &[&str] = &[
        ROOM_CHANNEL_NAME,
        "welcome-everyone",
        "general",
    ];
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
            .map(|c| {
                c.member_pubkeys
                    .iter()
                    .map(|p| p.to_lowercase())
                    .collect()
            })
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

    Ok(SeedRoomAgentsResult {
        channel_id,
        created_channel,
        created_agents,
        errors,
    })
}
