//! Bridge to `zeroclaw acp` — a *custom* lightweight JSON-RPC 2.0/stdio
//! client, deliberately not built on the `agent-client-protocol` crate types
//! used by `agent_bridge.rs`.
//!
//! Why a separate client (Phase 0 spike conclusion): zeroclaw's own ACP docs
//! (`docs/book/src/channels/acp.md` in the zeroclaw repo) explicitly document
//! several deviations from the base ACP spec it otherwise mirrors —
//! `session/cancel`, `session/stop`, `session/load`, `session/resume`,
//! `session/close`, and inbound `session/update` are all called out as
//! "ZeroClaw extension, not part of the base ACP spec"; `session/prompt`'s
//! result shape (`{sessionId, stopReason, content}`) and `initialize`'s
//! nested `agentCapabilities`/`agentInfo`/`_meta.zeroclaw` shape are also
//! zeroclaw-specific. Coercing that dialect through a strict spec-typed crate
//! is fragile; a small hand-written client that mirrors the documented wire
//! shapes exactly is more robust and easier to debug.
//!
//! The output still normalizes into the very same [`AgentEvent`] enum
//! `agent_bridge.rs` uses, so the UI/timeline code needs no awareness of
//! which backend produced an event.

use std::cell::RefCell;
use std::collections::HashMap;
use std::path::PathBuf;
use std::rc::Rc;

use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::sync::{mpsc, oneshot};

use crate::events::{AgentEvent, UiCommand};

#[derive(Clone)]
pub struct ZcBridgeConfig {
    pub bin: PathBuf,
    pub cwd: PathBuf,
    /// `ZEROCLAW_providers__models__<type>__<alias>__api_key` (or `None` if
    /// no credential could be resolved — the spawned config already has the
    /// non-secret `uri`/`model` fields, so a missing key just surfaces as a
    /// normal provider auth error from zeroclaw itself).
    pub provider_env: Option<(String, String)>,
}

/// Start the zeroclaw bridge on its own background thread/executor — mirrors
/// `agent_bridge::spawn_bridge`'s shape so `app.rs` can treat both backends
/// symmetrically.
pub fn spawn_zeroclaw_bridge(
    config: ZcBridgeConfig,
    egui_ctx: egui::Context,
    event_tx: std::sync::mpsc::Sender<AgentEvent>,
) -> mpsc::UnboundedSender<UiCommand> {
    let (cmd_tx, cmd_rx) = mpsc::unbounded_channel::<UiCommand>();
    std::thread::Builder::new()
        .name("zeroclaw-agent-bridge".into())
        .spawn(move || {
            let rt = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .expect("tokio runtime");
            let local = tokio::task::LocalSet::new();
            rt.block_on(local.run_until(async move {
                if let Err(err) = run_bridge_loop(config, egui_ctx, event_tx, cmd_rx).await {
                    tracing::error!(error = %err, "zeroclaw bridge failed");
                }
            }));
        })
        .expect("spawn zeroclaw agent bridge thread");
    cmd_tx
}

async fn run_bridge_loop(
    config: ZcBridgeConfig,
    egui_ctx: egui::Context,
    event_tx: std::sync::mpsc::Sender<AgentEvent>,
    mut cmd_rx: mpsc::UnboundedReceiver<UiCommand>,
) -> anyhow::Result<()> {
    loop {
        match run_session(&config, &egui_ctx, &event_tx, &mut cmd_rx).await {
            SessionEnd::Shutdown => break,
            SessionEnd::Reconnect => {
                let _ = event_tx.send(AgentEvent::Status("ZeroClaw 正在重连…".into()));
                egui_ctx.request_repaint();
                continue;
            }
            SessionEnd::Fatal(err) => {
                let _ = event_tx.send(AgentEvent::Error(format!("ZeroClaw: {err}")));
                egui_ctx.request_repaint();
                // Wait for Shutdown; anything else just gets reported again
                // so the UI can decide to fall back to grok.
                while let Some(cmd) = cmd_rx.recv().await {
                    if matches!(cmd, UiCommand::Shutdown) {
                        return Ok(());
                    }
                }
                return Ok(());
            }
        }
    }
    Ok(())
}

enum SessionEnd {
    Shutdown,
    Reconnect,
    Fatal(String),
}

type PendingMap = Rc<RefCell<HashMap<u64, oneshot::Sender<Result<Value, String>>>>>;

/// Minimal JSON-RPC 2.0/stdio client — request/response correlation by
/// numeric id, fire-and-forget notifications, and a background reader that
/// dispatches responses, forwards `session/update` notifications straight
/// into `AgentEvent`s, and auto-answers `session/request_permission`.
struct RpcClient {
    writer_tx: mpsc::UnboundedSender<String>,
    pending: PendingMap,
    next_id: Rc<RefCell<u64>>,
}

impl RpcClient {
    fn notify(&self, method: &str, params: Value) {
        let line = json!({"jsonrpc": "2.0", "method": method, "params": params}).to_string();
        let _ = self.writer_tx.send(line);
    }

    async fn request(&self, method: &str, params: Value) -> Result<Value, String> {
        let id = {
            let mut guard = self.next_id.borrow_mut();
            *guard += 1;
            *guard
        };
        let (tx, rx) = oneshot::channel();
        self.pending.borrow_mut().insert(id, tx);
        let line =
            json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params}).to_string();
        if self.writer_tx.send(line).is_err() {
            self.pending.borrow_mut().remove(&id);
            return Err("zeroclaw 子进程已退出".into());
        }
        match rx.await {
            Ok(result) => result,
            Err(_) => Err("zeroclaw 未响应（连接已关闭）".into()),
        }
    }
}

async fn run_session(
    config: &ZcBridgeConfig,
    egui_ctx: &egui::Context,
    event_tx: &std::sync::mpsc::Sender<AgentEvent>,
    cmd_rx: &mut mpsc::UnboundedReceiver<UiCommand>,
) -> SessionEnd {
    let emit = |e: AgentEvent| {
        let _ = event_tx.send(e);
        egui_ctx.request_repaint();
    };

    let mut cmd = crate::process::tokio_command(&config.bin);
    cmd.arg("acp")
        .current_dir(&config.cwd)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .kill_on_drop(true);
    if let Some((key, value)) = &config.provider_env {
        cmd.env(key, value);
    }
    let mut child = match cmd.spawn() {
        Ok(c) => c,
        Err(e) => {
            return SessionEnd::Fatal(format!(
                "failed to spawn `{} acp`: {e}",
                config.bin.display()
            ));
        }
    };

    let stdin = match child.stdin.take() {
        Some(s) => s,
        None => return SessionEnd::Fatal("missing stdin".into()),
    };
    let stdout = match child.stdout.take() {
        Some(s) => s,
        None => return SessionEnd::Fatal("missing stdout".into()),
    };
    if let Some(stderr) = child.stderr.take() {
        tokio::task::spawn_local(async move {
            use tokio::io::AsyncReadExt;
            let mut stderr = stderr;
            let mut buf = [0u8; 4096];
            loop {
                match stderr.read(&mut buf).await {
                    Ok(0) | Err(_) => break,
                    Ok(n) => {
                        tracing::info!(
                            target: "zeroclaw-agent",
                            "{}",
                            String::from_utf8_lossy(&buf[..n])
                        );
                    }
                }
            }
        });
    }

    let (writer_tx, mut writer_rx) = mpsc::unbounded_channel::<String>();
    tokio::task::spawn_local(async move {
        let mut stdin = stdin;
        while let Some(line) = writer_rx.recv().await {
            if stdin.write_all(line.as_bytes()).await.is_err() {
                break;
            }
            if stdin.write_all(b"\n").await.is_err() {
                break;
            }
            let _ = stdin.flush().await;
        }
    });

    let pending: PendingMap = Rc::new(RefCell::new(HashMap::new()));
    let next_id = Rc::new(RefCell::new(0u64));
    let client = RpcClient {
        writer_tx: writer_tx.clone(),
        pending: pending.clone(),
        next_id,
    };

    spawn_reader(stdout, pending, writer_tx.clone(), event_tx.clone(), egui_ctx.clone());

    emit(AgentEvent::Status("ZeroClaw 正在初始化…".into()));
    if let Err(e) = client.request("initialize", json!({})).await {
        let _ = child.kill().await;
        return SessionEnd::Fatal(format!("initialize failed: {e}"));
    }

    emit(AgentEvent::Status("ZeroClaw 正在打开会话…".into()));
    // Bind the ACP session to our managed agent alias. ZeroClaw's
    // `is_dispatchable` still requires that alias to have non-empty
    // model_provider / risk_profile / runtime_profile + enabled=true.
    let session_result = client
        .request(
            "session/new",
            json!({
                "cwd": config.cwd.to_string_lossy(),
                "agentAlias": "bonybuild",
            }),
        )
        .await;
    let session_id = match session_result {
        Ok(v) => match v.get("sessionId").and_then(|s| s.as_str()) {
            Some(s) => s.to_string(),
            None => {
                let _ = child.kill().await;
                return SessionEnd::Fatal("session/new: missing sessionId".into());
            }
        },
        Err(e) => {
            let _ = child.kill().await;
            return SessionEnd::Fatal(format!("session/new failed: {e}"));
        }
    };
    emit(AgentEvent::Status("ZeroClaw 就绪".into()));

    while let Some(cmd) = cmd_rx.recv().await {
        match cmd {
            UiCommand::Shutdown => {
                let _ = client
                    .request("session/stop", json!({ "sessionId": session_id }))
                    .await;
                let _ = child.kill().await;
                return SessionEnd::Shutdown;
            }
            UiCommand::ForceStop => {
                let _ = child.kill().await;
                return SessionEnd::Reconnect;
            }
            UiCommand::Cancel => {
                client.notify("session/cancel", json!({ "sessionId": session_id }));
            }
            UiCommand::Prompt { text, attachments: _ } => {
                let text = text.trim().to_string();
                if text.is_empty() {
                    continue;
                }
                emit(AgentEvent::Status("ZeroClaw 正在思考…".into()));
                let prompt_fut = client.request(
                    "session/prompt",
                    json!({ "sessionId": session_id, "prompt": text }),
                );
                tokio::pin!(prompt_fut);
                let turn_result = loop {
                    tokio::select! {
                        biased;
                        cmd = cmd_rx.recv() => {
                            match cmd {
                                None => {
                                    let _ = child.kill().await;
                                    return SessionEnd::Shutdown;
                                }
                                Some(UiCommand::Cancel) => {
                                    emit(AgentEvent::Status("ZeroClaw 正在停止…".into()));
                                    client.notify("session/cancel", json!({ "sessionId": session_id }));
                                }
                                Some(UiCommand::ForceStop) => {
                                    let _ = child.kill().await;
                                    return SessionEnd::Reconnect;
                                }
                                Some(UiCommand::Shutdown) => {
                                    client.notify("session/cancel", json!({ "sessionId": session_id }));
                                    let _ = child.kill().await;
                                    return SessionEnd::Shutdown;
                                }
                                _ => {}
                            }
                        }
                        resp = &mut prompt_fut => break resp,
                    }
                };
                match turn_result {
                    Ok(resp) => {
                        let stop_reason = resp
                            .get("stopReason")
                            .and_then(|s| s.as_str())
                            .unwrap_or("end_turn")
                            .to_string();
                        emit(AgentEvent::TurnDone {
                            stop_reason,
                            usage: crate::usage::TokenUsage::default(),
                        });
                    }
                    Err(e) => emit(AgentEvent::Error(format!("ZeroClaw 回复失败: {e}"))),
                }
            }
            // ZeroClaw sessions don't expose model/mode switching or the
            // interactive login flow grok has; silently ignore rather than
            // erroring the whole bridge.
            UiCommand::SetModel { .. }
            | UiCommand::SetMode { .. }
            | UiCommand::Login
            | UiCommand::PermissionResponse { .. } => {}
        }
    }

    let _ = child.kill().await;
    SessionEnd::Shutdown
}

/// Background reader: dispatches responses to pending requests, answers
/// incoming `session/request_permission` requests automatically (always
/// "allow once", mirroring the always-approve default so a non-coding
/// ZeroClaw turn never silently hangs waiting on UI we don't wire up in v1),
/// and turns `session/update` notifications straight into `AgentEvent`s.
fn spawn_reader(
    stdout: tokio::process::ChildStdout,
    pending: PendingMap,
    writer_tx: mpsc::UnboundedSender<String>,
    event_tx: std::sync::mpsc::Sender<AgentEvent>,
    egui_ctx: egui::Context,
) {
    tokio::task::spawn_local(async move {
        let mut lines = BufReader::new(stdout).lines();
        loop {
            let line = match lines.next_line().await {
                Ok(Some(l)) => l,
                Ok(None) | Err(_) => break,
            };
            if line.trim().is_empty() {
                continue;
            }
            let Ok(value) = serde_json::from_str::<Value>(&line) else {
                continue;
            };
            handle_incoming(&value, &pending, &writer_tx, &event_tx, &egui_ctx);
        }
    });
}

fn handle_incoming(
    value: &Value,
    pending: &PendingMap,
    writer_tx: &mpsc::UnboundedSender<String>,
    event_tx: &std::sync::mpsc::Sender<AgentEvent>,
    egui_ctx: &egui::Context,
) {
    let emit = |e: AgentEvent| {
        let _ = event_tx.send(e);
        egui_ctx.request_repaint();
    };

    let has_result_or_error = value.get("result").is_some() || value.get("error").is_some();
    if let Some(id) = value.get("id") {
        if has_result_or_error {
            // Response to one of OUR requests.
            let Some(id_num) = id.as_u64() else { return };
            if let Some(tx) = pending.borrow_mut().remove(&id_num) {
                let outcome = if let Some(err) = value.get("error") {
                    Err(err
                        .get("message")
                        .and_then(|m| m.as_str())
                        .unwrap_or("unknown error")
                        .to_string())
                } else {
                    Ok(value.get("result").cloned().unwrap_or(Value::Null))
                };
                let _ = tx.send(outcome);
            }
            return;
        }
        // Incoming request FROM zeroclaw (e.g. session/request_permission).
        let method = value.get("method").and_then(|m| m.as_str()).unwrap_or("");
        let response = build_permission_auto_response(method, value.get("params"));
        let line = json!({"jsonrpc": "2.0", "id": id.clone(), "result": response}).to_string();
        let _ = writer_tx.send(line);
        return;
    }

    // Notification.
    let method = value.get("method").and_then(|m| m.as_str()).unwrap_or("");
    if method != "session/update" {
        return;
    }
    let Some(update) = value.get("params").and_then(|p| p.get("update")) else {
        return;
    };
    let kind = update.get("sessionUpdate").and_then(|k| k.as_str()).unwrap_or("");
    match kind {
        "agent_message_chunk" => {
            if let Some(text) = text_content(update.get("content")) {
                emit(AgentEvent::AssistantDelta(text));
            }
        }
        "agent_thought_chunk" => {
            if let Some(text) = text_content(update.get("content")) {
                emit(AgentEvent::ThoughtDelta(text));
            }
        }
        "tool_call" => {
            let id = update.get("toolCallId").and_then(|v| v.as_str()).unwrap_or("").to_string();
            let title = update.get("title").and_then(|v| v.as_str()).unwrap_or("").to_string();
            let kind = update.get("kind").and_then(|v| v.as_str()).unwrap_or("").to_string();
            let detail = update
                .get("rawInput")
                .map(pretty_json_truncated)
                .unwrap_or_default();
            emit(AgentEvent::ToolStart { id, title, kind, detail });
        }
        "tool_call_update" => {
            let id = update.get("toolCallId").and_then(|v| v.as_str()).unwrap_or("").to_string();
            let status = update.get("status").and_then(|v| v.as_str()).unwrap_or("").to_string();
            let kind = update.get("kind").and_then(|v| v.as_str()).unwrap_or("").to_string();
            let detail = update
                .get("rawOutput")
                .map(pretty_json_truncated)
                .unwrap_or_default();
            emit(AgentEvent::ToolUpdate { id, status, kind, detail });
        }
        _ => {}
    }
}

fn text_content(content: Option<&Value>) -> Option<String> {
    let content = content?;
    if content.get("type")?.as_str()? != "text" {
        return None;
    }
    let text = content.get("text")?.as_str()?;
    if text.is_empty() {
        return None;
    }
    Some(text.to_string())
}

fn pretty_json_truncated(value: &Value) -> String {
    let raw = format_tool_payload(value);
    const MAX: usize = 4000;
    if raw.chars().count() <= MAX {
        return raw;
    }
    let mut out: String = raw.chars().take(MAX).collect();
    out.push('…');
    out
}

/// Render tool input/output for humans: plain strings keep real newlines
/// (not `\"…\\n…\"` JSON escaping). Nested `{content|text|result|output}`
/// wrappers unwrap to the inner text when it's a string.
fn format_tool_payload(value: &Value) -> String {
    match value {
        Value::Null => String::new(),
        Value::Bool(b) => b.to_string(),
        Value::Number(n) => n.to_string(),
        Value::String(s) => s.clone(),
        Value::Array(items) => {
            // Single-string array → show that string; else pretty JSON.
            if items.len() == 1 {
                if let Some(s) = items[0].as_str() {
                    return s.to_string();
                }
            }
            serde_json::to_string_pretty(value).unwrap_or_else(|_| value.to_string())
        }
        Value::Object(map) => {
            for key in ["content", "text", "result", "output", "message", "stdout"] {
                if let Some(inner) = map.get(key) {
                    if let Some(s) = inner.as_str() {
                        // Prefer long multi-line tool bodies over full object dump.
                        if s.lines().count() > 1 || s.len() > 80 {
                            return s.to_string();
                        }
                    }
                }
            }
            serde_json::to_string_pretty(value).unwrap_or_else(|_| value.to_string())
        }
    }
}

/// Always resolve `session/request_permission` immediately: prefer
/// "allow_once", then "allow_always", else the first option, else cancel.
/// Keeps a general-assistant ZeroClaw turn from hanging on a permission UI
/// Bony Build doesn't render for this backend in v1.
fn build_permission_auto_response(method: &str, params: Option<&Value>) -> Value {
    if method != "session/request_permission" {
        return json!({ "outcome": { "outcome": "cancelled" } });
    }
    let options = params
        .and_then(|p| p.get("options"))
        .and_then(|o| o.as_array())
        .cloned()
        .unwrap_or_default();
    let pick = options
        .iter()
        .find(|o| o.get("kind").and_then(|k| k.as_str()) == Some("allow_once"))
        .or_else(|| {
            options
                .iter()
                .find(|o| o.get("kind").and_then(|k| k.as_str()) == Some("allow_always"))
        })
        .or_else(|| options.first());
    match pick.and_then(|o| o.get("optionId")).cloned() {
        Some(option_id) => json!({ "outcome": { "outcome": "selected", "optionId": option_id } }),
        None => json!({ "outcome": { "outcome": "cancelled" } }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn drain(rx: &std::sync::mpsc::Receiver<AgentEvent>) -> Vec<AgentEvent> {
        let mut out = Vec::new();
        while let Ok(e) = rx.try_recv() {
            out.push(e);
        }
        out
    }

    fn dispatch(value: Value) -> Vec<AgentEvent> {
        let pending: PendingMap = Rc::new(RefCell::new(HashMap::new()));
        let (writer_tx, _writer_rx) = mpsc::unbounded_channel::<String>();
        let (event_tx, event_rx) = std::sync::mpsc::channel::<AgentEvent>();
        let ctx = egui::Context::default();
        handle_incoming(&value, &pending, &writer_tx, &event_tx, &ctx);
        drain(&event_rx)
    }

    #[test]
    fn session_update_agent_message_chunk_becomes_assistant_delta() {
        let events = dispatch(json!({
            "jsonrpc": "2.0",
            "method": "session/update",
            "params": {
                "update": {
                    "sessionUpdate": "agent_message_chunk",
                    "content": { "type": "text", "text": "你好" }
                }
            }
        }));
        assert_eq!(events.len(), 1);
        assert!(matches!(&events[0], AgentEvent::AssistantDelta(t) if t == "你好"));
    }

    #[test]
    fn session_update_agent_thought_chunk_becomes_thought_delta() {
        let events = dispatch(json!({
            "jsonrpc": "2.0",
            "method": "session/update",
            "params": {
                "update": {
                    "sessionUpdate": "agent_thought_chunk",
                    "content": { "type": "text", "text": "thinking…" }
                }
            }
        }));
        assert_eq!(events.len(), 1);
        assert!(matches!(&events[0], AgentEvent::ThoughtDelta(t) if t == "thinking…"));
    }

    #[test]
    fn session_update_tool_call_becomes_tool_start() {
        let events = dispatch(json!({
            "jsonrpc": "2.0",
            "method": "session/update",
            "params": {
                "update": {
                    "sessionUpdate": "tool_call",
                    "toolCallId": "t1",
                    "title": "发送提醒",
                    "kind": "other",
                    "rawInput": {"when": "9am"}
                }
            }
        }));
        assert_eq!(events.len(), 1);
        match &events[0] {
            AgentEvent::ToolStart { id, title, kind, .. } => {
                assert_eq!(id, "t1");
                assert_eq!(title, "发送提醒");
                assert_eq!(kind, "other");
            }
            other => panic!("unexpected event: {other:?}"),
        }
    }

    #[test]
    fn empty_text_content_is_ignored() {
        let events = dispatch(json!({
            "jsonrpc": "2.0",
            "method": "session/update",
            "params": {
                "update": {
                    "sessionUpdate": "agent_message_chunk",
                    "content": { "type": "text", "text": "" }
                }
            }
        }));
        assert!(events.is_empty());
    }

    #[test]
    fn unrelated_notification_is_ignored() {
        let events = dispatch(json!({
            "jsonrpc": "2.0",
            "method": "some/other",
            "params": {}
        }));
        assert!(events.is_empty());
    }

    #[test]
    fn response_with_result_resolves_pending_request() {
        let pending: PendingMap = Rc::new(RefCell::new(HashMap::new()));
        let (writer_tx, _writer_rx) = mpsc::unbounded_channel::<String>();
        let (event_tx, _event_rx) = std::sync::mpsc::channel::<AgentEvent>();
        let ctx = egui::Context::default();
        let (tx, mut rx) = oneshot::channel();
        pending.borrow_mut().insert(1, tx);

        handle_incoming(
            &json!({"jsonrpc": "2.0", "id": 1, "result": {"sessionId": "abc"}}),
            &pending,
            &writer_tx,
            &event_tx,
            &ctx,
        );

        let outcome = rx.try_recv().expect("resolved");
        assert_eq!(outcome.unwrap().get("sessionId").unwrap(), "abc");
        assert!(pending.borrow().is_empty());
    }

    #[test]
    fn response_with_error_resolves_pending_request_as_err() {
        let pending: PendingMap = Rc::new(RefCell::new(HashMap::new()));
        let (writer_tx, _writer_rx) = mpsc::unbounded_channel::<String>();
        let (event_tx, _event_rx) = std::sync::mpsc::channel::<AgentEvent>();
        let ctx = egui::Context::default();
        let (tx, mut rx) = oneshot::channel();
        pending.borrow_mut().insert(7, tx);

        handle_incoming(
            &json!({"jsonrpc": "2.0", "id": 7, "error": {"message": "boom"}}),
            &pending,
            &writer_tx,
            &event_tx,
            &ctx,
        );

        let outcome = rx.try_recv().expect("resolved");
        assert_eq!(outcome.unwrap_err(), "boom");
    }

    #[test]
    fn incoming_permission_request_is_auto_answered_allow_once() {
        let pending: PendingMap = Rc::new(RefCell::new(HashMap::new()));
        let (writer_tx, mut writer_rx) = mpsc::unbounded_channel::<String>();
        let (event_tx, _event_rx) = std::sync::mpsc::channel::<AgentEvent>();
        let ctx = egui::Context::default();

        handle_incoming(
            &json!({
                "jsonrpc": "2.0",
                "id": 42,
                "method": "session/request_permission",
                "params": {
                    "options": [
                        {"kind": "allow_always", "optionId": "always"},
                        {"kind": "allow_once", "optionId": "once"}
                    ]
                }
            }),
            &pending,
            &writer_tx,
            &event_tx,
            &ctx,
        );

        let line = writer_rx.try_recv().expect("reply written");
        let reply: Value = serde_json::from_str(&line).unwrap();
        assert_eq!(reply["id"], 42);
        assert_eq!(reply["result"]["outcome"]["outcome"], "selected");
        assert_eq!(reply["result"]["outcome"]["optionId"], "once");
    }

    #[test]
    fn permission_auto_response_prefers_allow_once_over_allow_always() {
        let resp = build_permission_auto_response(
            "session/request_permission",
            Some(&json!({
                "options": [
                    {"kind": "allow_always", "optionId": "a"},
                    {"kind": "allow_once", "optionId": "b"}
                ]
            })),
        );
        assert_eq!(resp["outcome"]["optionId"], "b");
    }

    #[test]
    fn permission_auto_response_falls_back_to_first_option() {
        let resp = build_permission_auto_response(
            "session/request_permission",
            Some(&json!({
                "options": [{"kind": "reject_once", "optionId": "r"}]
            })),
        );
        assert_eq!(resp["outcome"]["optionId"], "r");
    }

    #[test]
    fn permission_auto_response_cancels_when_no_options() {
        let resp = build_permission_auto_response(
            "session/request_permission",
            Some(&json!({ "options": [] })),
        );
        assert_eq!(resp["outcome"]["outcome"], "cancelled");
    }

    #[test]
    fn permission_auto_response_cancels_for_other_methods() {
        let resp = build_permission_auto_response("session/other", None);
        assert_eq!(resp["outcome"]["outcome"], "cancelled");
    }

    #[test]
    fn text_content_rejects_non_text_type() {
        assert_eq!(text_content(Some(&json!({"type": "image", "text": "x"}))), None);
        assert_eq!(text_content(None), None);
    }

    #[test]
    fn pretty_json_truncated_appends_ellipsis_when_over_limit() {
        let big = json!("x".repeat(5000));
        let out = pretty_json_truncated(&big);
        assert!(out.ends_with('…'));
        assert!(out.chars().count() <= 4001);
    }

    #[test]
    fn pretty_json_truncated_leaves_small_payload_untouched() {
        let small = json!({"a": 1});
        let out = pretty_json_truncated(&small);
        assert!(!out.ends_with('…'));
        assert!(out.contains("\"a\""));
    }

    #[test]
    fn format_tool_payload_uses_real_newlines_for_string_values() {
        let v = json!("Weather for Shenzhen\nTemperature: 30°C\nHumidity: 68%");
        let out = format_tool_payload(&v);
        assert!(!out.contains("\\n"));
        assert!(out.contains('\n'));
        assert!(out.starts_with("Weather for Shenzhen"));
        // must not be JSON-quoted
        assert!(!out.starts_with('"'));
    }

    #[test]
    fn format_tool_payload_unwraps_content_object() {
        let v = json!({"content": "line1\nline2"});
        assert_eq!(format_tool_payload(&v), "line1\nline2");
    }
}
