//! After a tender is awarded, run the winning agent on the task and settle.
//!
//! Market rules (no keyword / named-agent hard routes):
//! 1. Winner attempts the job in Local Room (tools).
//! 2. If that fails, winner **pays** to hire the next agent (reputation + stake
//!    + capability diversity) and retries with prior materials.
//! 3. Up to two hires. LLM is only a last-resort fallback when room paths fail.

use std::time::Duration;

use nostr::Event;
use tauri::{AppHandle, Manager, State};

use crate::{
    app_state::AppState,
    commands::{
        channels::get_channels,
        messages::{send_channel_message, send_managed_agent_channel_message},
        personas::resolve_env_from_layers,
    },
    managed_agents::{
        find_managed_agent_mut, load_global_agent_config, load_managed_agents, load_personas,
        record_capabilities, TenderSnapshot,
    },
    relay::{effective_agent_relay_url, query_relay, relay_ws_url_with_override},
};

const ROOM_CHANNEL_NAME: &str = "Local Room";
const ROOM_POLL_ATTEMPTS: u32 = 12;
const ROOM_POLL_ATTEMPTS_TOOLS: u32 = 24;
const ROOM_POLL_INTERVAL_MS: u64 = 1_500;
const LLM_TIMEOUT_SECS: u64 = 45;
const DEFAULT_LLM_MODEL: &str = "grok-3-mini";
const MAX_SUPPORT_HIRES: u32 = 2;

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
    let title = tender.title.trim().to_string();
    if title.is_empty() {
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

    let mut worker_pk = winner_pubkey.clone();
    let mut worker_name = winner_name.clone();
    let mut materials: Vec<String> = Vec::new();
    let hire_budget = (tender.budget.max(1) / 2).max(1);

    for hire_round in 0..=MAX_SUPPORT_HIRES {
        let prompt = compose_work_prompt(&title, &materials);
        if let Some(text) =
            run_room_fulfill(app, state, &worker_pk, &worker_name, &prompt).await
        {
            // ACP turn-progress lines are not deliverables — keep polling path
            // from treating them as answers (pick_agent_answer already skips).
            if is_room_progress_message(&text) {
                tracing::debug!("tender fulfill: ignoring progress status as answer");
            } else if answer_looks_usable(&text) {
                let _ = post_room_result(
                    app,
                    state,
                    &winner_pubkey,
                    &winner_name,
                    &tender.tender_id,
                    &title,
                    &text,
                )
                .await;
                return persist_outcome(tender, text, true).await;
            } else {
                materials.push(text);
            }
        }

        if hire_round == MAX_SUPPORT_HIRES {
            break;
        }

        let hire = {
            let app = app.clone();
            let payer = winner_pubkey.clone();
            let payer_name = winner_name.clone();
            let task_ref = tender.task_ref.clone();
            let max_pay = hire_budget;
            match tokio::task::spawn_blocking(move || {
                let agents = load_roster(&app)?;
                crate::managed_agents::economy::hire_support(
                    &payer,
                    Some(&payer_name),
                    "open",
                    &task_ref,
                    max_pay,
                    &agents,
                )
            })
            .await
            {
                Ok(Ok(hire)) => hire,
                Ok(Err(error)) => {
                    tracing::warn!(%error, "tender fulfill: open hire failed");
                    break;
                }
                Err(error) => {
                    tracing::warn!(%error, "tender fulfill: open hire join failed");
                    break;
                }
            }
        };

        tracing::info!(
            hiree = %hire.hiree_name,
            paid = hire.paid,
            score = hire.score,
            round = hire_round + 1,
            "tender fulfill: winner hired open-market support"
        );
        worker_pk = hire.hiree_pubkey.to_ascii_lowercase();
        worker_name = hire.hiree_name;
    }

    // Last resort: winner LLM (no tool invent preferred, but better than silence).
    match complete_with_winner_llm(app, state, &winner_pubkey, &winner_name, &title).await {
        Ok(text) if answer_looks_usable(&text) => {
            let _ = post_room_result(
                app,
                state,
                &winner_pubkey,
                &winner_name,
                &tender.tender_id,
                &title,
                &text,
            )
            .await;
            persist_outcome(tender, text, true).await
        }
        Ok(text) => {
            let note = if text.trim().is_empty() {
                "履约失败：房间工具与后备路径均未产出可用结果".to_string()
            } else {
                text
            };
            persist_outcome(tender, note, false).await
        }
        Err(_) => {
            persist_outcome(
                tender,
                "履约失败：房间工具与后备路径均未产出可用结果".to_string(),
                false,
            )
            .await
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

fn compose_work_prompt(title: &str, materials: &[String]) -> String {
    if materials.is_empty() {
        return format!(
            "请完成任务「{title}」。只输出最终可用结果，不要寒暄。\
             若你缺少必要能力，请明确写出缺少什么。"
        );
    }
    let mut buf = format!("请继续完成任务「{title}」。只输出最终可用结果，不要寒暄。\n");
    for (i, body) in materials.iter().enumerate() {
        buf.push_str(&format!("\n—— 先前材料 {} ——\n{body}\n", i + 1));
    }
    buf
}

fn answer_looks_usable(text: &str) -> bool {
    let t = text.trim();
    if t.is_empty() {
        return false;
    }
    if is_room_progress_message(t) {
        return false;
    }
    let lower = t.to_ascii_lowercase();
    const BAD: &[&str] = &[
        "error:",
        "failed",
        "failure",
        "i can't",
        "i cannot",
        "无法完成",
        "做不到",
        "缺少",
        "没有联网",
        "no internet",
        "cannot complete",
        "失败",
        "出错",
        "履约失败",
    ];
    !BAD.iter().any(|n| {
        if n.is_ascii() {
            lower.contains(n)
        } else {
            t.contains(n)
        }
    })
}

/// ACP coding-progress posts look like `📝 **处理文档** · 失败 · …` — not answers.
fn is_room_progress_message(text: &str) -> bool {
    let t = text.trim();
    if t.is_empty() {
        return false;
    }
    if t.contains("等待最终回复") || t.contains("仍在运行") {
        return true;
    }
    if t.contains(" · 失败 · ") {
        return true;
    }
    if t.contains(" · 第 ") && t.contains(" 步") {
        return true;
    }
    const LABELS: &[&str] = &[
        "**处理文档**",
        "**检索中**",
        "**查询天气**",
        "**读写文件**",
        "**执行命令**",
        "**编码中**",
        "**发送消息**",
        "**处理中**",
    ];
    LABELS.iter().any(|label| t.contains(label))
}

fn load_roster(app: &AppHandle) -> Result<Vec<buzz_economy::RosterAgent>, String> {
    let state = app.state::<AppState>();
    let _store = state
        .managed_agents_store_lock
        .lock()
        .map_err(|e| e.to_string())?;
    let records = load_managed_agents(app)?;
    let known: Vec<(String, String, Vec<String>)> = records
        .iter()
        .map(|record| {
            (
                record.pubkey.clone(),
                record.name.clone(),
                record_capabilities(record),
            )
        })
        .collect();
    Ok(crate::managed_agents::economy::roster_from_known(&known))
}

async fn persist_outcome(tender: TenderSnapshot, answer: String, success: bool) -> TenderSnapshot {
    let tender_id = tender.tender_id.clone();
    // Never mark progress / error-like text as a successful settlement.
    let success = if success {
        answer_looks_usable(&answer)
    } else {
        false
    };
    match tokio::task::spawn_blocking(move || {
        crate::managed_agents::economy::record_tender_outcome(&tender_id, &answer, success)
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

async fn run_room_fulfill(
    app: &AppHandle,
    state: &State<'_, AppState>,
    pubkey: &str,
    name: &str,
    task: &str,
) -> Option<String> {
    let _ = ensure_winner_runtime(app, state, pubkey).await;
    let prompt_created_at = chrono::Utc::now().timestamp().saturating_sub(2);
    let channel_id = post_room_task(state, pubkey, name, task).await.ok()?;
    poll_room_answer(
        state,
        &channel_id,
        pubkey,
        prompt_created_at,
        ROOM_POLL_ATTEMPTS_TOOLS,
    )
    .await
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
    // Marker text is hard-gated in buzz-acp: only the p-tagged winner turns.
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

/// Publish a settlement notice as the winner (no work request).
///
/// Posted under the agent key so peer/subscribe=all bots treat it as agent
/// traffic; content marker `【招标结果】` is also hard-skipped in buzz-acp.
async fn post_room_result(
    app: &AppHandle,
    state: &State<'_, AppState>,
    winner_pubkey: &str,
    winner_name: &str,
    tender_id: &str,
    title: &str,
    answer: &str,
) -> Result<(), String> {
    let channels = get_channels(state.clone()).await?;
    let channel = channels
        .iter()
        .find(|channel| channel.name == ROOM_CHANNEL_NAME && channel.archived_at.is_none())
        .ok_or_else(|| "Local Room channel not found".to_string())?;
    let clipped: String = answer.chars().take(1200).collect();
    let content = format!("【招标结果】任务：{title}\n答案：{clipped}\n— @{winner_name}");
    let marker = format!("tender-result:{tender_id}");
    let _ = send_managed_agent_channel_message(
        winner_pubkey.to_string(),
        channel.id.clone(),
        content,
        Some(marker),
        Some("agent".into()),
        None,
        None,
        None,
        app.clone(),
        state.clone(),
    )
    .await?;
    Ok(())
}

async fn poll_room_answer(
    state: &State<'_, AppState>,
    channel_id: &str,
    winner_pubkey: &str,
    since: i64,
    attempts: u32,
) -> Option<String> {
    let attempts = attempts.max(ROOM_POLL_ATTEMPTS);
    for _ in 0..attempts {
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
        if content.contains("【招标任务】") || content.contains("【招标结果】") {
            continue;
        }
        if is_room_progress_message(content) {
            continue;
        }
        if newest
            .map(|cur| event.created_at > cur.created_at)
            .unwrap_or(true)
        {
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
