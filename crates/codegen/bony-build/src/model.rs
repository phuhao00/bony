//! UI-facing session state (Codex-style timeline).

use std::path::PathBuf;

use crate::events::{AgentEvent, ModeChoice, ModelChoice, PermissionOptionView, PlanEntryView};
use crate::usage::{
    SessionUsageState, TaskSummary, TokenUsage, TurnRecord, aggregate_tasks, load_recent_projects,
    load_recent_turns, remember_project,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Role {
    User,
    Assistant,
    System,
}

/// Which ACP backend produced a message — used only to render a small,
/// unobtrusive source tag on assistant bubbles. Not a plugin toggle: both
/// backends share one timeline, one composer, one chat window.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum MessageSource {
    #[default]
    Grok,
    Zeroclaw,
}

#[derive(Debug, Clone)]
pub struct ChatMessage {
    pub role: Role,
    pub text: String,
    /// Per-turn token bill (set on assistant messages when a turn completes).
    pub turn_usage: Option<TokenUsage>,
    pub source: MessageSource,
}

#[derive(Debug, Clone)]
pub struct ToolCard {
    pub id: String,
    pub title: String,
    pub kind: String,
    pub status: String,
    pub detail: String,
    pub open: bool,
}

#[derive(Debug, Clone)]
pub struct ThoughtCard {
    pub text: String,
    pub open: bool,
}

#[derive(Debug, Clone)]
pub struct PlanCard {
    pub entries: Vec<PlanEntryView>,
    pub open: bool,
}

#[derive(Debug, Clone)]
pub struct RouteCard {
    /// Fixed intent backend name: "coding" | "general"
    pub intent: String,
    /// Why we classified that way.
    pub intent_reason: String,
    pub matched_keyword: Option<String>,
    /// Intended target before degrade: "grok" | "zeroclaw"
    pub intended: String,
    /// Actual backend used: "grok" | "zeroclaw"
    pub actual: String,
    pub zc_status: String,
    pub steps: Vec<String>,
    pub degraded: bool,
    pub degrade_reason: Option<String>,
    pub open: bool,
}

#[derive(Debug, Clone)]
pub enum TimelineItem {
    Message(ChatMessage),
    Tool(ToolCard),
    Thought(ThoughtCard),
    Plan(PlanCard),
    Route(RouteCard),
}

#[derive(Debug, Clone)]
pub struct PendingPermission {
    pub tool_call_id: String,
    pub title: String,
    pub options: Vec<PermissionOptionView>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum UsageTab {
    #[default]
    Charts,
    Models,
    Turns,
}

/// Primary left-nav destination (Codex-style shell).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum MainNav {
    #[default]
    Chat,
    Unity,
    Scheduled,
    Plugins,
    Sites,
    PullRequests,
}

impl MainNav {
    pub fn title(self) -> &'static str {
        self.title_lang(crate::i18n::Language::Zh)
    }

    pub fn title_lang(self, lang: crate::i18n::Language) -> &'static str {
        use crate::i18n::t;
        match self {
            Self::Chat => t(lang, "nav.chat"),
            Self::Unity => t(lang, "nav.unity"),
            Self::Scheduled => t(lang, "nav.scheduled"),
            Self::Plugins => t(lang, "nav.plugins"),
            Self::Sites => t(lang, "nav.sites"),
            Self::PullRequests => t(lang, "nav.prs"),
        }
    }

    pub fn placeholder_blurb(self) -> &'static str {
        match self {
            Self::Chat => "",
            Self::Unity => "",
            Self::Scheduled => "定时任务与提醒将出现在这里。当前版本尚未接入调度能力。",
            Self::Plugins => "启用或关闭本地扩展能力，例如 Unity 编辑器控制。",
            Self::Sites => "管理预览站点与部署入口。站点功能即将推出。",
            Self::PullRequests => "查看与处理拉取请求。Git 集成即将推出。",
        }
    }
}

#[derive(Debug, Default)]
pub struct AppModel {
    pub status: String,
    pub session_id: Option<String>,
    pub cwd: Option<PathBuf>,
    pub timeline: Vec<TimelineItem>,
    /// Snapshot of live timeline while viewing a historical task.
    pub live_timeline: Vec<TimelineItem>,
    pub pending_permission: Option<PendingPermission>,
    pub busy: bool,
    pub draft: String,
    pub auto_scroll: bool,
    pub connected: bool,
    pub needs_login: bool,
    pub login_message: String,
    pub current_model_id: String,
    pub current_model_name: String,
    pub available_models: Vec<ModelChoice>,
    pub show_model_picker: bool,
    pub current_mode_id: String,
    pub available_modes: Vec<ModeChoice>,
    pub show_user_menu: bool,
    pub show_usage_detail: bool,
    pub show_about: bool,
    pub show_left_sidebar: bool,
    pub show_right_panel: bool,
    pub focus_composer: bool,
    pub main_nav: MainNav,
    /// Local filter for the task list (sidebar search).
    pub task_filter: String,
    pub show_task_search: bool,
    pub usage_tab: UsageTab,
    /// `None` = live current session; `Some` = read-only history view.
    pub viewing_session_id: Option<String>,
    pub task_title: String,
    pub display_name: String,
    pub usage: SessionUsageState,
    /// Recent turns loaded from disk (includes prior sessions).
    pub history_turns: Vec<TurnRecord>,
    /// Expanded turn ids in the usage detail panel.
    pub history_expanded: Vec<String>,
    /// Recent project working directories.
    pub recent_projects: Vec<PathBuf>,
}

impl AppModel {
    pub fn new(initial_cwd: PathBuf) -> Self {
        let mut recent = load_recent_projects();
        remember_project(&mut recent, &initial_cwd);
        let catalog = crate::config_io::load_models_catalog();
        let (current_model_id, current_model_name) = catalog
            .default_id
            .as_ref()
            .and_then(|id| {
                catalog
                    .models
                    .iter()
                    .find(|m| &m.id == id)
                    .map(|m| (m.id.clone(), m.name.clone()))
            })
            .or_else(|| {
                catalog
                    .models
                    .first()
                    .map(|m| (m.id.clone(), m.name.clone()))
            })
            .unwrap_or_else(|| (String::new(), "model".into()));
        Self {
            status: "Connecting…".into(),
            auto_scroll: true,
            login_message: "Sign in to chat with Bony Build.".into(),
            current_model_id,
            current_model_name,
            available_models: catalog.models,
            task_title: "新对话".into(),
            display_name: default_display_name(),
            history_turns: load_recent_turns(80),
            cwd: Some(initial_cwd),
            recent_projects: recent,
            show_left_sidebar: true,
            ..Default::default()
        }
    }

    pub fn go_chat(&mut self) {
        self.main_nav = MainNav::Chat;
    }

    pub fn apply(&mut self, event: AgentEvent) {
        match event {
            AgentEvent::Status(s) => self.status = s,
            AgentEvent::NeedsLogin { message } => {
                self.needs_login = true;
                self.connected = false;
                self.busy = false;
                self.login_message = message;
                self.status = "Sign in required".into();
            }
            AgentEvent::Disconnected => {
                self.connected = false;
                self.session_id = None;
                self.status = "Reconnecting…".into();
            }
            AgentEvent::Connected {
                session_id,
                cwd,
                current_model_id,
                current_model_name,
                models,
                current_mode_id,
                modes,
                restored: _,
            } => {
                self.session_id = Some(session_id);
                self.cwd = Some(cwd);
                self.connected = true;
                self.needs_login = false;
                self.busy = false;
                self.current_model_id = current_model_id;
                self.current_model_name = current_model_name;
                self.available_models = models;
                self.current_mode_id = current_mode_id;
                self.available_modes = modes;
                self.status = "Ready".into();
            }
            AgentEvent::ModelChanged { model_id, name } => {
                self.current_model_id = model_id;
                self.current_model_name = name;
                self.show_model_picker = false;
                self.status = "Ready".into();
            }
            AgentEvent::ModeChanged { mode_id } => {
                self.current_mode_id = mode_id;
                self.status = "Ready".into();
            }
            AgentEvent::AssistantDelta(delta) => {
                self.ensure_live_view();
                self.busy = true;
                self.status = "Working…".into();
                match self.timeline.last_mut() {
                    Some(TimelineItem::Message(m)) if m.role == Role::Assistant => {
                        merge_stream_text(&mut m.text, &delta);
                    }
                    _ => self.timeline.push(TimelineItem::Message(ChatMessage {
                        role: Role::Assistant,
                        text: delta,
                        turn_usage: None,
                        source: MessageSource::Grok,
                    })),
                }
            }
            AgentEvent::ThoughtDelta(delta) => {
                self.ensure_live_view();
                self.busy = true;
                self.status = "Thinking…".into();
                match self.timeline.last_mut() {
                    Some(TimelineItem::Thought(t)) => {
                        merge_stream_text(&mut t.text, &delta);
                    }
                    _ => self.timeline.push(TimelineItem::Thought(ThoughtCard {
                        text: delta,
                        open: true,
                    })),
                }
            }
            AgentEvent::PlanUpdate { entries } => {
                self.ensure_live_view();
                self.busy = true;
                self.status = "Planning…".into();
                // Replace the latest plan after the most recent user message.
                let mut replace_idx: Option<usize> = None;
                for (idx, item) in self.timeline.iter().enumerate().rev() {
                    match item {
                        TimelineItem::Message(m) if m.role == Role::User => break,
                        TimelineItem::Plan(_) => {
                            replace_idx = Some(idx);
                            break;
                        }
                        _ => {}
                    }
                }
                if let Some(idx) = replace_idx {
                    if let Some(TimelineItem::Plan(plan)) = self.timeline.get_mut(idx) {
                        plan.entries = entries;
                    }
                } else {
                    self.timeline.push(TimelineItem::Plan(PlanCard {
                        entries,
                        open: true,
                    }));
                }
            }
            AgentEvent::ToolStart {
                id,
                title,
                kind,
                detail,
            } => {
                self.ensure_live_view();
                self.busy = true;
                self.status = "Running tools…".into();
                if let Some(card) = self.find_tool_mut(&id) {
                    card.title = title;
                    if !kind.is_empty() {
                        card.kind = kind;
                    }
                    card.status = "InProgress".into();
                    if !detail.is_empty() {
                        card.detail = detail;
                    }
                    card.open = true;
                } else {
                    self.timeline.push(TimelineItem::Tool(ToolCard {
                        id,
                        title,
                        kind,
                        status: "InProgress".into(),
                        detail,
                        open: true,
                    }));
                }
            }
            AgentEvent::ToolUpdate {
                id,
                status,
                kind,
                detail,
            } => {
                self.ensure_live_view();
                if let Some(card) = self.find_tool_mut(&id) {
                    if !status.is_empty() {
                        card.status = status;
                    }
                    if !kind.is_empty() {
                        card.kind = kind;
                    }
                    // ACP content/locations/raw_* are full snapshots, not deltas.
                    if !detail.is_empty() {
                        card.detail = detail;
                    }
                    card.open = true;
                } else {
                    self.timeline.push(TimelineItem::Tool(ToolCard {
                        id,
                        title: "Tool".into(),
                        kind,
                        status,
                        detail,
                        open: true,
                    }));
                }
            }
            AgentEvent::PermissionRequest {
                tool_call_id,
                title,
                options,
            } => {
                self.pending_permission = Some(PendingPermission {
                    tool_call_id,
                    title,
                    options,
                });
                self.status = "Needs approval".into();
            }
            AgentEvent::ContextUsage { used, size } => {
                self.usage.apply_context_window(used, size);
            }
            AgentEvent::TurnDone { stop_reason, usage } => {
                self.ensure_live_view();
                self.busy = false;
                self.status = "Ready".into();
                let session_id = self.session_id.clone().unwrap_or_else(|| "local".into());
                let assistant = self.last_assistant_text();
                let tools = self.tools_since_last_user();
                let record = self.usage.finish_turn(
                    &session_id,
                    &self.current_model_id,
                    &self.current_model_name,
                    &stop_reason,
                    assistant,
                    tools,
                    usage,
                );
                if let Some(TimelineItem::Message(m)) =
                    self.timeline.iter_mut().rev().find(
                        |i| matches!(i, TimelineItem::Message(m) if m.role == Role::Assistant),
                    )
                {
                    m.turn_usage = Some(record.usage_delta.clone());
                }
                if is_default_task_title(&self.task_title) && !record.user_text.is_empty() {
                    self.task_title = truncate_chars(&record.user_text, 28);
                }
                self.history_turns.push(record);
                if self.history_turns.len() > 200 {
                    let drop_n = self.history_turns.len() - 200;
                    self.history_turns.drain(0..drop_n);
                }
            }
            AgentEvent::Error(err) => {
                self.ensure_live_view();
                self.busy = false;
                self.status = "Error".into();
                self.timeline.push(TimelineItem::Message(ChatMessage {
                    role: Role::System,
                    text: err,
                    turn_usage: None,
                    source: MessageSource::Grok,
                }));
            }
        }
    }

    /// Apply an [`AgentEvent`] produced by the ZeroClaw backend into the same
    /// shared timeline `apply()` uses for grok — deliberately a *separate*
    /// method rather than a `source` parameter on `apply()`: ZeroClaw is a
    /// second, independent ACP connection with its own session lifecycle, so
    /// its `Connected`/`Disconnected`/`NeedsLogin`/model/mode events must
    /// never clobber grok's connection state (`connected`, `session_id`,
    /// `available_models`, …), which the rest of the UI treats as singular.
    /// Only the message/tool/thought stream — the part that genuinely
    /// belongs on one unified timeline — is handled here.
    pub fn apply_zc(&mut self, event: AgentEvent) {
        match event {
            AgentEvent::Status(s) => self.status = s,
            AgentEvent::AssistantDelta(delta) => {
                self.ensure_live_view();
                self.busy = true;
                self.status = "Working…".into();
                match self.timeline.last_mut() {
                    Some(TimelineItem::Message(m))
                        if m.role == Role::Assistant && m.source == MessageSource::Zeroclaw =>
                    {
                        merge_stream_text(&mut m.text, &delta);
                    }
                    _ => self.timeline.push(TimelineItem::Message(ChatMessage {
                        role: Role::Assistant,
                        text: delta,
                        turn_usage: None,
                        source: MessageSource::Zeroclaw,
                    })),
                }
            }
            AgentEvent::ThoughtDelta(delta) => {
                self.ensure_live_view();
                self.busy = true;
                match self.timeline.last_mut() {
                    Some(TimelineItem::Thought(t)) => merge_stream_text(&mut t.text, &delta),
                    _ => self.timeline.push(TimelineItem::Thought(ThoughtCard {
                        text: delta,
                        open: true,
                    })),
                }
            }
            AgentEvent::ToolStart { id, title, kind, detail } => {
                self.ensure_live_view();
                self.busy = true;
                if let Some(card) = self.find_tool_mut(&id) {
                    card.title = title;
                    card.status = "InProgress".into();
                    card.detail = detail;
                    card.open = true;
                } else {
                    self.timeline.push(TimelineItem::Tool(ToolCard {
                        id,
                        title,
                        kind,
                        status: "InProgress".into(),
                        detail,
                        open: true,
                    }));
                }
            }
            AgentEvent::ToolUpdate { id, status, kind, detail } => {
                self.ensure_live_view();
                if let Some(card) = self.find_tool_mut(&id) {
                    if !status.is_empty() {
                        card.status = status;
                    }
                    if !kind.is_empty() {
                        card.kind = kind;
                    }
                    if !detail.is_empty() {
                        card.detail = detail;
                    }
                } else {
                    self.timeline.push(TimelineItem::Tool(ToolCard {
                        id,
                        title: "Tool".into(),
                        kind,
                        status,
                        detail,
                        open: true,
                    }));
                }
            }
            AgentEvent::TurnDone { .. } => {
                self.busy = false;
                self.status = "Ready".into();
            }
            AgentEvent::Error(err) => {
                self.ensure_live_view();
                self.busy = false;
                self.status = "Error".into();
                self.timeline.push(TimelineItem::Message(ChatMessage {
                    role: Role::System,
                    text: err,
                    turn_usage: None,
                    source: MessageSource::Zeroclaw,
                }));
            }
            // Connection/model/mode/permission events belong to grok's
            // singular connection state; ZeroClaw's bridge intentionally
            // never emits `PermissionRequest` (auto-answered at the bridge
            // layer) and the rest don't apply to a stdio-only backend.
            _ => {}
        }
    }

    /// Tag the just-sent user bubble as routed to ZeroClaw, so the timeline
    /// can (optionally) reflect it — called right after `push_user`.
    pub fn mark_last_user_zeroclaw(&mut self) {
        if let Some(TimelineItem::Message(m)) = self.timeline.last_mut()
            && m.role == Role::User
        {
            m.source = MessageSource::Zeroclaw;
        }
    }

    /// Append a routing card after the user bubble so every turn shows who
    /// handled it and why (including degrade path).
    pub fn push_route(&mut self, card: RouteCard) {
        self.ensure_live_view();
        self.timeline.push(TimelineItem::Route(card));
        self.auto_scroll = true;
    }

    pub fn push_user(&mut self, text: String) {
        self.ensure_live_view();
        if is_default_task_title(&self.task_title) {
            self.task_title = truncate_chars(&text, 28);
        }
        self.usage.begin_turn(&text);
        self.timeline.push(TimelineItem::Message(ChatMessage {
            role: Role::User,
            text,
            turn_usage: None,
            source: MessageSource::Grok,
        }));
        self.busy = true;
        self.status = "Working…".into();
    }

    /// Complete an app-owned action without involving the ACP agent stream.
    pub fn push_local_assistant(&mut self, text: String) {
        self.ensure_live_view();
        self.timeline.push(TimelineItem::Message(ChatMessage {
            role: Role::Assistant,
            text,
            turn_usage: None,
            source: MessageSource::Grok,
        }));
        self.busy = false;
        self.status = "Ready".into();
        self.auto_scroll = true;
    }

    pub fn push_local_user(&mut self, text: String) {
        self.ensure_live_view();
        self.timeline.push(TimelineItem::Message(ChatMessage {
            role: Role::User,
            text,
            turn_usage: None,
            source: MessageSource::Grok,
        }));
        self.busy = true;
        self.status = "正在控制 Unity…".into();
        self.auto_scroll = true;
    }

    /// Clear local chat and start a fresh task UI (same ACP session).
    pub fn new_task(&mut self) {
        self.main_nav = MainNav::Chat;
        self.viewing_session_id = None;
        self.live_timeline.clear();
        self.timeline.clear();
        self.usage.turns.clear();
        self.usage.pending_user_text.clear();
        self.usage.pending_started_at.clear();
        // Keep cumulative session token totals across "new task" clears.
        self.task_title = "新对话".into();
        self.draft.clear();
        self.pending_permission = None;
        self.auto_scroll = true;
        self.show_user_menu = false;
        if self.connected && !self.needs_login {
            self.status = "Ready".into();
        }
    }

    pub fn project_label(path: &std::path::Path) -> String {
        path.file_name()
            .map(|s| s.to_string_lossy().to_string())
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| path.to_string_lossy().to_string())
    }

    pub fn filtered_tasks(&self) -> Vec<TaskSummary> {
        let q = self.task_filter.trim().to_lowercase();
        let tasks = self.tasks();
        if q.is_empty() {
            return tasks;
        }
        tasks
            .into_iter()
            .filter(|t| t.title.to_lowercase().contains(&q))
            .collect()
    }

    /// Read-only replay of a historical session's turns.
    pub fn load_task_view(&mut self, session_id: &str) {
        self.main_nav = MainNav::Chat;
        if self.viewing_session_id.is_none() {
            self.live_timeline = self.timeline.clone();
        }
        self.viewing_session_id = Some(session_id.to_string());
        let turns: Vec<&TurnRecord> = self
            .history_turns
            .iter()
            .filter(|t| t.session_id == session_id)
            .collect();
        self.task_title = turns
            .first()
            .map(|t| truncate_chars(&t.user_text, 42))
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| "历史任务".into());
        let mut timeline = Vec::new();
        for turn in turns {
            if !turn.user_text.is_empty() {
                timeline.push(TimelineItem::Message(ChatMessage {
                    role: Role::User,
                    text: turn.user_text.clone(),
                    turn_usage: None,
                    source: MessageSource::Grok,
                }));
            }
            for tool in &turn.tool_titles {
                timeline.push(TimelineItem::Tool(ToolCard {
                    id: format!("{}-{}", turn.id, tool),
                    title: tool.clone(),
                    kind: String::new(),
                    status: "Completed".into(),
                    detail: String::new(),
                    open: false,
                }));
            }
            if !turn.assistant_text.is_empty() {
                timeline.push(TimelineItem::Message(ChatMessage {
                    role: Role::Assistant,
                    text: turn.assistant_text.clone(),
                    turn_usage: Some(turn.usage_delta.clone()),
                    source: MessageSource::Grok,
                }));
            }
        }
        self.timeline = timeline;
        self.auto_scroll = true;
        self.show_user_menu = false;
    }

    pub fn return_to_live(&mut self) {
        self.main_nav = MainNav::Chat;
        if self.viewing_session_id.take().is_some() {
            self.timeline = std::mem::take(&mut self.live_timeline);
            self.task_title = self
                .timeline
                .iter()
                .find_map(|i| match i {
                    TimelineItem::Message(m) if m.role == Role::User => {
                        Some(truncate_chars(&m.text, 42))
                    }
                    _ => None,
                })
                .filter(|s| !s.is_empty())
                .unwrap_or_else(|| "新对话".into());
        }
    }

    fn ensure_live_view(&mut self) {
        if self.viewing_session_id.is_some() {
            self.return_to_live();
        }
    }

    pub fn tasks(&self) -> Vec<TaskSummary> {
        aggregate_tasks(&self.history_turns)
    }

    pub fn is_viewing_history(&self) -> bool {
        self.viewing_session_id.is_some()
    }

    pub fn initials(&self) -> String {
        let name = self.display_name.trim();
        let mut chars = name.chars().filter(|c| !c.is_whitespace());
        let a = chars.next().unwrap_or('B');
        let b = chars.next().unwrap_or('B');
        format!("{a}{b}").to_uppercase()
    }

    fn last_assistant_text(&self) -> String {
        for item in self.timeline.iter().rev() {
            if let TimelineItem::Message(m) = item
                && m.role == Role::Assistant
            {
                return m.text.clone();
            }
        }
        String::new()
    }

    pub fn latest_assistant_text(&self) -> String {
        self.last_assistant_text()
    }

    pub fn replace_latest_assistant(&mut self, text: String) {
        if let Some(TimelineItem::Message(message)) = self
            .timeline
            .iter_mut()
            .rev()
            .find(|item| matches!(item, TimelineItem::Message(m) if m.role == Role::Assistant))
        {
            message.text = text;
        }
    }

    fn tools_since_last_user(&self) -> Vec<String> {
        let mut titles = Vec::new();
        for item in self.timeline.iter().rev() {
            match item {
                TimelineItem::Message(m) if m.role == Role::User => break,
                TimelineItem::Tool(t) => titles.push(t.title.clone()),
                _ => {}
            }
        }
        titles.reverse();
        titles
    }

    fn find_tool_mut(&mut self, id: &str) -> Option<&mut ToolCard> {
        for item in self.timeline.iter_mut().rev() {
            if let TimelineItem::Tool(card) = item
                && card.id == id
            {
                return Some(card);
            }
        }
        None
    }

    pub fn is_empty_chat(&self) -> bool {
        self.timeline
            .iter()
            .all(|i| matches!(i, TimelineItem::Message(m) if m.role == Role::System))
            || self.timeline.is_empty()
    }

    pub fn toggle_history_expanded(&mut self, id: &str) {
        if let Some(pos) = self.history_expanded.iter().position(|x| x == id) {
            self.history_expanded.remove(pos);
        } else {
            self.history_expanded.push(id.to_string());
        }
    }

    pub fn is_history_expanded(&self, id: &str) -> bool {
        self.history_expanded.iter().any(|x| x == id)
    }
}

/// Merge an ACP text update defensively. Normal ACP updates are deltas, but a
/// reconnecting or provider-specific bridge can re-deliver a long chunk or a
/// cumulative snapshot. Avoid rendering either form as repeated paragraphs.
fn merge_stream_text(current: &mut String, incoming: &str) {
    if incoming.is_empty() {
        return;
    }
    const LONG_CHUNK: usize = 32;
    if incoming.len() >= LONG_CHUNK && current.ends_with(incoming) {
        return;
    }
    if current.len() >= LONG_CHUNK && incoming.starts_with(current.as_str()) {
        *current = incoming.to_owned();
        return;
    }
    current.push_str(incoming);
}

#[cfg(test)]
mod stream_merge_tests {
    use super::merge_stream_text;

    #[test]
    fn keeps_normal_short_deltas() {
        let mut text = "ha".to_string();
        merge_stream_text(&mut text, "ha");
        assert_eq!(text, "haha");
    }

    #[test]
    fn drops_repeated_long_chunks() {
        let chunk = "这是一段足够长的流式响应，用来验证同一个 ACP 消息块不会被重复渲染。";
        let mut text = chunk.to_string();
        merge_stream_text(&mut text, chunk);
        assert_eq!(text, chunk);
    }

    #[test]
    fn accepts_cumulative_snapshots_without_appending_them() {
        let first = "This is a sufficiently long partial assistant response.";
        let full = format!("{first} And this is the completed suffix.");
        let mut text = first.to_string();
        merge_stream_text(&mut text, &full);
        assert_eq!(text, full);
    }
}

fn is_default_task_title(title: &str) -> bool {
    matches!(
        title.trim(),
        "" | "新任务" | "新对话" | "未命名对话" | "未命名任务"
    )
}

fn default_display_name() -> String {
    std::env::var("USERNAME")
        .or_else(|_| std::env::var("USER"))
        .unwrap_or_else(|_| "bony".into())
}

fn truncate_chars(s: &str, max: usize) -> String {
    let mut out = String::new();
    for (i, ch) in s.chars().enumerate() {
        if i >= max {
            out.push('…');
            break;
        }
        if ch == '\n' || ch == '\r' {
            break;
        }
        out.push(ch);
    }
    out.trim().to_string()
}
