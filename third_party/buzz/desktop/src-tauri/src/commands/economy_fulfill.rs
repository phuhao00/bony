//! After a tender is awarded, run the winning agent on the task title and
//! persist the deliverable text (`outcome`) plus economy settlement.
//!
//! Prefer a live Local Room `@winner` reply when one arrives quickly; otherwise
//! fall back to an OpenAI-compatible completion using the winner's API key so
//! simple Q&A tenders (e.g. `1+1=?`) still surface an answer in the market UI.

use std::time::Duration;

use nostr::Event;
use tauri::{AppHandle, State};

use crate::{
    app_state::AppState,
    commands::{
        channels::get_channels,
        messages::send_channel_message,
        personas::resolve_env_from_layers,
    },
    managed_agents::{
        find_managed_agent_mut, load_global_agent_config, load_managed_agents, load_personas,
        TenderSnapshot,
    },
    relay::{effective_agent_relay_url, query_relay, relay_ws_url_with_override},
};

const ROOM_CHANNEL_NAME: &str = "Local Room";
const ROOM_POLL_ATTEMPTS: u32 = 12;
const ROOM_POLL_INTERVAL_MS: u64 = 1_500;
const LLM_TIMEOUT_SECS: u64 = 45;
const DEFAULT_LLM_MODEL: &str = "grok-3-mini";

pub async fn fulfill_awarded_tender(
    app: &AppHandle,
    state: &State<'_, AppState>,
    tender: TenderSnapshot,
) -> TenderSnapshot {
    if tender
        .outcome
        .as_deref()
        .map(str::trim)
        .is_some_and(|s| !s.is_empty())
    {
        return tender;
    }
    if tender.status != "resolved" {
        return tender;
    }
    let Some(winner_pubkey) = tender
        .winner_pubkey
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_ascii_lowercase)
    else {
        return tender;
    };
    let winner_name = tender
        .winner_name
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .unwrap_or("agent")
        .to_string();
    let title = tender.title.trim().to_string();
    if title.is_empty() {
        return tender;
    }

    // Prefer a fast OpenAI-compatible completion with the winner's key so
    // market UI can show the task answer (e.g. `1+1=?` → `2`) without waiting
    // on an idle ACP turn. Fall back to Local Room @mention + poll.
    let answer = match complete_with_winner_llm(app, state, &winner_pubkey, &winner_name, &title)
        .await
    {
        Ok(text) => {
            // Best-effort timeline handoff (do not block on agent reply).
            let _ = ensure_winner_runtime(app, state, &winner_pubkey).await;
            let _ = post_room_task(state, &winner_pubkey, &winner_name, &title).await;
            text
        }
        Err(llm_error) => {
            tracing::warn!(%llm_error, "tender fulfill: LLM path failed; trying room poll");
            let _ = ensure_winner_runtime(app, state, &winner_pubkey).await;
            let prompt_created_at = chrono::Utc::now().timestamp().saturating_sub(2);
            match post_room_task(state, &winner_pubkey, &winner_name, &title).await {
                Ok(channel_id) => {
                    match poll_room_answer(state, &channel_id, &winner_pubkey, prompt_created_at)
                        .await
                    {
                        Some(text) => text,
                        None => return tender,
                    }
                }
                Err(error) => {
                    tracing::warn!(%error, "tender fulfill: Local Room post failed");
                    return tender;
                }
            }
        }
    };

    let tender_id = tender.tender_id.clone();
    match tokio::task::spawn_blocking(move || {
        crate::managed_agents::economy::record_tender_outcome(&tender_id, &answer, true)
    })
    .await
    {
        Ok(Ok(updated)) => updated,
        Ok(Err(error)) => {
            tracing::warn!(%error, "tender fulfill: record_tender_outcome failed");
            tender
        }
        Err(error) => {
            tracing::warn!(%error, "tender fulfill: spawn_blocking failed");
            tender
        }
    }
}

pub async fn fulfill_many(
    app: &AppHandle,
    state: &State<'_, AppState>,
    tenders: Vec<TenderSnapshot>,
) -> Vec<TenderSnapshot> {
    let mut out = Vec::with_capacity(tenders.len());
    for tender in tenders {
        out.push(fulfill_awarded_tender(app, state, tender).await);
    }
    out
}

async fn ensure_winner_runtime(
    app: &AppHandle,
    state: &State<'_, AppState>,
    winner_pubkey: &str,
) -> Result<(), String> {
    let (relay_url, already_running) = {
        let _store = state
            .managed_agents_store_lock
            .lock()
            .map_err(|e| e.to_string())?;
        let mut records = load_managed_agents(app)?;
        let record = find_managed_agent_mut(&mut records, winner_pubkey)?;
        let workspace = relay_ws_url_with_override(state);
        let relay_url = effective_agent_relay_url(&record.relay_url, &workspace);
        let mut runtimes = state
            .managed_agent_processes
            .lock()
            .map_err(|e| e.to_string())?;
        let key = crate::managed_agents::ManagedAgentRuntimeKey::new(
            winner_pubkey.to_string(),
            &relay_url,
        )?;
        let already_running = runtimes.get_mut(&key).is_some_and(|runtime| {
            runtime.child.try_wait().ok().flatten().is_none()
        });
        (relay_url, already_running)
    };
    if already_running {
        return Ok(());
    }
    let pubkey = winner_pubkey.to_string();
    let app = app.clone();
    tokio::task::spawn_blocking(move || {
        crate::managed_agents::start_managed_agent_runtime_pair_lazy(pubkey, relay_url, app)
    })
    .await
    .map_err(|e| format!("start agent join: {e}"))?
    .map(|_| ())?;
    tokio::time::sleep(Duration::from_secs(2)).await;
    Ok(())
}

async fn post_room_task(
    state: &State<'_, AppState>,
    winner_pubkey: &str,
    winner_name: &str,
    title: &str,
) -> Result<String, String> {
    let channels = get_channels(state.clone()).await?;
    let channel = channels
        .iter()
        .find(|channel| channel.name == ROOM_CHANNEL_NAME && channel.archived_at.is_none())
        .ok_or_else(|| "Local Room channel not found".to_string())?;
    let content = format!(
        "@{winner_name} 【招标任务】请直接回答下面的问题，只输出最终答案，不要寒暄：\n{title}"
    );
    let _ = send_channel_message(
        channel.id.clone(),
        content,
        None,
        None,
        None,
        None,
        Some(vec![winner_pubkey.to_string()]),
        None,
        None,
        state.clone(),
    )
    .await?;
    Ok(channel.id.clone())
}

async fn poll_room_answer(
    state: &State<'_, AppState>,
    channel_id: &str,
    winner_pubkey: &str,
    since: i64,
) -> Option<String> {
    for _ in 0..ROOM_POLL_ATTEMPTS {
        tokio::time::sleep(Duration::from_millis(ROOM_POLL_INTERVAL_MS)).await;
        let filter = serde_json::json!({
            "kinds": [buzz_core_pkg::kind::KIND_STREAM_MESSAGE],
            "#h": [channel_id],
            "authors": [winner_pubkey],
            "since": since,
            "limit": 20,
        });
        let events = match query_relay(state, &[filter]).await {
            Ok(events) => events,
            Err(error) => {
                tracing::debug!(%error, "tender fulfill: room poll query failed");
                continue;
            }
        };
        if let Some(text) = pick_agent_answer(&events, winner_pubkey) {
            return Some(text);
        }
    }
    None
}

fn pick_agent_answer(events: &[Event], winner_pubkey: &str) -> Option<String> {
    let mut newest: Option<&Event> = None;
    for event in events {
        if !event.pubkey.to_hex().eq_ignore_ascii_case(winner_pubkey) {
            continue;
        }
        let content = event.content.trim();
        if content.is_empty() {
            continue;
        }
        if content.contains("【招标任务】") {
            continue;
        }
        if newest.map(|cur| event.created_at > cur.created_at).unwrap_or(true) {
            newest = Some(event);
        }
    }
    newest.map(|event| trim_answer(&event.content))
}

fn trim_answer(raw: &str) -> String {
    let trimmed = raw.trim();
    // Drop a leading @mention line the agent may echo.
    let without_mention = trimmed
        .lines()
        .skip_while(|line| {
            let t = line.trim();
            t.starts_with('@') && t.split_whitespace().count() <= 2
        })
        .collect::<Vec<_>>()
        .join("\n");
    let cleaned = without_mention.trim();
    if cleaned.is_empty() {
        trimmed.to_string()
    } else {
        cleaned.to_string()
    }
}

async fn complete_with_winner_llm(
    app: &AppHandle,
    state: &State<'_, AppState>,
    winner_pubkey: &str,
    winner_name: &str,
    title: &str,
) -> Result<String, String> {
    let (api_key, base_url, model) = {
        let _store = state
            .managed_agents_store_lock
            .lock()
            .map_err(|e| e.to_string())?;
        let mut records = load_managed_agents(app)?;
        let record = find_managed_agent_mut(&mut records, winner_pubkey)?;
        let global = load_global_agent_config(app).unwrap_or_default();
        let personas = load_personas(app).unwrap_or_default();
        let persona_env = record
            .persona_id
            .as_deref()
            .and_then(|pid| personas.iter().find(|p| p.id == pid))
            .map(|p| p.env_vars.clone())
            .unwrap_or_default();
        let api_key = resolve_env_from_layers(
            "OPENAI_API_KEY",
            &global.env_vars,
            &persona_env,
            &record.env_vars,
            std::env::var("OPENAI_API_KEY").ok(),
        )
        .ok_or_else(|| "winner agent has no OPENAI_API_KEY".to_string())?;
        let base_url = resolve_env_from_layers(
            "OPENAI_BASE_URL",
            &global.env_vars,
            &persona_env,
            &record.env_vars,
            std::env::var("OPENAI_BASE_URL").ok(),
        );
        let model = resolve_env_from_layers(
            "OPENAI_MODEL",
            &global.env_vars,
            &persona_env,
            &record.env_vars,
            std::env::var("OPENAI_MODEL")
                .ok()
                .or_else(|| std::env::var("BUZZ_AGENT_MODEL").ok())
                .or_else(|| std::env::var("XAI_MODEL").ok()),
        )
        .unwrap_or_else(|| DEFAULT_LLM_MODEL.to_string());
        (api_key, base_url, model)
    };

    let url = chat_completions_url(base_url);
    let body = serde_json::json!({
        "model": model,
        "temperature": 0,
        "messages": [
            {
                "role": "system",
                "content": format!(
                    "You are {winner_name}, fulfilling a room tender. Answer the user task \
                     directly. Prefer a short final answer with no preamble."
                )
            },
            { "role": "user", "content": title }
        ],
    });
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(LLM_TIMEOUT_SECS))
        .build()
        .map_err(|e| format!("http client: {e}"))?;
    let resp = client
        .post(&url)
        .bearer_auth(&api_key)
        .json(&body)
        .send()
        .await
        .map_err(|e| format!("chat completions request failed: {e}"))?;
    let status = resp.status();
    let payload: serde_json::Value = resp
        .json()
        .await
        .map_err(|e| format!("chat completions response was not JSON: {e}"))?;
    if !status.is_success() {
        let detail = payload
            .pointer("/error/message")
            .and_then(|v| v.as_str())
            .unwrap_or("unknown error");
        return Err(format!("chat completions failed (HTTP {status}): {detail}"));
    }
    let content = payload
        .pointer("/choices/0/message/content")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| "chat completions returned empty content".to_string())?;
    Ok(trim_answer(content))
}

fn chat_completions_url(base_url: Option<String>) -> String {
    let base = base_url
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .unwrap_or("https://api.x.ai/v1");
    format!("{}/chat/completions", base.trim_end_matches('/'))
}
