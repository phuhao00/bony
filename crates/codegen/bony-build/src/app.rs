//! Codex-style shell: left task sidebar, main chat, floating composer.

use std::sync::mpsc;

use eframe::egui::{
    self, Align2, Color32, CornerRadius, CursorIcon, Frame, Margin, Pos2, RichText, Shadow, Stroke,
    Vec2,
};
use tokio::sync::mpsc as tokio_mpsc;

use crate::agent_bridge::{self, BridgeConfig};
use crate::charts;
use crate::events::{AgentEvent, AttachmentPayload, UiCommand};
use crate::git_workspace::{ChangeKind, FileChange, GitWorkspaceService};
use crate::markdown;
use crate::model::{AppModel, MainNav, Role, TimelineItem, UsageTab};
use crate::task::{
    PermissionMode, SqliteTaskRepository, TaskRepository, TaskState, TaskStatus, unix_time,
};
use crate::unity::{
    CliStatus, EVAL_PRESETS, EditorLinkStatus, LoopPhase, PipelineStatus, SetupStep, StepState,
    UNITY_CHAT_CHIPS, UnityAction, UnityChatCmd, UnityState, compile_unity_scene_command,
    format_relative, parse_generated_unity_plan_unrestricted, parse_unity_chat_command,
    unity_chat_help_text, wants_unity_help,
};
use crate::bevy::{BevyState, BevyStatus};
use crate::i18n::{self, Language, UiPrefs, load_ui_prefs, save_ui_prefs};
use crate::openmontage::{OpenMontageState, OpenMontageStatus};
use crate::usage::{
    ChatInteraction, PluginPrefs, aggregate_model_usage, forget_project, format_tokens,
    load_plugin_prefs, remember_project, save_plugin_prefs,
};

const BG: Color32 = Color32::from_rgb(22, 22, 24);
const SIDEBAR: Color32 = Color32::from_rgb(18, 18, 20);
const PANEL: Color32 = Color32::from_rgb(32, 32, 36);
const PANEL_2: Color32 = Color32::from_rgb(40, 40, 46);
const BORDER: Color32 = Color32::from_rgb(55, 55, 62);
const TEXT: Color32 = Color32::from_rgb(236, 236, 240);
const MUTED: Color32 = Color32::from_rgb(148, 150, 160);
const ACCENT: Color32 = Color32::from_rgb(245, 245, 247);
const USER_BG: Color32 = Color32::from_rgb(48, 52, 64);
const ASSIST_BG: Color32 = Color32::from_rgb(28, 28, 32);
const TOOL_BG: Color32 = Color32::from_rgb(30, 30, 34);
const DANGER: Color32 = Color32::from_rgb(220, 90, 90);
const OK: Color32 = Color32::from_rgb(110, 190, 130);
const ACCENT_BAR: Color32 = Color32::from_rgb(90, 140, 255);
const AVATAR: Color32 = Color32::from_rgb(70, 120, 220);
const SELECTED: Color32 = Color32::from_rgb(48, 50, 60);
const HOVER: Color32 = Color32::from_rgb(38, 38, 46);
const UNITY_ACCENT: Color32 = Color32::from_rgb(0, 180, 216);
const OM_ACCENT: Color32 = Color32::from_rgb(232, 120, 72);
const BEVY_ACCENT: Color32 = Color32::from_rgb(230, 126, 34);
const MAX_CHAT_W: f32 = 860.0;
const SIDEBAR_W: f32 = 248.0;
const RIGHT_PANEL_W: f32 = 280.0;
const TITLE_BAR_H: f32 = 36.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
enum TaskListFilter {
    #[default]
    All,
    Active,
    WaitingApproval,
    Completed,
    Failed,
    Archived,
}

#[derive(Clone)]
struct PendingUnityApproval {
    summary: String,
    csharp: String,
    risks: Vec<String>,
}

impl TaskListFilter {
    fn label(self) -> &'static str {
        match self {
            Self::All => "全部",
            Self::Active => "进行中",
            Self::WaitingApproval => "待审批",
            Self::Completed => "已完成",
            Self::Failed => "失败",
            Self::Archived => "已归档",
        }
    }

    fn matches(self, status: TaskStatus) -> bool {
        match self {
            // "全部"只列活跃对话；归档单独挂在项目下的「已归档」区。
            Self::All => !matches!(status, TaskStatus::Archived),
            Self::Active => matches!(status, TaskStatus::Draft | TaskStatus::Running),
            Self::WaitingApproval => status == TaskStatus::WaitingApproval,
            Self::Completed => status == TaskStatus::Completed,
            Self::Failed => status == TaskStatus::Failed,
            Self::Archived => status == TaskStatus::Archived,
        }
    }
}

pub struct BonyBuildApp {
    model: AppModel,
    event_rx: mpsc::Receiver<AgentEvent>,
    event_tx: mpsc::Sender<AgentEvent>,
    cmd_tx: Option<tokio_mpsc::UnboundedSender<UiCommand>>,
    started: bool,
    config: BridgeConfig,
    task_repo: Option<SqliteTaskRepository>,
    tasks: Vec<TaskState>,
    active_task_id: Option<String>,
    attachments: Vec<AttachmentPayload>,
    changes: Vec<FileChange>,
    selected_diff: Option<(std::path::PathBuf, String)>,
    task_error: Option<String>,
    pending_git_action: Option<(bool, std::path::PathBuf)>,
    task_list_filter: TaskListFilter,
    rename_task: Option<(String, String)>,
    delete_task: Option<String>,
    unity: UnityState,
    openmontage: OpenMontageState,
    bevy: BevyState,
    /// Latest operation id before a Unity action launched from chat.
    pending_unity_chat: Option<u64>,
    pending_unity_planner: bool,
    pending_unity_approval: Option<PendingUnityApproval>,
    /// Composer shows Unity quick-control chips (local CLI, not agent).
    unity_chat_mode: bool,
    /// Empty-state / inline Unity docs panel (commands stay behind this).
    unity_docs_open: bool,
    /// Floating docs window opened from the plugins manager.
    show_unity_docs_window: bool,
    /// Codex-style 「+」 menu above the composer.
    show_composer_plus: bool,
    /// Skip outside-click dismiss on the open frame.
    composer_plus_just_opened: bool,
    /// Anchor for the composer + menu (screen coords).
    composer_plus_anchor: Option<egui::Rect>,
    /// Skip outside-click dismiss on the open frame for account menu.
    user_menu_just_opened: bool,
    /// Anchor for the account menu (screen coords).
    user_menu_anchor: Option<egui::Rect>,
    /// Plugin enablement (persisted).
    plugin_prefs: PluginPrefs,
    /// UI prefs (language, etc.).
    ui_prefs: UiPrefs,
    /// Collapsed project keys in the sidebar conversation list.
    collapsed_projects: std::collections::HashSet<String>,
    /// Expanded "已归档" subsections keyed by project.
    expanded_archived: std::collections::HashSet<String>,
    /// After soft cancel, next Stop click force-kills the agent.
    stop_armed_force: bool,
    /// Background `git worktree add` so creating a task doesn't freeze the UI.
    pending_worktree_rx: Option<mpsc::Receiver<Result<(String, std::path::PathBuf, String), (String, String)>>>,
    /// Text field for the "create new Bevy project" flow in the plugins card.
    bevy_new_project_name: String,
    /// Manual title-bar window move (screen-absolute on Windows).
    window_dragging: bool,
    /// Cursor offset from window outer origin at drag start (points).
    window_drag_grab: Option<Vec2>,
    /// Cached sidebar grouping — rebuilt only when tasks / filter / cwd change.
    sidebar_groups_cache: Option<(u64, Vec<ConversationGroup>)>,
    /// Skip backdrop dismiss on the frame the delete dialog opens (same click would close it).
    delete_modal_ignore_click: bool,
    /// After top-level「新建对话」: no project/conversation selected until the user
    /// picks one (project row / per-project「新建」/ open project).
    awaiting_project_choice: bool,
    /// Empty-state「最近对话」section expanded beyond the default 3 rows.
    recent_inbox_expanded: bool,
    /// Next task created via [`Self::ensure_task_for_send`] came from top-level「新建对话」.
    pending_from_new_chat: bool,
    /// Plugins page: Plugins | Skills tab.
    plugins_subnav: PluginsSubNav,
    /// Local filter for the plugins/skills store.
    plugins_search: String,
    /// Selected catalog card → show detail (existing config UI).
    plugins_selected: Option<PluginCatalogId>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
enum PluginsSubNav {
    #[default]
    Plugins,
    Skills,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum PluginCatalogId {
    Unity,
    OpenMontage,
    Bevy,
    SkillOpenMontage,
    SkillBevy,
}

#[derive(Clone, Copy)]
struct PluginCatalogEntry {
    id: PluginCatalogId,
    category_key: &'static str,
    title_key: &'static str,
    blurb_key: &'static str,
    glyph: SidebarGlyph,
    accent: Color32,
}

impl BonyBuildApp {
    pub fn new(cc: &eframe::CreationContext<'_>, mut config: BridgeConfig) -> Self {
        crate::fonts::install(&cc.egui_ctx);
        configure_style(&cc.egui_ctx);
        let (event_tx, event_rx) = mpsc::channel();
        let task_repo = SqliteTaskRepository::open_default().ok();
        let mut tasks = task_repo
            .as_ref()
            .and_then(|r| r.list(true).ok())
            .unwrap_or_default();
        let mut active = tasks
            .iter()
            .find(|t| {
                t.status != TaskStatus::Archived
                    && t.project_path == config.cwd
                    && t.session_id.is_some()
            })
            .cloned();
        let mut init_error = None;
        if active.is_none() {
            let project = GitWorkspaceService::primary_repo_root(&config.cwd)
                .ok()
                .flatten()
                .unwrap_or_else(|| config.cwd.clone());
            let mut task = TaskState::draft(project.clone(), String::new());
            match GitWorkspaceService::create_worktree(&project, &task.id, &task.title) {
                Ok(worktree) => {
                    task.worktree_path = worktree.path;
                    task.branch = Some(worktree.branch);
                    task.isolated = true;
                }
                Err(e)
                    if GitWorkspaceService::repo_root(&project)
                        .ok()
                        .flatten()
                        .is_some() =>
                {
                    init_error = Some(format!(
                        "初始任务无法创建 worktree：{e}。当前任务使用共享目录，发送前请确认。"
                    ))
                }
                Err(_) => {}
            }
            if let Some(repo) = &task_repo {
                let _ = repo.save(&task);
            }
            tasks.insert(0, task.clone());
            active = Some(task);
        }
        if let Some(task) = active.as_ref() {
            config.cwd = task.worktree_path.clone();
            config.resume_session_id = task.session_id.clone();
        }
        let active_task_id = active.as_ref().map(|t| t.id.clone());
        let project_root = canonical_project_root(&config.cwd);
        let mut model = AppModel::new(config.cwd.clone());
        // Collapse worktree short-id folders into primary project roots.
        let mut normalized = Vec::new();
        for path in std::mem::take(&mut model.recent_projects) {
            remember_project(&mut normalized, &canonical_project_root(&path));
        }
        model.recent_projects = normalized;
        remember_project(&mut model.recent_projects, &project_root);
        let prefs = load_plugin_prefs();
        let openmontage = OpenMontageState::from_prefs(&prefs);
        let bevy = BevyState::from_prefs(&prefs);
        Self {
            model,
            event_rx,
            event_tx,
            cmd_tx: None,
            started: false,
            config,
            task_repo,
            tasks,
            active_task_id,
            attachments: Vec::new(),
            changes: Vec::new(),
            selected_diff: None,
            task_error: init_error,
            pending_git_action: None,
            task_list_filter: TaskListFilter::All,
            rename_task: None,
            delete_task: None,
            unity: UnityState::default(),
            openmontage,
            bevy,
            pending_unity_chat: None,
            pending_unity_planner: false,
            pending_unity_approval: None,
            // Fresh conversation: no plugins until user adds via 「+」.
            unity_chat_mode: false,
            unity_docs_open: false,
            show_unity_docs_window: false,
            show_composer_plus: false,
            composer_plus_just_opened: false,
            composer_plus_anchor: None,
            user_menu_just_opened: false,
            user_menu_anchor: None,
            plugin_prefs: prefs,
            ui_prefs: load_ui_prefs(),
            collapsed_projects: std::collections::HashSet::new(),
            expanded_archived: std::collections::HashSet::new(),
            stop_armed_force: false,
            pending_worktree_rx: None,
            bevy_new_project_name: "my-game".into(),
            window_dragging: false,
            window_drag_grab: None,
            sidebar_groups_cache: None,
            delete_modal_ignore_click: false,
            awaiting_project_choice: false,
            recent_inbox_expanded: false,
            pending_from_new_chat: false,
            plugins_subnav: PluginsSubNav::Plugins,
            plugins_search: String::new(),
            plugins_selected: None,
        }
    }

    #[inline]
    fn lang(&self) -> Language {
        self.ui_prefs.language
    }

    #[inline]
    fn t<'a>(&'a self, key: &'a str) -> &'a str {
        i18n::t(self.lang(), key)
    }

    fn set_language(&mut self, lang: Language) {
        if self.ui_prefs.language == lang {
            return;
        }
        self.ui_prefs.language = lang;
        save_ui_prefs(&self.ui_prefs);
    }

    fn ensure_started(&mut self, ctx: &egui::Context) {
        if self.started {
            return;
        }
        self.started = true;
        let cmd_tx =
            agent_bridge::spawn_bridge(self.config.clone(), ctx.clone(), self.event_tx.clone());
        self.cmd_tx = Some(cmd_tx);
    }

    /// Shut down the agent and reconnect against a new working directory.
    fn switch_project(&mut self, ctx: &egui::Context, path: std::path::PathBuf) {
        let root = canonical_project_root(&path);
        let keep_new_chat = self
            .active_task_id
            .as_ref()
            .and_then(|id| self.tasks.iter().find(|t| &t.id == id))
            .is_some_and(|t| t.from_new_chat);
        let same = self
            .config
            .cwd
            .canonicalize()
            .ok()
            .zip(path.canonicalize().ok())
            .is_some_and(|(a, b)| a == b)
            || canonical_project_root(
                &self.model.cwd.clone().unwrap_or_else(|| self.config.cwd.clone()),
            ) == root;
        if same {
            // User explicitly clicked this project — accept it as the choice.
            self.awaiting_project_choice = false;
            if keep_new_chat {
                self.bind_active_new_chat_to_project(&root, &path);
            }
            self.sidebar_groups_cache = None;
            self.model.go_chat();
            return;
        }
        // Soft handoff — keep chrome (sidebar, models, recent list); only clear chat.
        self.send_cmd(UiCommand::Shutdown);
        self.cmd_tx = None;
        self.config.cwd = path.clone();
        self.config.resume_session_id = None;
        self.awaiting_project_choice = false;
        if keep_new_chat {
            self.bind_active_new_chat_to_project(&root, &path);
        } else {
            self.active_task_id = None;
        }
        remember_project(&mut self.model.recent_projects, &root);
        self.model.cwd = Some(path);
        self.model.connected = false;
        self.model.session_id = None;
        self.model.needs_login = false;
        self.model.new_task();
        self.model.status = "Connecting…".into();
        self.model.usage = crate::usage::SessionUsageState::default();
        self.clear_session_plugins();
        let cmd_tx =
            agent_bridge::spawn_bridge(self.config.clone(), ctx.clone(), self.event_tx.clone());
        self.cmd_tx = Some(cmd_tx);
        self.started = true;
        ctx.request_repaint();
    }

    fn pick_project(&mut self, ctx: &egui::Context) {
        let start = self
            .model
            .cwd
            .clone()
            .unwrap_or_else(|| self.config.cwd.clone());
        if let Some(path) = rfd::FileDialog::new()
            .set_title("选择项目文件夹")
            .set_directory(start)
            .pick_folder()
        {
            self.switch_project(ctx, path);
        }
    }

    /// Pick a Unity project root for the Unity CLI panel only (does not switch agent cwd).
    fn pick_unity_project(&mut self, _ctx: &egui::Context) {
        let start = if self.unity.project_path.is_dir() {
            self.unity.project_path.clone()
        } else {
            self.config.cwd.clone()
        };
        if let Some(path) = rfd::FileDialog::new()
            .set_title("选择 Unity 工程根目录（含 Assets）")
            .set_directory(start)
            .pick_folder()
        {
            self.unity.set_project_path(path);
            if crate::unity::is_unity_project_root(&self.unity.project_path) {
                self.unity.toast = Some("已绑定 Unity 工程，可继续安装 Pipeline / 探测".into());
            } else {
                self.unity.toast = Some(
                    "所选目录不像 Unity 工程根：请选含 Assets + ProjectSettings 的文件夹".into(),
                );
            }
        }
    }

    fn drain_events(&mut self) {
        while let Ok(ev) = self.event_rx.try_recv() {
            let finish_unity_planner = self.pending_unity_planner
                && matches!(&ev, AgentEvent::TurnDone { .. } | AgentEvent::Error(_));
            if matches!(
                ev,
                AgentEvent::TurnDone { .. }
                    | AgentEvent::Error(_)
                    | AgentEvent::Disconnected
                    | AgentEvent::NeedsLogin { .. }
                    | AgentEvent::Connected { .. }
            ) {
                self.stop_armed_force = false;
            }
            self.persist_event(&ev);
            self.model.apply(ev);
            if finish_unity_planner {
                self.finish_unity_planner();
            }
        }
    }

    fn finish_unity_planner(&mut self) {
        self.pending_unity_planner = false;
        let raw = self.model.latest_assistant_text();
        match parse_generated_unity_plan_unrestricted(&raw) {
            Ok((summary, csharp, risks)) => {
                let mode = self
                    .active_task_id
                    .as_ref()
                    .and_then(|id| self.tasks.iter().find(|task| &task.id == id))
                    .map(|task| task.permission_mode)
                    .unwrap_or(PermissionMode::Ask);
                if mode == PermissionMode::ReadOnly {
                    self.model.replace_latest_assistant(format!(
                        "Unity 计划已生成：{summary}\n\n当前任务是只读模式，没有执行修改。"
                    ));
                } else if mode == PermissionMode::Ask || !risks.is_empty() {
                    self.model
                        .replace_latest_assistant(format!("Unity 计划等待批准：{summary}"));
                    self.pending_unity_approval = Some(PendingUnityApproval {
                        summary,
                        csharp,
                        risks,
                    });
                    self.model.busy = false;
                    self.model.status = "等待 Unity 权限确认".into();
                } else {
                    self.execute_unity_plan(summary, csharp);
                }
            }
            Err(error) => {
                self.model.replace_latest_assistant(format!(
                    "无法生成可执行的 Unity 计划：{error}\n\n没有执行任何编辑器操作。"
                ));
                self.model.busy = false;
                self.model.status = "Unity 计划被拒绝".into();
            }
        }
    }

    fn execute_unity_plan(&mut self, summary: String, csharp: String) {
        self.model
            .replace_latest_assistant(format!("Unity 计划已批准：{summary}\n\n正在执行…"));
        self.unity.eval_input = csharp;
        self.pending_unity_chat = Some(self.unity.latest_record_id());
        self.model.busy = true;
        self.model.status = "正在执行 Unity 计划…".into();
        self.unity.run_action(UnityAction::Eval);
    }

    fn persist_event(&mut self, event: &AgentEvent) {
        let Some(id) = self.active_task_id.clone() else {
            return;
        };
        let Some(task) = self.tasks.iter_mut().find(|t| t.id == id) else {
            return;
        };
        let mut soft_cancel_restored = false;
        match event {
            AgentEvent::Connected {
                session_id,
                current_model_id,
                restored,
                ..
            } => {
                task.session_id = Some(session_id.clone());
                task.model_id = current_model_id.clone();
                task.status = TaskStatus::Draft;
                if *restored {
                    // Soft-cancel only — ForceStop here caused an infinite reconnect loop
                    // because resume_session_id stays set and every Connected re-triggers it.
                    self.model.busy = false;
                    self.stop_armed_force = false;
                    self.model.status = "会话已恢复".into();
                    soft_cancel_restored = true;
                    // Next reconnects in this process should not look like a fresh restore.
                    self.config.resume_session_id = None;
                }
            }
            AgentEvent::PermissionRequest { .. } => task.status = TaskStatus::WaitingApproval,
            AgentEvent::TurnDone { .. } => {
                task.status = TaskStatus::Completed;
                self.changes =
                    GitWorkspaceService::changes(&task.worktree_path).unwrap_or_default();
            }
            AgentEvent::Error(_) => task.status = TaskStatus::Failed,
            _ => return,
        }
        task.updated_at = unix_time();
        if let Some(repo) = &self.task_repo {
            let _ = repo.save(task);
        }
        if soft_cancel_restored {
            self.send_cmd(UiCommand::Cancel);
        }
    }

    fn create_task(&mut self, _ctx: &egui::Context) {
        // Top-level「新建对话」: create an inbox draft immediately (最近对话),
        // with no project selected in the sidebar tree.
        self.pending_worktree_rx = None;
        self.awaiting_project_choice = true;
        self.recent_inbox_expanded = false;
        self.pending_from_new_chat = false;
        self.attachments.clear();
        self.changes.clear();
        self.selected_diff = None;
        self.clear_session_plugins();

        let provisional = canonical_project_root(
            &self
                .model
                .cwd
                .clone()
                .unwrap_or_else(|| self.config.cwd.clone()),
        );
        let mut task = TaskState::draft(provisional.clone(), self.model.current_model_id.clone());
        task.worktree_path = provisional;
        task.isolated = false;
        task.from_new_chat = true;
        if let Some(repo) = &self.task_repo {
            if let Err(e) = repo.save(&task) {
                self.task_error = Some(e);
                return;
            }
        }
        self.active_task_id = Some(task.id.clone());
        self.model.new_task();
        self.model.task_title = task.title.clone();
        self.model.focus_composer = true;
        self.tasks.insert(0, task);
        self.sidebar_groups_cache = None;
        if self.model.connected && !self.model.needs_login {
            self.model.status = "Ready".into();
        }
    }

    /// Create a sidebar task under the current project when sending — but only
    /// after the user has chosen a project. Never invent a default project.
    fn ensure_task_for_send(&mut self) {
        if self.active_task_id.is_some() || self.awaiting_project_choice {
            return;
        }
        let project = canonical_project_root(
            &self
                .model
                .cwd
                .clone()
                .unwrap_or_else(|| self.config.cwd.clone()),
        );
        let project = GitWorkspaceService::primary_repo_root(&project)
            .ok()
            .flatten()
            .unwrap_or(project);
        let mut task = TaskState::draft(project.clone(), self.model.current_model_id.clone());
        task.worktree_path = self
            .model
            .cwd
            .clone()
            .unwrap_or_else(|| project.clone());
        task.isolated = false;
        task.from_new_chat = self.pending_from_new_chat;
        self.pending_from_new_chat = false;
        if let Some(repo) = &self.task_repo {
            if let Err(e) = repo.save(&task) {
                self.task_error = Some(e);
                return;
            }
        }
        self.active_task_id = Some(task.id.clone());
        self.model.task_title = task.title.clone();
        self.tasks.insert(0, task);
        self.sidebar_groups_cache = None;
    }

    /// Point the active top-level「新建对话」draft at a chosen project root.
    fn bind_active_new_chat_to_project(
        &mut self,
        root: &std::path::Path,
        worktree: &std::path::Path,
    ) {
        let Some(id) = self.active_task_id.clone() else {
            return;
        };
        let Some(task) = self
            .tasks
            .iter_mut()
            .find(|t| t.id == id && t.from_new_chat)
        else {
            return;
        };
        task.project_path = root.to_path_buf();
        task.worktree_path = worktree.to_path_buf();
        task.updated_at = unix_time();
        if let Some(repo) = &self.task_repo {
            let _ = repo.save(task);
        }
        self.sidebar_groups_cache = None;
    }

    /// Drop an unused inbox draft when the user switches to per-project「新建」.
    fn discard_unused_new_chat_draft(&mut self) {
        let Some(id) = self.active_task_id.clone() else {
            return;
        };
        let Some(pos) = self.tasks.iter().position(|t| {
            t.id == id && t.from_new_chat && t.session_id.is_none() && is_placeholder_task_title(&t.title)
        }) else {
            return;
        };
        let id = self.tasks.remove(pos).id;
        if let Some(repo) = &self.task_repo {
            let _ = repo.delete(&id);
        }
        self.active_task_id = None;
        self.sidebar_groups_cache = None;
    }

    /// Create a new task under an explicit project root (used by per-project 「新建」).
    fn create_task_for(&mut self, ctx: &egui::Context, project: std::path::PathBuf) {
        if self.pending_worktree_rx.is_some() {
            self.model.status = "正在创建上一个工作区，请稍候…".into();
            return;
        }
        // Per-project「新建」is never an inbox / top-level New chat entry.
        self.pending_from_new_chat = false;
        self.discard_unused_new_chat_draft();
        let project = GitWorkspaceService::primary_repo_root(&project)
            .ok()
            .flatten()
            .unwrap_or(project);
        let mut task = TaskState::draft(project.clone(), self.model.current_model_id.clone());
        // Placeholder cwd so chat UI opens immediately; real worktree path
        // arrives from the background job below.
        task.worktree_path = project.clone();
        task.isolated = false;
        task.from_new_chat = false;
        if let Some(repo) = &self.task_repo {
            if let Err(e) = repo.save(&task) {
                self.task_error = Some(e);
                return;
            }
        }
        let task_id = task.id.clone();
        let title = task.title.clone();
        self.tasks.insert(0, task.clone());
        self.sidebar_groups_cache = None;
        self.activate_task_ui_only(task);
        self.model.status = "正在创建隔离工作区…".into();
        ctx.request_repaint();

        let (tx, rx) = mpsc::channel();
        self.pending_worktree_rx = Some(rx);
        std::thread::spawn(move || {
            let result = match GitWorkspaceService::create_worktree(&project, &task_id, &title) {
                Ok(w) => Ok((task_id, w.path, w.branch)),
                Err(e) => Err((task_id, e)),
            };
            let _ = tx.send(result);
        });
    }

    /// Create under `project_root` in one shot — do NOT switch_project first
    /// (that used to double-restart the agent bridge and flash like a slideshow).
    fn create_task_in_project(&mut self, ctx: &egui::Context, project_root: std::path::PathBuf) {
        self.create_task_for(ctx, canonical_project_root(&project_root));
    }

    /// Apply chat chrome for a task without (re)starting the agent bridge.
    fn activate_task_ui_only(&mut self, task: TaskState) {
        self.awaiting_project_choice = false;
        self.active_task_id = Some(task.id.clone());
        remember_project(
            &mut self.model.recent_projects,
            &canonical_project_root(&task.project_path),
        );
        self.model.cwd = Some(task.worktree_path.clone());
        self.model.connected = false;
        self.model.session_id = None;
        self.model.needs_login = false;
        self.model.new_task();
        self.model.task_title = task.title;
        self.model.status = "Connecting…".into();
        self.model.busy = false;
        self.stop_armed_force = false;
        self.attachments.clear();
        self.changes.clear();
        self.selected_diff = None;
        self.clear_session_plugins();
    }

    fn activate_task(&mut self, ctx: &egui::Context, task: TaskState) {
        // Already on this conversation — don't tear down the bridge.
        if self.active_task_id.as_ref() == Some(&task.id) {
            self.model.go_chat();
            self.model.return_to_live();
            return;
        }

        let same_cwd = self
            .config
            .cwd
            .canonicalize()
            .ok()
            .zip(task.worktree_path.canonicalize().ok())
            .is_some_and(|(a, b)| a == b);

        // Same worktree, flip conversation chrome only — keep agent alive for
        // fresh drafts that share a cwd and have no session to resume.
        if same_cwd
            && self.cmd_tx.is_some()
            && task.session_id.is_none()
            && self.config.resume_session_id.is_none()
        {
            self.awaiting_project_choice = false;
            self.active_task_id = Some(task.id.clone());
            remember_project(
                &mut self.model.recent_projects,
                &canonical_project_root(&task.project_path),
            );
            self.model.new_task();
            self.model.task_title = task.title;
            self.model.cwd = Some(task.worktree_path);
            self.model.go_chat();
            if self.model.connected && !self.model.needs_login {
                self.model.status = "Ready".into();
            }
            self.clear_session_plugins();
            ctx.request_repaint();
            return;
        }

        self.send_cmd(UiCommand::ForceStop);
        self.send_cmd(UiCommand::Shutdown);
        self.cmd_tx = None;
        self.config.cwd = task.worktree_path.clone();
        self.config.resume_session_id = task.session_id.clone();
        self.activate_task_ui_only(task);
        let tx =
            agent_bridge::spawn_bridge(self.config.clone(), ctx.clone(), self.event_tx.clone());
        self.cmd_tx = Some(tx);
        self.started = true;
        ctx.request_repaint();
    }

    fn poll_pending_worktree(&mut self, ctx: &egui::Context) {
        let Some(rx) = self.pending_worktree_rx.as_ref() else {
            return;
        };
        let Ok(result) = rx.try_recv() else {
            ctx.request_repaint_after(std::time::Duration::from_millis(50));
            return;
        };
        self.pending_worktree_rx = None;
        match result {
            Ok((task_id, path, branch)) => {
                if let Some(task) = self.tasks.iter_mut().find(|t| t.id == task_id) {
                    task.worktree_path = path;
                    task.branch = Some(branch);
                    task.isolated = true;
                    if let Some(repo) = &self.task_repo {
                        let _ = repo.save(task);
                    }
                    let task = task.clone();
                    if self.active_task_id.as_ref() == Some(&task_id) {
                        self.active_task_id = None; // force reconnect into real worktree
                        self.activate_task(ctx, task);
                        self.model.status = "隔离工作区已就绪".into();
                    }
                }
            }
            Err((task_id, err)) => {
                let is_git = self
                    .tasks
                    .iter()
                    .find(|t| t.id == task_id)
                    .and_then(|t| {
                        GitWorkspaceService::repo_root(&t.project_path)
                            .ok()
                            .flatten()
                    })
                    .is_some();
                if is_git {
                    self.tasks.retain(|t| t.id != task_id);
                    if self.active_task_id.as_ref() == Some(&task_id) {
                        self.active_task_id = None;
                    }
                    self.task_error = Some(format!(
                        "无法创建隔离 worktree：{err}\n任务未创建，避免静默共享工作目录。"
                    ));
                } else if self.active_task_id.as_ref() == Some(&task_id) {
                    if let Some(task) = self.tasks.iter().find(|t| t.id == task_id).cloned() {
                        self.active_task_id = None;
                        self.activate_task(ctx, task);
                    }
                }
            }
        }
    }

    fn maybe_autotitle_active_task(&mut self, text: &str) {
        let Some(id) = self.active_task_id.clone() else {
            return;
        };
        let Some(task) = self.tasks.iter_mut().find(|t| t.id == id) else {
            return;
        };
        if !is_placeholder_task_title(&task.title) {
            return;
        }
        let title = suggest_task_title(text);
        if title.is_empty() {
            return;
        }
        task.title = title;
        task.updated_at = unix_time();
        self.model.task_title = task.title.clone();
        if let Some(repo) = &self.task_repo {
            let _ = repo.save(task);
        }
    }

    fn send_cmd(&self, cmd: UiCommand) {
        if let Some(tx) = &self.cmd_tx {
            let _ = tx.send(cmd);
        }
    }

    fn send_prompt(&mut self) {
        let text = self.model.draft.trim().to_string();
        if self.try_send_unity_chat_command(&text) {
            self.model.draft.clear();
            return;
        }
        if (text.is_empty() && self.attachments.is_empty())
            || self.model.busy
            || self.model.needs_login
            || !self.model.connected
        {
            return;
        }
        let title_src = if text.is_empty() {
            format!("已附加 {} 个文件", self.attachments.len())
        } else {
            text.clone()
        };
        self.ensure_task_for_send();
        self.maybe_autotitle_active_task(&title_src);
        if let Some(id) = self.active_task_id.clone()
            && let Some(task) = self.tasks.iter_mut().find(|t| t.id == id)
        {
            task.status = TaskStatus::Running;
            task.updated_at = unix_time();
            if let Some(repo) = &self.task_repo {
                let _ = repo.save(task);
            }
        }
        self.model.draft.clear();
        self.model.push_user(if text.is_empty() {
            format!("已附加 {} 个文件", self.attachments.len())
        } else {
            text.clone()
        });
        let attachments = std::mem::take(&mut self.attachments);
        self.send_cmd(UiCommand::Prompt { text, attachments });
    }

    fn try_send_unity_chat_command(&mut self, text: &str) -> bool {
        if text.is_empty() || !self.attachments.is_empty() {
            return false;
        }
        let looks_unity = wants_unity_help(text)
            || parse_unity_chat_command(text).is_some()
            || compile_unity_scene_command(text).is_some()
            || (self.unity_chat_mode
                && text
                    .trim()
                    .to_ascii_lowercase()
                    .starts_with("/unity"));
        if looks_unity && !self.plugin_prefs.unity_enabled {
            self.model.push_local_user(text.to_string());
            self.model.push_local_assistant(
                "Unity 控制插件未启用。请打开侧栏「插件」，启用后再使用。".into(),
            );
            return true;
        }
        if wants_unity_help(text) {
            self.maybe_autotitle_active_task("Unity 说明");
            self.model.push_local_user(text.to_string());
            self.show_unity_docs_window = true;
            return true;
        }
        if let Some((label, eval)) = compile_unity_scene_command(text) {
            self.maybe_autotitle_active_task(&label);
            self.model.go_chat();
            self.set_chat_interaction(ChatInteraction::Unity);
            self.model.push_local_user(text.to_string());
            if self.unity.busy || self.unity.is_guiding() {
                self.model
                    .push_local_assistant("Unity 正在执行其他操作，请完成后再试。".into());
                return true;
            }
            let mode = self
                .active_task_id
                .as_ref()
                .and_then(|id| self.tasks.iter().find(|task| &task.id == id))
                .map(|task| task.permission_mode)
                .unwrap_or(PermissionMode::Ask);
            if mode == PermissionMode::ReadOnly {
                self.model
                    .push_local_assistant("当前任务是只读模式，没有执行 Unity 场景修改。".into());
                return true;
            }
            if mode == PermissionMode::Ask {
                self.pending_unity_approval = Some(PendingUnityApproval {
                    summary: label.clone(),
                    csharp: eval,
                    risks: Vec::new(),
                });
                self.model.busy = false;
                self.model.status = "等待 Unity 权限确认".into();
                return true;
            }
            self.unity.eval_input = eval;
            self.pending_unity_chat = Some(self.unity.latest_record_id());
            self.model.status = format!("正在控制 Unity：{label}");
            self.unity.run_action(UnityAction::Eval);
            return true;
        }
        let Some(cmd) = parse_unity_chat_command(text) else {
            if self.unity_chat_mode {
                if !self.model.connected || self.model.needs_login {
                    self.model.push_local_user(text.to_string());
                    self.model.push_local_assistant(
                        "通用 Unity 操作需要 Agent 生成结构化计划，请先连接 Agent。".into(),
                    );
                    return true;
                }
                self.model.push_user(text.to_string());
                self.pending_unity_planner = true;
                let planner_prompt = format!(
                    "You are a Unity Editor action compiler. Convert the user's request below into one Unity C# Eval body. Do not call tools. Output ONLY one JSON object with string fields `summary` and `csharp`, without markdown. The C# runs inside the open Unity Editor and must use UnityEngine/UnityEditor APIs, support Undo for mutations, mark changed scenes/assets dirty, and end with a return value. Never use filesystem, network, processes, environment variables, native interop, reflection, or shell APIs. User request: {}",
                    text
                );
                self.send_cmd(UiCommand::Prompt {
                    text: planner_prompt,
                    attachments: Vec::new(),
                });
                return true;
            }
            return false;
        };
        self.dispatch_unity_chat_cmd(cmd, Some(text));
        true
    }

    fn dispatch_unity_chat_cmd(&mut self, cmd: &UnityChatCmd, spoken: Option<&str>) {
        let label = spoken.unwrap_or(cmd.chip).to_string();
        self.maybe_autotitle_active_task(cmd.chip);
        self.model.go_chat();
        self.set_chat_interaction(ChatInteraction::Unity);
        self.model.push_local_user(label);
        if self.unity.busy || self.unity.is_guiding() {
            self.model
                .push_local_assistant("Unity 正在执行其他操作，请完成后再试。".into());
            return;
        }
        if let Some(expression) = cmd.eval {
            self.unity.eval_input = expression.to_string();
        }
        self.pending_unity_chat = Some(self.unity.latest_record_id());
        self.unity.run_action(cmd.action);
    }

    fn pick_attachments(&mut self) {
        let Some(paths) = rfd::FileDialog::new()
            .set_title("添加上下文文件")
            .pick_files()
        else {
            return;
        };
        for path in paths {
            let Ok(meta) = std::fs::metadata(&path) else {
                continue;
            };
            if meta.len() > 10 * 1024 * 1024 {
                self.task_error = Some(format!("附件超过 10 MB：{}", path.display()));
                continue;
            }
            let ext = path
                .extension()
                .and_then(|v| v.to_str())
                .unwrap_or("")
                .to_ascii_lowercase();
            let mime = match ext.as_str() {
                "png" => "image/png",
                "jpg" | "jpeg" => "image/jpeg",
                "gif" => "image/gif",
                "webp" => "image/webp",
                "txt" | "md" | "rs" | "toml" | "json" | "yaml" | "yml" | "js" | "ts" | "tsx"
                | "jsx" | "py" | "go" | "java" | "c" | "cpp" | "h" => "text/plain",
                _ => {
                    self.task_error = Some(format!("暂不支持的附件类型：{}", path.display()));
                    continue;
                }
            };
            if let Ok(data) = std::fs::read(&path) {
                self.attachments.push(AttachmentPayload {
                    name: path
                        .file_name()
                        .unwrap_or_default()
                        .to_string_lossy()
                        .into_owned(),
                    mime_type: mime.into(),
                    data,
                });
            }
        }
    }

    fn send_context_prompt(&mut self, display_text: &str, prompt: String) {
        if self.model.busy || !self.model.connected || self.model.needs_login {
            return;
        }
        self.model.draft.clear();
        self.model.push_user(display_text.to_string());
        self.send_cmd(UiCommand::Prompt {
            text: prompt,
            attachments: Vec::new(),
        });
    }
}

impl eframe::App for BonyBuildApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.ensure_started(ctx);
        self.drain_events();
        self.poll_pending_worktree(ctx);
        self.tick_window_drag(ctx);

        // Lightweight frame while dragging: title chrome only. Full UI tessellation
        // on every mouse-move is what makes OuterPosition drags feel "jumpy".
        if self.window_dragging {
            self.title_bar(ctx);
            egui::CentralPanel::default()
                .frame(Frame::NONE.fill(BG))
                .show(ctx, |_| {});
            return;
        }

        // Unity CLI detection once; cwd binding is cached inside consider_agent_cwd
        // (was: canonicalize + directory walk every frame → slideshow-level jank).
        if self.plugin_prefs.unity_enabled {
            self.unity.ensure_detecting();
            self.unity.consider_agent_cwd(&self.config.cwd);
            self.unity.sync_setup_step();
        }
        let unity_changed = if self.plugin_prefs.unity_enabled {
            self.unity.poll()
        } else {
            false
        };
        if self.plugin_prefs.unity_enabled
            && (unity_changed || self.unity.needs_repaint())
        {
            ctx.request_repaint_after(std::time::Duration::from_millis(40));
        }
        if self.plugin_prefs.unity_enabled
            && self.pending_unity_chat.is_some()
            && unity_changed
            && !self.unity.busy
            && !self.unity.is_guiding()
        {
            let previous_id = self.pending_unity_chat.take().unwrap_or_default();
            self.model
                .push_local_assistant(self.unity.latest_chat_result_since(previous_id));
        }
        if self.plugin_prefs.unity_enabled {
            if let Some(toast) = self.unity.take_toast() {
                self.model.status = toast;
            }
        }

        self.openmontage.ensure_checked();
        if self.openmontage.poll() || self.openmontage.busy {
            ctx.request_repaint_after(std::time::Duration::from_millis(40));
        }
        if matches!(
            self.openmontage.status,
            OpenMontageStatus::Ready | OpenMontageStatus::MissingDeps(_)
        ) && self.plugin_prefs.openmontage_root.as_ref() != Some(&self.openmontage.root)
        {
            self.plugin_prefs.openmontage_root = Some(self.openmontage.root.clone());
            save_plugin_prefs(&self.plugin_prefs);
        }
        if let Some(toast) = self.openmontage.take_toast() {
            self.model.status = toast;
        }

        self.bevy.ensure_checked();
        if self.bevy.poll() || self.bevy.busy {
            ctx.request_repaint_after(std::time::Duration::from_millis(40));
        }
        if matches!(self.bevy.status, BevyStatus::Ready)
            && self.plugin_prefs.bevy_project_root.as_ref() != Some(&self.bevy.project_path)
        {
            self.plugin_prefs.bevy_project_root = Some(self.bevy.project_path.clone());
            save_plugin_prefs(&self.plugin_prefs);
        }
        if let Some(toast) = self.bevy.take_toast() {
            self.model.status = toast;
        }

        if self.model.busy {
            ctx.request_repaint_after(std::time::Duration::from_millis(40));
        }

        self.title_bar(ctx);

        if self.model.show_left_sidebar {
            egui::SidePanel::left("codex_sidebar")
                .exact_width(SIDEBAR_W)
                .resizable(false)
                .frame(Frame::NONE.fill(SIDEBAR).inner_margin(Margin {
                    left: 12,
                    right: 12,
                    top: 10,
                    bottom: 12,
                }))
                .show(ctx, |ui| {
                    self.sidebar(ui, ctx);
                });
        }

        if self.model.show_right_panel {
            egui::SidePanel::right("codex_right")
                .exact_width(RIGHT_PANEL_W)
                .resizable(false)
                .frame(
                    Frame::NONE
                        .fill(SIDEBAR)
                        .inner_margin(Margin::symmetric(14, 14))
                        .stroke(Stroke::new(1.0, BORDER)),
                )
                .show(ctx, |ui| {
                    self.right_panel(ui);
                });
        }

        let on_chat = self.model.main_nav == MainNav::Chat;
        let on_unity = self.model.main_nav == MainNav::Unity;
        let on_plugins = self.model.main_nav == MainNav::Plugins;
        // Unity settings page is only reachable via the plugins manager.
        if on_unity && !self.plugin_prefs.unity_enabled {
            self.model.main_nav = MainNav::Plugins;
        }
        let on_unity = self.model.main_nav == MainNav::Unity;
        let show_task_title =
            on_chat && (!self.model.is_empty_chat() || self.model.is_viewing_history());
        // Plugins page owns its own Plugins|Skills chrome — skip the duplicate「插件」title.
        let show_nav_title = !on_chat && !on_plugins;

        egui::CentralPanel::default()
            .frame(Frame::NONE.fill(BG).inner_margin(Margin::symmetric(0, 0)))
            .show(ctx, |ui| {
                // No second control row under the window buttons — title only when needed.
                if show_task_title || show_nav_title {
                    ui.add_space(10.0);
                    ui.horizontal(|ui| {
                        ui.add_space(20.0);
                        let title = if on_chat {
                            if self.model.task_title.contains("只读分析") {
                                "Unity 状态分析"
                            } else {
                                self.model.task_title.as_str()
                            }
                        } else {
                            self.model.main_nav.title_lang(self.lang())
                        };
                        ui.label(RichText::new(title).size(14.0).strong().color(TEXT));
                        if on_unity {
                            ui.add_space(8.0);
                            let status_color = match self.unity.status {
                                CliStatus::Ready => OK,
                                CliStatus::Missing | CliStatus::Error => DANGER,
                                CliStatus::Checking | CliStatus::Unknown | CliStatus::Installing => {
                                    MUTED
                                }
                            };
                            ui.label(
                                RichText::new(self.unity.status.label())
                                    .size(12.0)
                                    .color(status_color),
                            );
                            if self.unity.demo_mode {
                                ui.label(
                                    RichText::new(self.t("common.demo_mode"))
                                        .size(12.0)
                                        .color(UNITY_ACCENT),
                                );
                            }
                        }
                    });
                }

                if on_chat {
                    // Size composer to its content first so Unity chips / shortcuts
                    // never clip the + / model / send row at the bottom.
                    egui::TopBottomPanel::bottom("chat_composer")
                        .frame(egui::Frame::NONE)
                        .show_separator_line(false)
                        .show_inside(ui, |ui| {
                            ui.add_space(8.0);
                            centered_column(ui, |ui| {
                                self.floating_composer(ui);
                            });
                            ui.add_space(12.0);
                        });

                    egui::ScrollArea::vertical()
                        .id_salt("chat_scroll")
                        .stick_to_bottom(self.model.auto_scroll)
                        .auto_shrink([false, false])
                        .show(ui, |ui| {
                            centered_column(ui, |ui| {
                                if self.model.is_viewing_history() {
                                    ui.add_space(8.0);
                                    ui.vertical_centered(|ui| {
                                        ui.label(
                                            RichText::new(self.t("composer.readonly_history"))
                                                .size(12.0)
                                                .color(MUTED),
                                        );
                                    });
                                    ui.add_space(8.0);
                                }
                                if self.model.is_empty_chat() {
                                    self.empty_state(ui, ctx);
                                } else {
                                    self.timeline(ui);
                                }
                                if self.model.busy {
                                    ui.add_space(8.0);
                                    ui.horizontal(|ui| {
                                        ui.spinner();
                                        ui.label(
                                            RichText::new(self.t("composer.processing"))
                                                .size(12.5)
                                                .color(MUTED),
                                        );
                                    });
                                }
                                ui.add_space(20.0);
                            });
                        });
                } else if on_plugins {
                    egui::ScrollArea::vertical()
                        .id_salt("plugins_scroll")
                        .auto_shrink([false, false])
                        .show(ui, |ui| {
                            // Full-bleed store — don't reuse the narrow chat column.
                            let pad = 28.0;
                            ui.add_space(4.0);
                            ui.horizontal(|ui| {
                                ui.add_space(pad);
                                // Leave matching right inset so actions aren't flush to the pane edge.
                                let w = (ui.available_width() - pad).max(320.0);
                                ui.allocate_ui_with_layout(
                                    Vec2::new(w, ui.available_height()),
                                    egui::Layout::top_down(egui::Align::Min)
                                        .with_cross_justify(true),
                                    |ui| {
                                        ui.set_width(w);
                                        self.plugins_panel(ui);
                                    },
                                );
                            });
                        });
                } else if on_unity {
                    egui::ScrollArea::vertical()
                        .id_salt("unity_scroll")
                        .auto_shrink([false, false])
                        .show(ui, |ui| {
                            centered_column(ui, |ui| {
                                self.unity_panel(ui);
                            });
                        });
                } else {
                    centered_column(ui, |ui| {
                        self.nav_placeholder(ui);
                    });
                }
            });

        self.user_menu_popup(ctx);
        self.composer_plus_popup(ctx);
        self.usage_detail_window(ctx);
        self.unity_docs_window(ctx);
        self.permission_modal(ctx);
        self.unity_permission_modal(ctx);
        self.model_picker_modal(ctx);
        self.about_modal(ctx);
        self.rename_task_modal(ctx);
        self.delete_task_modal(ctx);
        self.task_error_modal(ctx);
        self.git_confirmation_modal(ctx);
    }

    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        self.send_cmd(UiCommand::Shutdown);
    }
}

impl BonyBuildApp {
    /// One-row Codex chrome: left controls + menus | drag | right toggle + window buttons.
    fn title_bar(&mut self, ctx: &egui::Context) {
        egui::TopBottomPanel::top("title_bar")
            .exact_height(TITLE_BAR_H)
            .frame(Frame::NONE.fill(SIDEBAR).inner_margin(Margin {
                left: 8,
                right: 0,
                top: 0,
                bottom: 0,
            }))
            .show(ctx, |ui| {
                let full = ui.max_rect();
                ui.painter()
                    .hline(full.x_range(), full.bottom(), Stroke::new(1.0, BORDER));

                ui.allocate_ui_with_layout(
                    Vec2::new(full.width(), TITLE_BAR_H),
                    egui::Layout::left_to_right(egui::Align::Center),
                    |ui| {
                        // —— Left cluster ——
                        let left_on = self.model.show_left_sidebar;
                        if panel_toggle_btn(ui, PanelSide::Left, self.t("tip.toggle_sidebar"), left_on)
                        {
                            self.model.show_left_sidebar = !left_on;
                        }
                        let can_back = self.model.is_viewing_history();
                        if nav_chevron_btn(ui, NavDir::Back, self.t("tip.back_live"), can_back)
                            && can_back
                        {
                            self.model.return_to_live();
                        }
                        let _ = nav_chevron_btn(ui, NavDir::Forward, self.t("tip.forward"), false);

                        ui.add_space(8.0);
                        self.project_chip(ui, ctx);

                        ui.add_space(8.0);
                        ui.spacing_mut().button_padding = Vec2::new(6.0, 2.0);
                        ui.visuals_mut().button_frame = false;
                        for (label_key, build) in [
                            ("menu.file", 0u8),
                            ("menu.edit", 1u8),
                            ("menu.view", 2u8),
                            ("menu.help", 3u8),
                        ] {
                            let label = self.t(label_key);
                            ui.menu_button(RichText::new(label).size(13.0).color(MUTED), |ui| {
                                ui.visuals_mut().button_frame = true;
                                match build {
                                    0 => {
                                        if ui.button(self.t("menu.new_task")).clicked() {
                                            self.create_task(ctx);
                                            ui.close_menu();
                                        }
                                        if ui.button(self.t("menu.open_project")).clicked() {
                                            self.pick_project(ctx);
                                            ui.close_menu();
                                        }
                                        ui.separator();
                                        if ui.button(self.t("menu.quit")).clicked() {
                                            ui.close_menu();
                                            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                                        }
                                    }
                                    1 => {
                                        if ui.button(self.t("menu.focus_composer")).clicked() {
                                            self.model.go_chat();
                                            self.model.focus_composer = true;
                                            ui.close_menu();
                                        }
                                        if ui.button(self.t("menu.clear_draft")).clicked() {
                                            self.model.draft.clear();
                                            ui.close_menu();
                                        }
                                    }
                                    2 => {
                                        let left_label = if self.model.show_left_sidebar {
                                            self.t("menu.hide_sidebar")
                                        } else {
                                            self.t("menu.show_sidebar")
                                        };
                                        if ui.button(left_label).clicked() {
                                            self.model.show_left_sidebar =
                                                !self.model.show_left_sidebar;
                                            ui.close_menu();
                                        }
                                        let right_label = if self.model.show_right_panel {
                                            self.t("menu.hide_right")
                                        } else {
                                            self.t("menu.show_right")
                                        };
                                        if ui.button(right_label).clicked() {
                                            self.model.show_right_panel =
                                                !self.model.show_right_panel;
                                            ui.close_menu();
                                        }
                                        if ui.button(self.t("menu.usage")).clicked() {
                                            self.model.show_usage_detail = true;
                                            ui.close_menu();
                                        }
                                        if ui.button(self.t("menu.plugins")).clicked() {
                                            self.model.main_nav = MainNav::Plugins;
                                            ui.close_menu();
                                        }
                                    }
                                    _ => {
                                        if ui.button(self.t("menu.about")).clicked() {
                                            self.model.show_about = true;
                                            ui.close_menu();
                                        }
                                    }
                                }
                            });
                        }

                        // —— Right cluster + middle drag strip ——
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if win_chrome_btn(ui, WinChrome::Close) {
                                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                            }
                            let maximized = ctx.input(|i| i.viewport().maximized.unwrap_or(false));
                            if win_chrome_btn(
                                ui,
                                if maximized {
                                    WinChrome::Restore
                                } else {
                                    WinChrome::Maximize
                                },
                            ) {
                                ctx.send_viewport_cmd(egui::ViewportCommand::Maximized(!maximized));
                            }
                            if win_chrome_btn(ui, WinChrome::Minimize) {
                                ctx.send_viewport_cmd(egui::ViewportCommand::Minimized(true));
                            }

                            ui.add_space(4.0);
                            let right_on = self.model.show_right_panel;
                            if panel_toggle_btn(
                                ui,
                                PanelSide::Right,
                                if right_on {
                                    self.t("tip.hide_right")
                                } else {
                                    self.t("tip.show_right")
                                },
                                right_on,
                            ) {
                                self.model.show_right_panel = !right_on;
                            }

                            // Explicitly own the remaining middle strip so hit-testing
                            // cannot fall through / get stolen by the panel background.
                            let drag_rect = ui.available_rect_before_wrap();
                            let drag_resp =
                                ui.allocate_rect(drag_rect, egui::Sense::click_and_drag());
                            self.on_title_drag_response(ctx, &drag_resp);
                        });
                    },
                );
            });
    }

    /// Persistent "which project am I in" indicator, visible from every page
    /// (chat / plugins / history) regardless of sidebar state. Click to
    /// switch, hover to see the full path.
    fn project_chip(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        let cwd = self.model.cwd.clone().unwrap_or_else(|| self.config.cwd.clone());
        let root = canonical_project_root(&cwd);
        let name = AppModel::project_label(&root);
        let full_path = root.display().to_string();
        let resp = Frame::new()
            .fill(PANEL_2)
            .corner_radius(CornerRadius::same(8))
            .stroke(Stroke::new(1.0, BORDER))
            .inner_margin(Margin::symmetric(8, 3))
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 5.0;
                    paint_sidebar_glyph(ui, SidebarGlyph::Folder, MUTED);
                    ui.label(RichText::new(name).size(12.5).color(TEXT).strong());
                });
            })
            .response
            .interact(egui::Sense::click())
            .on_hover_text(format!("{full_path}\n点击切换项目"));
        if resp.hovered() {
            ctx.set_cursor_icon(CursorIcon::PointingHand);
        }
        if resp.clicked() {
            self.pick_project(ctx);
        }
    }

    /// Start / cursor affordance for the title-bar drag strip.
    fn on_title_drag_response(&mut self, ctx: &egui::Context, resp: &egui::Response) {
        if resp.hovered() || self.window_dragging {
            ctx.set_cursor_icon(if self.window_dragging {
                CursorIcon::Grabbing
            } else {
                CursorIcon::Grab
            });
        }

        if resp.double_clicked() {
            let maximized = ctx.input(|i| i.viewport().maximized.unwrap_or(false));
            ctx.send_viewport_cmd(egui::ViewportCommand::Maximized(!maximized));
            self.window_dragging = false;
            self.window_drag_grab = None;
            return;
        }

        if resp.is_pointer_button_down_on() && ctx.input(|i| i.pointer.primary_pressed()) {
            if ctx.input(|i| i.viewport().maximized.unwrap_or(false)) {
                ctx.send_viewport_cmd(egui::ViewportCommand::Maximized(false));
            }
            let ppp = ctx.pixels_per_point().max(1e-3);
            let outer_min = ctx
                .input(|i| i.viewport().outer_rect.map(|r| r.min))
                .unwrap_or(Pos2::ZERO);
            // Prefer real screen cursor so grab offset stays stable across DPI.
            let grab = screen_cursor_pos_points(ppp)
                .map(|screen| screen - outer_min)
                .or_else(|| ctx.input(|i| i.pointer.latest_pos().map(|p| p.to_vec2())))
                .unwrap_or(Vec2::ZERO);
            self.window_drag_grab = Some(grab);
            self.window_dragging = true;
            ctx.request_repaint();
        }
    }

    /// Pin the window under the cursor via screen-absolute coordinates.
    ///
    /// Avoids `pointer.delta()` (fights the moving window) and avoids relying on
    /// raw device-motion unit scaling.
    fn tick_window_drag(&mut self, ctx: &egui::Context) {
        if !self.window_dragging {
            return;
        }
        if !ctx.input(|i| i.pointer.primary_down()) {
            self.window_dragging = false;
            self.window_drag_grab = None;
            return;
        }

        let Some(grab) = self.window_drag_grab else {
            return;
        };
        let ppp = ctx.pixels_per_point().max(1e-3);

        let new_outer = if let Some(screen) = screen_cursor_pos_points(ppp) {
            screen - grab
        } else {
            // Non-Windows / fallback: integrate raw motion when available.
            let outer_min = ctx
                .input(|i| i.viewport().outer_rect.map(|r| r.min))
                .unwrap_or(Pos2::ZERO);
            let delta = ctx.input(|i| {
                i.pointer
                    .motion()
                    .map(|m| m / ppp)
                    .unwrap_or_else(|| i.pointer.delta())
            });
            outer_min + delta
        };

        ctx.send_viewport_cmd(egui::ViewportCommand::OuterPosition(new_outer));
        ctx.set_cursor_icon(CursorIcon::Grabbing);
        ctx.request_repaint();
    }

    fn sidebar(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        ui.horizontal(|ui| {
            ui.label(RichText::new("Bony Build").size(16.0).strong().color(TEXT));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let search_on = self.model.show_task_search;
                if search_icon_btn(ui, search_on) {
                    self.model.show_task_search = !search_on;
                    if !self.model.show_task_search {
                        self.model.task_filter.clear();
                    }
                }
            });
        });

        if self.model.show_task_search {
            ui.add_space(6.0);
            let filter_hint = self.t("sidebar.filter_tasks").to_owned();
            ui.add(
                egui::TextEdit::singleline(&mut self.model.task_filter)
                    .desired_width(f32::INFINITY)
                    .hint_text(filter_hint)
                    .frame(true),
            );
        }

        ui.add_space(12.0);

        if nav_row(ui, SidebarGlyph::Plus, self.t("nav.new_task"), false) {
            self.create_task(ctx);
            return;
        }
        ui.add_space(2.0);
        let chat_selected =
            self.model.main_nav == MainNav::Chat && !self.model.is_viewing_history();
        if nav_row(ui, SidebarGlyph::Chat, self.t("nav.chat"), chat_selected) {
            self.model.return_to_live();
        }
        ui.add_space(2.0);
        let plugins_selected =
            self.model.main_nav == MainNav::Plugins || self.model.main_nav == MainNav::Unity;
        if nav_row(ui, SidebarGlyph::Plug, self.t("nav.plugins"), plugins_selected) {
            self.model.main_nav = MainNav::Plugins;
        }

        ui.add_space(16.0);
        ui.horizontal(|ui| {
            ui.label(
                RichText::new(self.t("sidebar.by_project"))
                    .size(11.5)
                    .color(MUTED),
            );
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if icon_btn(ui, SidebarGlyph::Plus, self.t("menu.open_project_short"), false)
                    .clicked()
                {
                    self.pick_project(ctx);
                }
            });
        });
        ui.add_space(6.0);

        egui::ComboBox::from_id_salt("task_status_filter")
            .selected_text(self.task_list_filter.label())
            .width(ui.available_width())
            .show_ui(ui, |ui| {
                for filter in [
                    TaskListFilter::All,
                    TaskListFilter::Active,
                    TaskListFilter::WaitingApproval,
                    TaskListFilter::Completed,
                    TaskListFilter::Failed,
                    TaskListFilter::Archived,
                ] {
                    ui.selectable_value(&mut self.task_list_filter, filter, filter.label());
                }
            });
        ui.add_space(8.0);

        egui::TopBottomPanel::bottom("sidebar_account")
            .exact_height(44.0)
            .frame(Frame::NONE)
            .show_separator_line(false)
            .show_inside(ui, |ui| {
                ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
                    let open = self.model.show_user_menu;
                    let pill = Frame::new()
                        .fill(if open { SELECTED } else { PANEL })
                        .corner_radius(CornerRadius::same(18))
                        .inner_margin(Margin::symmetric(8, 6))
                        .stroke(Stroke::new(
                            1.0,
                            if open {
                                Color32::from_rgb(70, 72, 84)
                            } else {
                                BORDER
                            },
                        ))
                        .show(ui, |ui| {
                            ui.horizontal(|ui| {
                                avatar_circle(ui, &self.model.initials());
                                ui.add_space(8.0);
                                ui.label(
                                    RichText::new(&self.model.display_name)
                                        .size(13.0)
                                        .color(TEXT),
                                );
                            });
                        })
                        .response
                        .interact(egui::Sense::click());
                    if pill.hovered() {
                        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
                    }
                    if pill.clicked() {
                        self.model.show_user_menu = !self.model.show_user_menu;
                        self.user_menu_just_opened = self.model.show_user_menu;
                        if self.model.show_user_menu {
                            self.user_menu_anchor = Some(pill.rect);
                        } else {
                            self.user_menu_anchor = None;
                        }
                    } else if self.model.show_user_menu {
                        self.user_menu_anchor = Some(pill.rect);
                    }
                });
            });

        let filter = self.model.task_filter.trim().to_lowercase();
        let groups = self.conversation_groups_cached(&filter).to_vec();
        let active_id = self.active_task_id.clone();

        egui::ScrollArea::vertical()
            .id_salt("task_list")
            .auto_shrink([false, false])
            .show(ui, |ui| {
                if groups.is_empty() {
                    ui.label(
                        RichText::new(if self.model.task_filter.trim().is_empty() {
                            self.t("sidebar.no_history")
                        } else {
                            self.t("sidebar.no_match")
                        })
                        .size(12.0)
                        .color(MUTED),
                    );
                }
                for group in &groups {
                    let key = project_group_key(&group.project_path);
                    let expanded = !self.collapsed_projects.contains(&key);
                    let mut toggle = false;
                    let mut switch_to = false;
                    let mut forget = false;
                    let mut new_task_here = false;
                    let project_path = group.project_path.clone();
                    let name = AppModel::project_label(&group.project_path);
                    let count = group.tasks.len() + group.archived.len();
                    let is_current = group.is_current;

                    ui.push_id(key.clone(), |ui| {
                        let hover_id = ui.make_persistent_id(("project_row_hover", key.as_str()));
                        let hovered = ui
                            .ctx()
                            .data(|d| d.get_temp::<bool>(hover_id))
                            .unwrap_or(false);
                        // Project rows are never "selected" like a chat — only a quiet
                        // current marker (text weight) + hover wash. Selection belongs
                        // exclusively to the conversation row (blue bar).
                        let header_fill = if hovered {
                            HOVER
                        } else {
                            Color32::TRANSPARENT
                        };
                        let header = Frame::new()
                            .fill(header_fill)
                            .corner_radius(CornerRadius::same(8))
                            .stroke(Stroke::new(
                                1.0,
                                if hovered { BORDER } else { Color32::TRANSPARENT },
                            ))
                            .inner_margin(Margin::symmetric(6, 5))
                            .show(ui, |ui| {
                                ui.set_width(ui.available_width());
                                ui.horizontal(|ui| {
                                    if icon_btn(
                                        ui,
                                        if expanded {
                                            SidebarGlyph::ChevronDown
                                        } else {
                                            SidebarGlyph::ChevronRight
                                        },
                                        if expanded { "折叠项目" } else { "展开项目" },
                                        false,
                                    )
                                    .clicked()
                                    {
                                        toggle = true;
                                    }
                                    paint_sidebar_glyph(
                                        ui,
                                        SidebarGlyph::Folder,
                                        if is_current || hovered { TEXT } else { MUTED },
                                    );
                                    ui.add_space(6.0);
                                    let name_resp = ui
                                        .add(
                                            egui::Label::new(
                                                RichText::new(&name)
                                                    .size(13.0)
                                                    .color(if is_current || hovered {
                                                        TEXT
                                                    } else {
                                                        MUTED
                                                    })
                                                    .strong(),
                                            )
                                            .sense(egui::Sense::click())
                                            .truncate(),
                                        )
                                        .on_hover_text(format!(
                                            "{}\n点击切换到此项目",
                                            project_path.display()
                                        ));
                                    if name_resp.clicked() {
                                        if !is_current {
                                            switch_to = true;
                                        } else {
                                            toggle = true;
                                        }
                                    }
                                    ui.with_layout(
                                        egui::Layout::right_to_left(egui::Align::Center),
                                        |ui| {
                                            let new_resp = ui
                                                .add(
                                                    egui::Button::new(
                                                        RichText::new("新建")
                                                            .size(11.5)
                                                            .color(if is_current || hovered {
                                                                TEXT
                                                            } else {
                                                                MUTED
                                                            }),
                                                    )
                                                    .fill(if hovered {
                                                        PANEL_2
                                                    } else {
                                                        Color32::TRANSPARENT
                                                    })
                                                    .stroke(Stroke::new(1.0, BORDER))
                                                    .corner_radius(CornerRadius::same(6))
                                                    .min_size(Vec2::new(40.0, 22.0)),
                                                )
                                                .on_hover_text("在此项目新建任务");
                                            if new_resp.clicked() {
                                                new_task_here = true;
                                            }
                                            if count > 0 {
                                                ui.label(
                                                    RichText::new(count.to_string())
                                                        .size(11.0)
                                                        .color(MUTED),
                                                );
                                            }
                                        },
                                    );
                                });
                            })
                            .response
                            .interact(egui::Sense::click());
                        ui.ctx()
                            .data_mut(|d| d.insert_temp(hover_id, header.hovered()));
                        if header.hovered() != hovered {
                            // Same-frame hover chrome: force a follow-up paint now,
                            // otherwise the highlight waits for an unrelated repaint.
                            ui.ctx().request_repaint();
                        }
                        if header.hovered() {
                            ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
                        }
                        if header.clicked() && !toggle && !new_task_here && !switch_to {
                            if !is_current {
                                switch_to = true;
                            } else {
                                toggle = true;
                            }
                        }
                        header.context_menu(|ui| {
                            if ui.button("在此项目新建任务").clicked() {
                                new_task_here = true;
                                ui.close_menu();
                            }
                            if !is_current
                                && ui.button(self.t("sidebar.switch_project")).clicked()
                            {
                                switch_to = true;
                                ui.close_menu();
                            }
                            if ui.button(self.t("sidebar.remove_from_list")).clicked() {
                                forget = true;
                                ui.close_menu();
                            }
                        });
                    });

                    if toggle {
                        if expanded {
                            self.collapsed_projects.insert(key.clone());
                        } else {
                            self.collapsed_projects.remove(&key);
                        }
                    }
                    if new_task_here {
                        self.create_task_in_project(ctx, project_path.clone());
                        return;
                    }
                    if switch_to {
                        self.switch_project(ctx, project_path.clone());
                        return;
                    }
                    if forget {
                        forget_project(&mut self.model.recent_projects, &project_path);
                        if group.tasks.is_empty() && group.archived.is_empty() {
                            continue;
                        }
                    }
                    if !expanded {
                        ui.add_space(4.0);
                        continue;
                    }
                    if group.tasks.is_empty() && group.archived.is_empty() {
                        ui.horizontal(|ui| {
                            ui.add_space(22.0);
                            ui.label(
                                RichText::new(self.t("sidebar.no_chats"))
                                    .size(11.5)
                                    .color(MUTED),
                            );
                            if ui.small_button("新建任务").clicked() {
                                self.create_task_in_project(ctx, project_path.clone());
                                return;
                            }
                        });
                        ui.add_space(4.0);
                        continue;
                    }
                    let show_archived_only = self.task_list_filter == TaskListFilter::Archived;
                    if !show_archived_only {
                        for &task_idx in &group.tasks {
                            let Some(task) = self.tasks.get(task_idx).cloned() else {
                                continue;
                            };
                            self.render_task_row(ui, ctx, &task, &active_id, false);
                        }
                    }
                    if !group.archived.is_empty()
                        && matches!(
                            self.task_list_filter,
                            TaskListFilter::All | TaskListFilter::Archived
                        )
                    {
                        let arch_expanded =
                            show_archived_only || self.expanded_archived.contains(&key);
                        let mut toggle_arch = false;
                        if !show_archived_only {
                            ui.add_space(2.0);
                            ui.horizontal(|ui| {
                                ui.add_space(10.0);
                                if icon_btn(
                                    ui,
                                    if arch_expanded {
                                        SidebarGlyph::ChevronDown
                                    } else {
                                        SidebarGlyph::ChevronRight
                                    },
                                    if arch_expanded {
                                        "折叠已归档"
                                    } else {
                                        "展开已归档"
                                    },
                                    false,
                                )
                                .clicked()
                                {
                                    toggle_arch = true;
                                }
                                paint_sidebar_glyph(ui, SidebarGlyph::Archive, MUTED);
                                ui.add_space(4.0);
                                let arch_resp = ui.add(
                                    egui::Label::new(
                                        RichText::new(format!(
                                            "已归档 · {}",
                                            group.archived.len()
                                        ))
                                        .size(11.5)
                                        .color(MUTED),
                                    )
                                    .sense(egui::Sense::click()),
                                );
                                if arch_resp.clicked() {
                                    toggle_arch = true;
                                }
                            });
                        }
                        if toggle_arch {
                            if arch_expanded {
                                self.expanded_archived.remove(&key);
                            } else {
                                self.expanded_archived.insert(key.clone());
                            }
                        }
                        if arch_expanded || show_archived_only {
                            for &task_idx in &group.archived {
                                let Some(task) = self.tasks.get(task_idx).cloned() else {
                                    continue;
                                };
                                self.render_task_row(ui, ctx, &task, &active_id, true);
                            }
                        }
                    }
                    ui.add_space(8.0);
                }

                // Always below projects. Do not gate on awaiting_project_choice —
                // selecting a recent chat used to clear that flag and the whole
                // section vanished, which felt like a broken jump to bony-build.
                ui.add_space(10.0);
                self.sidebar_recent_inbox(ui, ctx);
            });
    }

    fn rename_task_modal(&mut self, ctx: &egui::Context) {
        let Some((task_id, mut title)) = self.rename_task.take() else {
            return;
        };
        let mut keep_open = true;
        let mut save = false;
        let mut cancel = false;
        egui::Window::new("重命名任务")
            .collapsible(false)
            .resizable(false)
            .anchor(Align2::CENTER_CENTER, [0.0, 0.0])
            .show(ctx, |ui| {
                ui.set_min_width(360.0);
                let response = ui.add(
                    egui::TextEdit::singleline(&mut title)
                        .desired_width(f32::INFINITY)
                        .hint_text("任务名称"),
                );
                response.request_focus();
                ui.add_space(12.0);
                ui.horizontal(|ui| {
                    if ui.button("取消").clicked() {
                        cancel = true;
                    }
                    if ui
                        .add_enabled(!title.trim().is_empty(), egui::Button::new("保存"))
                        .clicked()
                    {
                        save = true;
                    }
                });
            });
        if save {
            if let Some(task) = self.tasks.iter_mut().find(|task| task.id == task_id) {
                task.title = title.trim().chars().take(80).collect();
                task.updated_at = unix_time();
                if self.active_task_id.as_ref() == Some(&task.id) {
                    self.model.task_title = task.title.clone();
                }
                if let Some(repo) = &self.task_repo
                    && let Err(error) = repo.save(task)
                {
                    self.task_error = Some(error);
                }
            }
            keep_open = false;
        }
        if cancel {
            keep_open = false;
        }
        if keep_open {
            self.rename_task = Some((task_id, title));
        }
    }

    fn delete_task_modal(&mut self, ctx: &egui::Context) {
        let Some(task_id) = self.delete_task.take() else {
            return;
        };
        let Some(task) = self.tasks.iter().find(|task| task.id == task_id).cloned() else {
            return;
        };
        let busy = matches!(
            task.status,
            TaskStatus::Running | TaskStatus::WaitingApproval
        );
        let title = display_task_title(&task);
        let mut keep_open = true;
        let mut confirmed = false;
        let mut dismiss = false;
        // Opening click must not also dismiss the backdrop on this frame.
        let ignore_backdrop = self.delete_modal_ignore_click;
        self.delete_modal_ignore_click = false;

        egui::Area::new(egui::Id::new("delete_task_dim"))
            .order(egui::Order::Middle)
            .interactable(true)
            .fixed_pos(Pos2::ZERO)
            .show(ctx, |ui| {
                let screen = ctx.screen_rect();
                let resp = ui.allocate_rect(screen, egui::Sense::click());
                ui.painter()
                    .rect_filled(screen, 0.0, Color32::from_black_alpha(160));
                if !ignore_backdrop && resp.clicked() {
                    dismiss = true;
                }
            });

        let mut open = true;
        egui::Window::new("删除对话？")
            .collapsible(false)
            .resizable(false)
            .title_bar(false)
            .anchor(Align2::CENTER_CENTER, [0.0, 0.0])
            .order(egui::Order::Foreground)
            .frame(
                Frame::new()
                    .fill(PANEL)
                    .corner_radius(CornerRadius::same(16))
                    .stroke(Stroke::new(1.0, BORDER))
                    .inner_margin(Margin::same(18))
                    .shadow(Shadow {
                        offset: [0, 12],
                        blur: 36,
                        spread: 0,
                        color: Color32::from_black_alpha(160),
                    }),
            )
            .open(&mut open)
            .show(ctx, |ui| {
                ui.set_min_width(380.0);
                ui.label(
                    RichText::new("删除对话？")
                        .size(16.0)
                        .strong()
                        .color(TEXT),
                );
                ui.add_space(8.0);
                ui.label(
                    RichText::new(format!("将删除「{title}」的本地记录。"))
                        .size(13.0)
                        .color(MUTED),
                );
                if task.isolated {
                    ui.add_space(4.0);
                    ui.label(
                        RichText::new("不会自动删除 worktree 或其中未提交的修改。")
                            .size(12.5)
                            .color(MUTED),
                    );
                }
                if busy {
                    ui.add_space(8.0);
                    ui.label(
                        RichText::new("该对话正在运行或等待审批，请先停止后再删除。")
                            .size(12.5)
                            .color(DANGER),
                    );
                }
                ui.add_space(16.0);
                ui.horizontal(|ui| {
                    if ui
                        .add(
                            egui::Button::new(RichText::new("取消").size(13.0).color(TEXT))
                                .fill(PANEL_2)
                                .stroke(Stroke::new(1.0, BORDER))
                                .corner_radius(CornerRadius::same(8))
                                .min_size(Vec2::new(72.0, 30.0)),
                        )
                        .clicked()
                    {
                        dismiss = true;
                    }
                    ui.add_space(8.0);
                    if ui
                        .add_enabled(
                            !busy,
                            egui::Button::new(
                                RichText::new("删除")
                                    .size(13.0)
                                    .color(if busy { MUTED } else { Color32::WHITE })
                                    .strong(),
                            )
                            .fill(if busy {
                                PANEL_2
                            } else {
                                Color32::from_rgb(160, 60, 60)
                            })
                            .stroke(Stroke::new(
                                1.0,
                                if busy {
                                    BORDER
                                } else {
                                    Color32::from_rgb(190, 80, 80)
                                },
                            ))
                            .corner_radius(CornerRadius::same(8))
                            .min_size(Vec2::new(72.0, 30.0)),
                        )
                        .clicked()
                    {
                        confirmed = true;
                    }
                });
            });

        if confirmed {
            keep_open = false;
            let was_active = self.active_task_id.as_ref() == Some(&task.id);
            if let Some(repo) = &self.task_repo
                && let Err(error) = repo.delete(&task.id)
            {
                self.task_error = Some(error);
                keep_open = true;
            } else {
                self.tasks.retain(|item| item.id != task.id);
                self.sidebar_groups_cache = None;
                if was_active {
                    self.active_task_id = None;
                    self.model.new_task();
                    self.model.task_title.clear();
                    self.model.status = "对话已删除".into();
                    self.clear_session_plugins();
                }
            }
        }
        if !open || dismiss {
            keep_open = false;
        }
        if keep_open {
            self.delete_task = Some(task_id);
        }
    }

    fn right_panel(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.label(RichText::new("详情").size(15.0).strong().color(TEXT));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if icon_btn(ui, SidebarGlyph::Close, "关闭详情", false).clicked() {
                    self.model.show_right_panel = false;
                }
            });
        });
        ui.add_space(12.0);

        let status = if self.model.needs_login {
            ("需要登录", DANGER)
        } else if self.model.status.contains("Error") {
            ("出错", DANGER)
        } else if self.model.busy {
            ("思考中…", MUTED)
        } else if self.model.connected {
            ("就绪", OK)
        } else {
            ("连接中…", MUTED)
        };
        ui.label(RichText::new("会话").size(12.0).color(MUTED));
        ui.label(RichText::new(status.0).size(14.0).color(status.1));
        ui.add_space(10.0);

        ui.label(RichText::new("工作目录").size(12.0).color(MUTED));
        let cwd = self
            .model
            .cwd
            .as_ref()
            .map(|p| p.display().to_string())
            .unwrap_or_else(|| "—".into());
        ui.label(RichText::new(cwd).size(12.5).color(TEXT));
        ui.add_space(10.0);

        ui.label(RichText::new("模型").size(12.0).color(MUTED));
        ui.label(
            RichText::new(&self.model.current_model_name)
                .size(13.5)
                .color(TEXT),
        );
        ui.add_space(10.0);

        let total = self.model.usage.cumulative.total_tokens;
        let hist: u64 = self
            .model
            .history_turns
            .iter()
            .map(|t| t.usage_delta.total_tokens)
            .sum();
        ui.label(RichText::new("Token").size(12.0).color(MUTED));
        ui.label(
            RichText::new(format!("Σ {}", format_tokens(total.max(hist))))
                .size(14.0)
                .strong()
                .color(TEXT),
        );
        ui.add_space(16.0);

        if ui
            .add(
                egui::Button::new(RichText::new("打开使用统计").size(13.0).color(TEXT))
                    .fill(PANEL_2)
                    .min_size(Vec2::new(ui.available_width(), 34.0))
                    .corner_radius(CornerRadius::same(8)),
            )
            .clicked()
        {
            self.model.show_usage_detail = true;
        }

        ui.add_space(18.0);
        ui.horizontal(|ui| {
            ui.label(
                RichText::new(format!("Changes ({})", self.changes.len()))
                    .size(13.0)
                    .strong()
                    .color(TEXT),
            );
            if ui.small_button("刷新").clicked() {
                match GitWorkspaceService::changes(&self.config.cwd) {
                    Ok(v) => self.changes = v,
                    Err(e) => self.task_error = Some(e),
                }
            }
        });
        ui.add_space(6.0);
        egui::ScrollArea::vertical()
            .id_salt("changes")
            .max_height(220.0)
            .show(ui, |ui| {
                for change in self.changes.clone() {
                    let mark = match change.kind {
                        ChangeKind::Added => "A",
                        ChangeKind::Modified => "M",
                        ChangeKind::Deleted => "D",
                        ChangeKind::Renamed => "R",
                        ChangeKind::Untracked => "?",
                        ChangeKind::Conflicted => "!",
                    };
                    let label = format!("{mark}  {}", change.path.display());
                    if ui
                        .selectable_label(
                            self.selected_diff
                                .as_ref()
                                .is_some_and(|(p, _)| p == &change.path),
                            label,
                        )
                        .clicked()
                    {
                        match GitWorkspaceService::diff(&self.config.cwd, Some(&change.path), false)
                        {
                            Ok(diff) => self.selected_diff = Some((change.path.clone(), diff)),
                            Err(e) => self.task_error = Some(e),
                        }
                    }
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        let action = if change.staged {
                            "取消暂存"
                        } else {
                            "暂存"
                        };
                        if ui.small_button(action).clicked() {
                            self.pending_git_action = Some((!change.staged, change.path.clone()));
                        }
                    });
                }
            });
        if let Some((path, diff)) = &self.selected_diff {
            ui.separator();
            ui.label(
                RichText::new(path.display().to_string())
                    .size(12.0)
                    .strong(),
            );
            egui::ScrollArea::both()
                .id_salt("diff_preview")
                .max_height(260.0)
                .show(ui, |ui| {
                    ui.monospace(if diff.is_empty() {
                        "未跟踪文件暂无 diff"
                    } else {
                        diff
                    });
                });
        }
    }

    fn nav_placeholder(&mut self, ui: &mut egui::Ui) {
        ui.add_space(80.0);
        ui.vertical_centered(|ui| {
            ui.label(
                RichText::new(self.model.main_nav.title())
                    .size(22.0)
                    .strong()
                    .color(TEXT),
            );
            ui.add_space(10.0);
            ui.label(
                RichText::new(self.model.main_nav.placeholder_blurb())
                    .size(14.0)
                    .color(MUTED),
            );
            ui.add_space(20.0);
            if ui
                .add(
                    egui::Button::new(RichText::new("回到聊天").size(13.0).color(BG).strong())
                        .fill(ACCENT)
                        .min_size(Vec2::new(120.0, 34.0))
                        .corner_radius(CornerRadius::same(10)),
                )
                .clicked()
            {
                self.model.go_chat();
            }
        });
    }

    fn set_unity_plugin_enabled(&mut self, enabled: bool) {
        self.plugin_prefs.unity_enabled = enabled;
        if !enabled {
            self.clear_session_plugins();
            if self.model.main_nav == MainNav::Unity {
                self.model.main_nav = MainNav::Plugins;
            }
        }
        save_plugin_prefs(&self.plugin_prefs);
    }

    fn set_openmontage_enabled(&mut self, enabled: bool) {
        let root = self.openmontage.root.clone();
        let result = if enabled {
            if !self.openmontage.status.is_ready() {
                self.openmontage.refresh_status();
            }
            if !self.openmontage.status.is_ready() {
                self.model.status = "OpenMontage 尚未就绪，请先安装".into();
                return;
            }
            crate::openmontage::enable_skill(&mut self.plugin_prefs, &root)
        } else {
            crate::openmontage::disable_skill(&mut self.plugin_prefs)
        };
        match result {
            Ok(()) => {
                save_plugin_prefs(&self.plugin_prefs);
                self.model.status = if enabled {
                    "已启用 OpenMontage".into()
                } else {
                    "已关闭 OpenMontage".into()
                };
            }
            Err(e) => {
                self.model.status = e;
            }
        }
    }

    fn pick_openmontage_root(&mut self) {
        let start = self.openmontage.root.clone();
        if let Some(path) = rfd::FileDialog::new()
            .set_title("选择 OpenMontage 安装目录")
            .set_directory(start.parent().unwrap_or(start.as_path()))
            .pick_folder()
        {
            self.openmontage.root = path.clone();
            self.plugin_prefs.openmontage_root = Some(path);
            save_plugin_prefs(&self.plugin_prefs);
            self.openmontage.refresh_status();
        }
    }

    fn set_bevy_enabled(&mut self, enabled: bool) {
        let project = self.bevy.project_path.clone();
        let result = if enabled {
            if !self.bevy.status.is_ready() {
                self.bevy.refresh_status();
            }
            if !self.bevy.status.is_ready() {
                self.model.status = "Bevy 项目尚未就绪，请先创建/选择项目".into();
                return;
            }
            crate::bevy::enable_skill(&mut self.plugin_prefs, &project)
        } else {
            crate::bevy::disable_skill(&mut self.plugin_prefs)
        };
        match result {
            Ok(()) => {
                save_plugin_prefs(&self.plugin_prefs);
                self.model.status = if enabled {
                    "已启用 Bevy".into()
                } else {
                    "已关闭 Bevy".into()
                };
            }
            Err(e) => {
                self.model.status = e;
            }
        }
    }

    fn pick_bevy_project(&mut self) {
        let start = self.bevy.project_path.clone();
        if let Some(path) = rfd::FileDialog::new()
            .set_title("选择已有 Bevy 项目目录（含 Cargo.toml）")
            .set_directory(start.parent().unwrap_or(start.as_path()))
            .pick_folder()
        {
            self.bevy.set_project_path(path.clone());
            self.plugin_prefs.bevy_project_root = Some(path);
            save_plugin_prefs(&self.plugin_prefs);
        }
    }

    fn create_bevy_project(&mut self) {
        let name = if self.bevy_new_project_name.trim().is_empty() {
            "my-game".to_string()
        } else {
            self.bevy_new_project_name.trim().to_string()
        };
        let default_parent = dirs::home_dir()
            .unwrap_or_else(|| std::path::PathBuf::from("."))
            .join("BonyBevyGames");
        let start = self
            .bevy
            .project_path
            .parent()
            .map(|p| p.to_path_buf())
            .unwrap_or(default_parent.clone());
        if let Some(chosen) = rfd::FileDialog::new()
            .set_title("选择新 Bevy 项目的父目录")
            .set_directory(if start.is_dir() { &start } else { &default_parent })
            .pick_folder()
        {
            self.bevy.create_project(chosen, name);
        }
    }

    /// Activate / deactivate a plugin for this conversation only (not persisted).
    fn set_chat_interaction(&mut self, mode: ChatInteraction) {
        let mode = if mode == ChatInteraction::Unity && !self.plugin_prefs.unity_enabled {
            ChatInteraction::Agent
        } else {
            mode
        };
        self.unity_chat_mode = mode == ChatInteraction::Unity;
        if self.unity_chat_mode {
            self.unity.ensure_detecting();
        }
    }

    fn clear_session_plugins(&mut self) {
        self.unity_chat_mode = false;
        self.unity_docs_open = false;
        self.show_unity_docs_window = false;
        self.show_composer_plus = false;
        self.composer_plus_anchor = None;
    }

    fn open_unity_settings(&mut self) {
        if !self.plugin_prefs.unity_enabled {
            self.set_unity_plugin_enabled(true);
        }
        self.model.main_nav = MainNav::Unity;
        self.unity.ensure_detecting();
    }

    fn plugins_panel(&mut self, ui: &mut egui::Ui) {
        let tab_plugins = self.t("plugins.tab_plugins").to_owned();
        let tab_skills = self.t("plugins.tab_skills").to_owned();
        let refresh_label = self.t("plugins.refresh").to_owned();
        let back_label = self.t("plugins.back_store").to_owned();
        let search_hint = self.t("plugins.search").to_owned();
        let blurb = match self.plugins_subnav {
            PluginsSubNav::Plugins => self.t("plugins.blurb"),
            PluginsSubNav::Skills => self.t("plugins.skills_blurb"),
        }
        .to_owned();

        ui.add_space(8.0);
        ui.horizontal(|ui| {
            for (tab, label) in [
                (PluginsSubNav::Plugins, tab_plugins.as_str()),
                (PluginsSubNav::Skills, tab_skills.as_str()),
            ] {
                let selected = self.plugins_subnav == tab;
                let resp = ui.add(
                    egui::Button::new(
                        RichText::new(label)
                            .size(13.0)
                            .strong()
                            .color(if selected { TEXT } else { MUTED }),
                    )
                    .fill(if selected {
                        Color32::from_rgb(48, 50, 60)
                    } else {
                        Color32::TRANSPARENT
                    })
                    .stroke(Stroke::new(
                        1.0,
                        if selected {
                            Color32::from_rgb(70, 72, 84)
                        } else {
                            Color32::TRANSPARENT
                        },
                    ))
                    .corner_radius(CornerRadius::same(8))
                    .min_size(Vec2::new(64.0, 28.0)),
                );
                if resp.clicked() && self.plugins_subnav != tab {
                    self.plugins_subnav = tab;
                    self.plugins_selected = None;
                    self.plugins_search.clear();
                }
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if plugin_secondary_btn(ui, &refresh_label).clicked() {
                    self.unity.ensure_detecting();
                    self.openmontage.refresh_status();
                    self.bevy.refresh_status();
                }
            });
        });

        if let Some(selected) = self.plugins_selected {
            ui.add_space(10.0);
            if plugin_link_btn(ui, &back_label).clicked() {
                self.plugins_selected = None;
            }
            ui.add_space(6.0);
            match selected {
                PluginCatalogId::Unity => self.unity_plugin_detail(ui),
                PluginCatalogId::OpenMontage | PluginCatalogId::SkillOpenMontage => {
                    if matches!(selected, PluginCatalogId::SkillOpenMontage) {
                        let path = crate::openmontage::skill_path().display().to_string();
                        let label = self.t("plugins.skill_path").to_owned();
                        plugin_path_row(ui, &label, &path, None, || {});
                        ui.add_space(8.0);
                    }
                    self.openmontage_plugin_card(ui);
                }
                PluginCatalogId::Bevy | PluginCatalogId::SkillBevy => {
                    if matches!(selected, PluginCatalogId::SkillBevy) {
                        let path = crate::bevy::skill_path().display().to_string();
                        let label = self.t("plugins.skill_path").to_owned();
                        plugin_path_row(ui, &label, &path, None, || {});
                        ui.add_space(8.0);
                    }
                    self.bevy_plugin_card(ui);
                }
            }
            ui.add_space(24.0);
            return;
        }

        ui.add_space(8.0);
        ui.label(RichText::new(&blurb).size(12.5).color(MUTED));
        ui.add_space(10.0);
        ui.add(
            egui::TextEdit::singleline(&mut self.plugins_search)
                .desired_width(ui.available_width().min(520.0))
                .hint_text(RichText::new(&search_hint).color(MUTED))
                .margin(Margin::symmetric(12, 8)),
        );
        ui.add_space(12.0);

        let entries = self.plugin_catalog_entries();
        let q = self.plugins_search.trim().to_lowercase();
        let filtered: Vec<PluginCatalogEntry> = entries
            .into_iter()
            .filter(|e| {
                if q.is_empty() {
                    return true;
                }
                let title = self.t(e.title_key).to_lowercase();
                let blurb = self.t(e.blurb_key).to_lowercase();
                title.contains(&q) || blurb.contains(&q)
            })
            .collect();

        self.plugins_installed_strip(ui, &filtered);
        ui.add_space(10.0);

        if filtered.is_empty() {
            ui.label(
                RichText::new(self.t("plugins.no_results"))
                    .size(13.0)
                    .color(MUTED),
            );
            ui.add_space(24.0);
            return;
        }

        // Full-width stacked cards — avoid egui horizontal wrap/misalign.
        let mut open_id: Option<PluginCatalogId> = None;
        let mut install_id: Option<PluginCatalogId> = None;
        let card_w = ui.available_width();
        for entry in &filtered {
            let enabled = self.catalog_enabled(entry.id);
            let title = self.t(entry.title_key).to_owned();
            let blurb = self.t(entry.blurb_key).to_owned();
            let install = self.t("plugins.install").to_owned();
            let configure = self.t("plugins.configure").to_owned();
            ui.allocate_ui_with_layout(
                Vec2::new(card_w, 0.0),
                egui::Layout::top_down(egui::Align::Min).with_cross_justify(true),
                |ui| {
                    ui.set_width(card_w);
                    let (open, install_clicked) = plugin_store_card(
                        ui,
                        entry.glyph,
                        entry.accent,
                        &title,
                        &blurb,
                        enabled,
                        &install,
                        &configure,
                    );
                    if open {
                        open_id = Some(entry.id);
                    }
                    if install_clicked {
                        install_id = Some(entry.id);
                    }
                },
            );
            ui.add_space(8.0);
        }

        if let Some(id) = install_id {
            self.catalog_install(id);
            self.plugins_selected = Some(id);
        } else if let Some(id) = open_id {
            self.plugins_selected = Some(id);
        }
        ui.add_space(16.0);
    }

    fn plugin_catalog_entries(&self) -> Vec<PluginCatalogEntry> {
        match self.plugins_subnav {
            PluginsSubNav::Plugins => vec![
                PluginCatalogEntry {
                    id: PluginCatalogId::Unity,
                    category_key: "plugins.featured",
                    title_key: "plugins.unity_title",
                    blurb_key: "plugins.unity_blurb",
                    glyph: SidebarGlyph::Unity,
                    accent: UNITY_ACCENT,
                },
                PluginCatalogEntry {
                    id: PluginCatalogId::OpenMontage,
                    category_key: "plugins.cat_video",
                    title_key: "plugins.openmontage_title",
                    blurb_key: "plugins.openmontage_blurb",
                    glyph: SidebarGlyph::Plug,
                    accent: OM_ACCENT,
                },
                PluginCatalogEntry {
                    id: PluginCatalogId::Bevy,
                    category_key: "plugins.cat_gamedev",
                    title_key: "plugins.bevy_title",
                    blurb_key: "plugins.bevy_blurb",
                    glyph: SidebarGlyph::Plug,
                    accent: BEVY_ACCENT,
                },
            ],
            PluginsSubNav::Skills => vec![
                PluginCatalogEntry {
                    id: PluginCatalogId::SkillOpenMontage,
                    category_key: "plugins.cat_video",
                    title_key: "plugins.skill_om_title",
                    blurb_key: "plugins.skill_om_blurb",
                    glyph: SidebarGlyph::Plug,
                    accent: OM_ACCENT,
                },
                PluginCatalogEntry {
                    id: PluginCatalogId::SkillBevy,
                    category_key: "plugins.cat_gamedev",
                    title_key: "plugins.skill_bevy_title",
                    blurb_key: "plugins.skill_bevy_blurb",
                    glyph: SidebarGlyph::Plug,
                    accent: BEVY_ACCENT,
                },
            ],
        }
    }

    fn catalog_enabled(&self, id: PluginCatalogId) -> bool {
        match id {
            PluginCatalogId::Unity => self.plugin_prefs.unity_enabled,
            PluginCatalogId::OpenMontage | PluginCatalogId::SkillOpenMontage => {
                self.plugin_prefs.openmontage_enabled
            }
            PluginCatalogId::Bevy | PluginCatalogId::SkillBevy => self.plugin_prefs.bevy_enabled,
        }
    }

    fn catalog_install(&mut self, id: PluginCatalogId) {
        match id {
            PluginCatalogId::Unity => self.set_unity_plugin_enabled(true),
            PluginCatalogId::OpenMontage | PluginCatalogId::SkillOpenMontage => {
                self.set_openmontage_enabled(true);
            }
            PluginCatalogId::Bevy | PluginCatalogId::SkillBevy => {
                self.set_bevy_enabled(true);
            }
        }
    }

    fn plugins_installed_strip(&mut self, ui: &mut egui::Ui, visible: &[PluginCatalogEntry]) {
        let installed_label = self.t("plugins.installed").to_owned();
        let installed: Vec<PluginCatalogEntry> = visible
            .iter()
            .copied()
            .filter(|e| self.catalog_enabled(e.id))
            .collect();
        if installed.is_empty() {
            return;
        }

        ui.horizontal(|ui| {
            ui.label(
                RichText::new(&installed_label)
                    .size(12.5)
                    .strong()
                    .color(MUTED),
            );
        });
        ui.add_space(6.0);
        let mut open: Option<PluginCatalogId> = None;
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing.x = 10.0;
            for entry in &installed {
                let size = Vec2::splat(36.0);
                let (rect, resp) = ui.allocate_exact_size(size, egui::Sense::click());
                let tip = self.t(entry.title_key);
                let resp = resp.on_hover_text(tip);
                ui.painter().rect_filled(
                    rect,
                    CornerRadius::same(9),
                    Color32::from_rgb(44, 46, 56),
                );
                ui.painter().rect_stroke(
                    rect,
                    CornerRadius::same(9),
                    Stroke::new(1.0, Color32::from_rgb(62, 64, 76)),
                    egui::StrokeKind::Inside,
                );
                paint_sidebar_glyph_at(ui.painter(), rect.center(), entry.glyph, entry.accent);
                if resp.clicked() {
                    open = Some(entry.id);
                }
            }
        });
        if let Some(id) = open {
            self.plugins_selected = Some(id);
        }
        ui.add_space(4.0);
    }

    fn unity_plugin_detail(&mut self, ui: &mut egui::Ui) {
        let unity_enabled = self.plugin_prefs.unity_enabled;
        let unity_status = self.unity.status.label().to_string();
        let unity_status_color = match self.unity.status {
            CliStatus::Ready => OK,
            CliStatus::Missing | CliStatus::Error => DANGER,
            _ => MUTED,
        };
        let unity_title = self.t("plugins.unity_title").to_string();
        let unity_blurb = self.t("plugins.unity_blurb").to_string();
        let unity_chat_hint = self.t("plugins.chat_hint").to_string();
        let unity_settings = self.t("plugins.open_settings").to_string();
        let unity_docs = self.t("plugins.docs").to_string();
        let unity_docs_tip = self.t("plugins.docs_tip").to_string();
        let unity_use = self.t("plugins.use_in_chat").to_string();
        let unity_off_hint = self.t("plugins.enabled_hint").to_string();
        let enable_tip = self.t("plugins.enable").to_string();

        plugin_section(ui, |ui| {
            plugin_header(
                ui,
                SidebarGlyph::Unity,
                if unity_enabled { UNITY_ACCENT } else { MUTED },
                &unity_title,
                &unity_blurb,
                Some((unity_enabled, UNITY_ACCENT, &enable_tip)),
                |on| self.set_unity_plugin_enabled(on),
            );

            ui.add_space(10.0);
            plugin_status_line(
                ui,
                unity_status_color,
                &unity_status,
                if unity_enabled {
                    Some(unity_chat_hint.as_str())
                } else {
                    None
                },
            );

            if unity_enabled {
                ui.add_space(12.0);
                ui.horizontal_wrapped(|ui| {
                    ui.spacing_mut().item_spacing.x = 8.0;
                    if plugin_primary_btn(ui, &unity_settings, UNITY_ACCENT).clicked() {
                        self.open_unity_settings();
                    }
                    if plugin_secondary_btn(ui, &unity_docs)
                        .on_hover_text(&unity_docs_tip)
                        .clicked()
                    {
                        self.show_unity_docs_window = true;
                    }
                    if plugin_secondary_btn(ui, &unity_use).clicked() {
                        self.model.go_chat();
                        self.set_chat_interaction(ChatInteraction::Unity);
                        self.model.focus_composer = true;
                    }
                });
            } else {
                ui.add_space(8.0);
                ui.label(RichText::new(&unity_off_hint).size(12.0).color(MUTED));
            }
        });
    }

    fn bevy_plugin_card(&mut self, ui: &mut egui::Ui) {
        let enabled = self.plugin_prefs.bevy_enabled;
        let status = self.bevy.status;
        let busy = self.bevy.busy;
        let running = self.bevy.running;
        let project_display = self.bevy.project_path.display().to_string();
        let last_error = self.bevy.last_error.clone();
        let install_hint = self.bevy.rust_install_hint().to_string();
        let can_install_rust = self.bevy.can_install_rust();
        let can_stop = self.bevy.can_stop();
        let status_color = match status {
            BevyStatus::Ready => OK,
            BevyStatus::NoRust | BevyStatus::Error => DANGER,
            BevyStatus::NoProject => BEVY_ACCENT,
            BevyStatus::Unknown => MUTED,
        };
        let title = self.t("plugins.bevy_title").to_string();
        let blurb = self.t("plugins.bevy_blurb").to_string();
        let enable_tip = self.t("plugins.enable").to_string();
        let project_label = self.t("plugins.project").to_string();
        let pick_label = self.t("plugins.pick_existing").to_string();
        let running_hint = self.t("plugins.bevy_running").to_string();
        let no_rust = self.t("plugins.bevy_no_rust").to_string();
        let install_rust = self.t("plugins.bevy_install_rust").to_string();
        let install_rust_tip = self.t("plugins.bevy_install_rust_tip").to_string();
        let copy_label = self.t("plugins.bevy_copy").to_string();
        let copied = self.t("plugins.bevy_copied").to_string();
        let no_project = self.t("plugins.bevy_no_project").to_string();
        let project_name = self.t("plugins.bevy_project_name").to_string();
        let create_label = self.t("plugins.bevy_create").to_string();
        let check_tip = self.t("plugins.bevy_check_tip").to_string();
        let run_label = self.t("plugins.bevy_run").to_string();
        let run_tip = self.t("plugins.bevy_run_tip").to_string();
        let stop_label = self.t("plugins.bevy_stop").to_string();
        let detecting = self.t("plugins.bevy_detecting").to_string();
        let enabled_hint = self.t("plugins.bevy_enabled_hint").to_string();
        let icon_color = if enabled && status.is_ready() {
            BEVY_ACCENT
        } else {
            MUTED
        };

        plugin_section(ui, |ui| {
            plugin_header(
                ui,
                SidebarGlyph::Plug,
                icon_color,
                &title,
                &blurb,
                if status.is_ready() {
                    Some((enabled, BEVY_ACCENT, enable_tip.as_str()))
                } else {
                    None
                },
                |on| self.set_bevy_enabled(on),
            );

            ui.add_space(10.0);
            plugin_path_row(
                ui,
                &project_label,
                &project_display,
                (!busy).then_some(pick_label.as_str()),
                || self.pick_bevy_project(),
            );

            ui.add_space(6.0);
            plugin_status_line(
                ui,
                status_color,
                status.label(),
                if running {
                    Some(running_hint.as_str())
                } else {
                    None
                },
            );

            match status {
                BevyStatus::NoRust => {
                    ui.add_space(10.0);
                    ui.label(RichText::new(&no_rust).size(12.0).color(MUTED));
                    ui.add_space(8.0);
                    Frame::new()
                        .fill(BG)
                        .corner_radius(CornerRadius::same(8))
                        .inner_margin(Margin::symmetric(10, 8))
                        .show(ui, |ui| {
                            ui.set_width(ui.available_width());
                            ui.horizontal(|ui| {
                                ui.add(
                                    egui::Label::new(
                                        RichText::new(&install_hint)
                                            .size(11.5)
                                            .monospace()
                                            .color(TEXT),
                                    )
                                    .truncate(),
                                );
                                ui.with_layout(
                                    egui::Layout::right_to_left(egui::Align::Center),
                                    |ui| {
                                        if plugin_link_btn(ui, &copy_label).clicked() {
                                            ui.ctx().copy_text(install_hint.clone());
                                            self.bevy.toast = Some(copied.clone());
                                        }
                                    },
                                );
                            });
                        });
                    ui.add_space(10.0);
                    if ui
                        .add_enabled(can_install_rust, plugin_primary_btn_widget(&install_rust, OK))
                        .on_hover_text(&install_rust_tip)
                        .clicked()
                    {
                        self.bevy.install_rust();
                    }
                    self.bevy_log_tail(ui);
                }
                BevyStatus::NoProject => {
                    ui.add_space(10.0);
                    ui.label(RichText::new(&no_project).size(12.0).color(MUTED));
                    ui.add_space(10.0);
                    ui.horizontal(|ui| {
                        ui.label(RichText::new(&project_name).size(12.0).color(MUTED));
                        ui.add(
                            egui::TextEdit::singleline(&mut self.bevy_new_project_name)
                                .desired_width(168.0)
                                .hint_text("my-game"),
                        );
                        if ui
                            .add_enabled(
                                !busy,
                                plugin_primary_btn_widget(&create_label, BEVY_ACCENT),
                            )
                            .clicked()
                        {
                            self.create_bevy_project();
                        }
                    });
                    self.bevy_log_tail(ui);
                }
                BevyStatus::Error => {
                    ui.add_space(8.0);
                    if let Some(err) = &last_error {
                        ui.label(RichText::new(err).size(12.0).color(DANGER));
                    }
                    self.bevy_log_tail(ui);
                }
                BevyStatus::Ready => {
                    ui.add_space(12.0);
                    ui.horizontal_wrapped(|ui| {
                        ui.spacing_mut().item_spacing.x = 8.0;
                        if ui
                            .add_enabled(!busy, plugin_secondary_btn_widget("cargo check"))
                            .on_hover_text(&check_tip)
                            .clicked()
                        {
                            self.bevy.check();
                        }
                        if ui
                            .add_enabled(!busy, plugin_secondary_btn_widget("cargo build"))
                            .clicked()
                        {
                            self.bevy.build();
                        }
                        if ui
                            .add_enabled(!busy, plugin_primary_btn_widget(&run_label, BEVY_ACCENT))
                            .on_hover_text(&run_tip)
                            .clicked()
                        {
                            self.bevy.run();
                        }
                        if can_stop && plugin_danger_btn(ui, &stop_label).clicked() {
                            self.bevy.stop();
                        }
                    });
                    if let Some(err) = &last_error {
                        ui.add_space(8.0);
                        ui.label(RichText::new(err).size(12.0).color(DANGER));
                    }
                    self.bevy_log_tail(ui);
                    if enabled {
                        ui.add_space(8.0);
                        ui.label(RichText::new(&enabled_hint).size(12.0).color(MUTED));
                    }
                }
                BevyStatus::Unknown => {
                    ui.add_space(8.0);
                    ui.label(RichText::new(&detecting).size(12.0).color(MUTED));
                }
            }
        });
    }

    fn bevy_log_tail(&self, ui: &mut egui::Ui) {
        if self.bevy.log_tail.is_empty() {
            return;
        }
        ui.add_space(10.0);
        Frame::new()
            .fill(BG)
            .corner_radius(CornerRadius::same(8))
            .inner_margin(Margin::symmetric(10, 8))
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                egui::ScrollArea::vertical()
                    .id_salt("bevy_log_tail")
                    .max_height(112.0)
                    .stick_to_bottom(true)
                    .show(ui, |ui| {
                        for line in &self.bevy.log_tail {
                            ui.label(
                                RichText::new(line)
                                    .size(11.0)
                                    .monospace()
                                    .color(MUTED),
                            );
                        }
                    });
            });
    }

    fn openmontage_plugin_card(&mut self, ui: &mut egui::Ui) {
        let enabled = self.plugin_prefs.openmontage_enabled;
        let status = self.openmontage.status.clone();
        let busy = self.openmontage.busy;
        let root_display = self.openmontage.root.display().to_string();
        let last_step = self.openmontage.last_step.clone();
        let fail_reason = match &status {
            OpenMontageStatus::InstallFailed(r) => Some(r.clone()),
            _ => None,
        };
        let missing_deps = match &status {
            OpenMontageStatus::MissingDeps(m) => Some(m.join("、")),
            _ => None,
        };
        let (status_label, status_color) = match &status {
            OpenMontageStatus::Ready => (self.t("plugins.openmontage_status_ready").to_string(), OK),
            OpenMontageStatus::NotInstalled | OpenMontageStatus::Unknown => {
                (self.t("plugins.openmontage_status_missing").to_string(), MUTED)
            }
            OpenMontageStatus::Installing => {
                (self.t("plugins.openmontage_status_installing").to_string(), OM_ACCENT)
            }
            OpenMontageStatus::InstallFailed(_) => {
                (self.t("plugins.openmontage_status_failed").to_string(), DANGER)
            }
            OpenMontageStatus::MissingDeps(_) => {
                (self.t("plugins.openmontage_status_deps").to_string(), DANGER)
            }
        };
        let title = self.t("plugins.openmontage_title").to_string();
        let blurb = self.t("plugins.openmontage_blurb").to_string();
        let path_label = self.t("plugins.openmontage_path").to_string();
        let change_label = self.t("plugins.openmontage_change").to_string();
        let enable_tip = self.t("plugins.enable").to_string();
        let prereq = self.t("plugins.openmontage_prereq").to_string();
        let install_label = self.t("plugins.openmontage_install").to_string();
        let installing_hint = self.t("plugins.openmontage_installing_hint").to_string();
        let retry_label = self.t("plugins.openmontage_retry").to_string();
        let reinstall_label = self.t("plugins.openmontage_reinstall_deps").to_string();
        let backlot_label = self.t("plugins.openmontage_backlot").to_string();
        let enabled_hint = self.t("plugins.openmontage_enabled_hint").to_string();
        let icon_color = if enabled && status.is_ready() {
            OM_ACCENT
        } else {
            MUTED
        };

        plugin_section(ui, |ui| {
            plugin_header(
                ui,
                SidebarGlyph::Plug,
                icon_color,
                &title,
                &blurb,
                if status.is_ready() {
                    Some((enabled, OM_ACCENT, enable_tip.as_str()))
                } else {
                    None
                },
                |on| self.set_openmontage_enabled(on),
            );

            ui.add_space(10.0);
            plugin_path_row(
                ui,
                &path_label,
                &root_display,
                (!busy).then_some(change_label.as_str()),
                || self.pick_openmontage_root(),
            );

            ui.add_space(6.0);
            plugin_status_line(
                ui,
                status_color,
                &status_label,
                if !last_step.is_empty() && busy {
                    Some(last_step.as_str())
                } else {
                    None
                },
            );

            match &status {
                OpenMontageStatus::NotInstalled | OpenMontageStatus::Unknown => {
                    ui.add_space(10.0);
                    ui.label(RichText::new(&prereq).size(12.0).color(MUTED));
                    ui.add_space(10.0);
                    if ui
                        .add_enabled(!busy, plugin_primary_btn_widget(&install_label, OM_ACCENT))
                        .clicked()
                    {
                        self.openmontage.start_install(false);
                    }
                }
                OpenMontageStatus::Installing => {
                    ui.add_space(8.0);
                    ui.label(RichText::new(&installing_hint).size(12.0).color(MUTED));
                    self.openmontage_log_tail(ui);
                }
                OpenMontageStatus::InstallFailed(_) => {
                    ui.add_space(8.0);
                    if let Some(reason) = &fail_reason {
                        ui.label(RichText::new(reason).size(12.0).color(DANGER));
                    }
                    self.openmontage_log_tail(ui);
                    ui.add_space(10.0);
                    if plugin_secondary_btn(ui, &retry_label).clicked() {
                        self.openmontage.start_install(false);
                    }
                }
                OpenMontageStatus::MissingDeps(_) => {
                    ui.add_space(8.0);
                    if let Some(missing) = &missing_deps {
                        ui.label(
                            RichText::new(format!("缺少：{missing}"))
                                .size(12.0)
                                .color(DANGER),
                        );
                    }
                    ui.add_space(10.0);
                    if ui
                        .add_enabled(!busy, plugin_primary_btn_widget(&reinstall_label, OM_ACCENT))
                        .clicked()
                    {
                        self.openmontage.start_install(true);
                    }
                }
                OpenMontageStatus::Ready => {
                    if enabled {
                        ui.add_space(12.0);
                        if plugin_secondary_btn(ui, &backlot_label).clicked() {
                            self.openmontage.open_backlot();
                        }
                    }
                    ui.add_space(8.0);
                    ui.label(RichText::new(&enabled_hint).size(12.0).color(MUTED));
                }
            }
        });
    }

    fn openmontage_log_tail(&self, ui: &mut egui::Ui) {
        if self.openmontage.log_tail.is_empty() {
            return;
        }
        ui.add_space(10.0);
        Frame::new()
            .fill(BG)
            .corner_radius(CornerRadius::same(8))
            .inner_margin(Margin::symmetric(10, 8))
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                egui::ScrollArea::vertical()
                    .id_salt("openmontage_log_tail")
                    .max_height(100.0)
                    .stick_to_bottom(true)
                    .show(ui, |ui| {
                        for line in &self.openmontage.log_tail {
                            ui.label(
                                RichText::new(line)
                                    .size(11.0)
                                    .monospace()
                                    .color(MUTED),
                            );
                        }
                    });
            });
    }

    fn unity_install_log_tail(&self, ui: &mut egui::Ui) {
        if self.unity.install_log.is_empty() {
            return;
        }
        ui.add_space(6.0);
        Frame::new()
            .fill(PANEL_2)
            .corner_radius(CornerRadius::same(8))
            .inner_margin(Margin::symmetric(10, 8))
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                egui::ScrollArea::vertical()
                    .max_height(120.0)
                    .stick_to_bottom(true)
                    .show(ui, |ui| {
                        for line in &self.unity.install_log {
                            ui.label(
                                RichText::new(line)
                                    .size(11.0)
                                    .monospace()
                                    .color(MUTED),
                            );
                        }
                    });
            });
    }

    fn unity_docs_window(&mut self, ctx: &egui::Context) {
        if !self.show_unity_docs_window {
            return;
        }
        let mut open = true;
        egui::Window::new("Unity 说明文档")
            .open(&mut open)
            .collapsible(false)
            .resizable(true)
            .default_size([520.0, 480.0])
            .anchor(Align2::CENTER_CENTER, [0.0, 0.0])
            .show(ctx, |ui| {
                egui::ScrollArea::vertical()
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        markdown::render(ui, &unity_chat_help_text(), TEXT);
                    });
            });
        if !open {
            self.show_unity_docs_window = false;
        }
    }

    fn unity_panel(&mut self, ui: &mut egui::Ui) {
        ui.add_space(8.0);
        ui.horizontal(|ui| {
            if ui
                .add(
                    egui::Button::new(RichText::new("← 插件").size(12.0).color(MUTED))
                        .fill(Color32::TRANSPARENT)
                        .frame(false),
                )
                .clicked()
            {
                self.model.main_nav = MainNav::Plugins;
            }
            ui.label(
                RichText::new("对话或按钮都能驱动编辑器：观察 → 行动 → 验证")
                    .size(13.0)
                    .color(MUTED),
            );
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let busy = self.unity.busy || self.unity.is_guiding();
                if ui
                    .add_enabled(
                        !busy,
                        egui::Button::new(
                            RichText::new("跑完整闭环").size(12.5).color(BG).strong(),
                        )
                        .fill(UNITY_ACCENT)
                        .corner_radius(CornerRadius::same(8)),
                    )
                    .on_hover_text("复现博文演示：观察禁用碰撞体 → 热修复 → Play 验证")
                    .clicked()
                {
                    self.unity.run_action(UnityAction::RunFullLoop);
                }
                if self.unity.can_stop()
                    && ui
                        .button(RichText::new("停止").size(12.0).color(DANGER))
                        .on_hover_text("取消当前命令并清空引导队列")
                        .clicked()
                {
                    self.unity.stop();
                }
                if ui
                    .button(RichText::new("打开对话控制").size(12.0).color(UNITY_ACCENT))
                    .on_hover_text("回到聊天，并切换到 Unity CLI 交互")
                    .clicked()
                {
                    self.model.go_chat();
                    self.set_chat_interaction(ChatInteraction::Unity);
                    self.model.focus_composer = true;
                }
                if ui.button(RichText::new("说明").size(12.0)).clicked() {
                    self.show_unity_docs_window = true;
                }
                if ui
                    .add_enabled(
                        !busy,
                        egui::Button::new(RichText::new("在聊天中分析").size(12.0)),
                    )
                    .on_hover_text("切换到聊天并给出简短诊断，不执行 Unity 命令")
                    .clicked()
                {
                    let briefing = self.unity.compact_chat_briefing();
                    self.model.go_chat();
                    self.send_context_prompt("分析当前 Unity 状态", briefing);
                }
            });
        });
        if let Some(guide) = &self.unity.guide_label {
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                ui.spinner();
                ui.label(RichText::new(guide).size(12.5).color(UNITY_ACCENT));
            });
        }
        ui.add_space(12.0);

        self.unity_setup_wizard(ui);
        ui.add_space(14.0);
        self.unity_status_card(ui);
        ui.add_space(14.0);
        self.unity_pipeline_card(ui);
        ui.add_space(14.0);
        self.unity_scene_card(ui);
        ui.add_space(14.0);
        self.unity_loop_card(ui);
        ui.add_space(14.0);
        self.unity_actions_card(ui);
        ui.add_space(14.0);
        self.unity_history_card(ui);
        ui.add_space(24.0);
    }

    fn unity_setup_wizard(&mut self, ui: &mut egui::Ui) {
        let busy = self.unity.busy || self.unity.is_guiding();
        let focus = self.unity.focused_setup_step();
        Frame::new()
            .fill(PANEL)
            .stroke(Stroke::new(1.5, UNITY_ACCENT))
            .corner_radius(CornerRadius::same(12))
            .inner_margin(Margin::same(14))
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.horizontal(|ui| {
                    ui.label(
                        RichText::new("引导执行")
                            .size(15.0)
                            .strong()
                            .color(TEXT),
                    );
                    ui.label(
                        RichText::new(format!(
                            "步骤 {}/{}",
                            self.unity.setup_step.index() + 1,
                            SetupStep::ALL.len()
                        ))
                        .size(12.0)
                        .color(UNITY_ACCENT),
                    );
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui.small_button("跟随推荐步骤").clicked() {
                            self.unity.setup_focus = None;
                            self.unity.sync_setup_step();
                        }
                    });
                });
                ui.add_space(4.0);
                ui.label(
                    RichText::new("按顺序完成：安装 CLI → 检测 → 确认项目 → Pipeline → 探测编辑器 → 闭环")
                        .size(12.0)
                        .color(MUTED),
                );
                ui.add_space(10.0);

                // Step rail
                ui.horizontal_wrapped(|ui| {
                    for step in SetupStep::ALL {
                        let state = self.unity.step_state(step);
                        let selected = focus == step;
                        let (fill, stroke, text_color) = match state {
                            StepState::Done => (
                                Color32::from_rgb(36, 56, 44),
                                Stroke::new(1.0, OK),
                                OK,
                            ),
                            StepState::Current => (
                                PANEL_2,
                                Stroke::new(1.5, UNITY_ACCENT),
                                UNITY_ACCENT,
                            ),
                            StepState::Locked => (
                                PANEL,
                                Stroke::new(1.0, BORDER),
                                MUTED,
                            ),
                        };
                        let label = format!(
                            "{} {}",
                            match state {
                                StepState::Done => "✓",
                                StepState::Current => "●",
                                StepState::Locked => "○",
                            },
                            step.title()
                        );
                        let resp = ui.add(
                            egui::Button::new(RichText::new(label).size(11.5).color(text_color))
                                .fill(if selected { PANEL_2 } else { fill })
                                .stroke(if selected {
                                    Stroke::new(1.5, ACCENT)
                                } else {
                                    stroke
                                })
                                .corner_radius(CornerRadius::same(8))
                                .min_size(Vec2::new(0.0, 28.0)),
                        );
                        if resp.clicked() {
                            self.unity.setup_focus = Some(step);
                        }
                    }
                });

                ui.add_space(12.0);
                Frame::new()
                    .fill(PANEL_2)
                    .corner_radius(CornerRadius::same(10))
                    .inner_margin(Margin::same(12))
                    .show(ui, |ui| {
                        ui.set_width(ui.available_width());
                        ui.label(
                            RichText::new(focus.title())
                                .size(14.0)
                                .strong()
                                .color(TEXT),
                        );
                        ui.add_space(4.0);
                        ui.label(RichText::new(focus.blurb()).size(12.5).color(MUTED));

                        match focus {
                            SetupStep::InstallCli => {
                                ui.add_space(8.0);
                                let hint = UnityState::install_hint();
                                ui.label(
                                    RichText::new(hint).size(11.5).monospace().color(TEXT),
                                );
                                ui.add_space(8.0);
                                ui.horizontal(|ui| {
                                    if ui
                                        .add(
                                            egui::Button::new(
                                                RichText::new("① 复制安装命令")
                                                    .size(12.5)
                                                    .color(BG)
                                                    .strong(),
                                            )
                                            .fill(UNITY_ACCENT)
                                            .min_size(Vec2::new(0.0, 32.0))
                                            .corner_radius(CornerRadius::same(8)),
                                        )
                                        .clicked()
                                    {
                                        ui.ctx().copy_text(hint.to_string());
                                        self.unity.advance_after_cli_install_copied();
                                    }
                                    if ui
                                        .add_enabled(
                                            !busy,
                                            egui::Button::new(
                                                RichText::new("② 我已安装，重新检测")
                                                    .size(12.5)
                                                    .color(TEXT),
                                            )
                                            .fill(PANEL)
                                            .stroke(Stroke::new(1.0, BORDER))
                                            .min_size(Vec2::new(0.0, 32.0))
                                            .corner_radius(CornerRadius::same(8)),
                                        )
                                        .clicked()
                                    {
                                        self.unity.setup_focus = Some(SetupStep::DetectCli);
                                        self.unity.run_action(UnityAction::RefreshDetect);
                                    }
                                });
                                ui.add_space(6.0);
                                ui.label(
                                    RichText::new(
                                        "在外部 PowerShell 粘贴执行安装脚本，完成后点②。安装过程可能需 1–2 分钟。",
                                    )
                                    .size(11.5)
                                    .color(MUTED),
                                );
                                ui.add_space(10.0);
                                ui.separator();
                                ui.add_space(6.0);
                                ui.label(
                                    RichText::new("或者，直接在应用内自动安装（无需手动打开终端）：")
                                        .size(11.5)
                                        .color(MUTED),
                                );
                                ui.add_space(6.0);
                                ui.horizontal(|ui| {
                                    if ui
                                        .add_enabled(
                                            self.unity.can_install_cli(),
                                            egui::Button::new(
                                                RichText::new("⚡ 一键自动安装")
                                                    .size(12.5)
                                                    .color(BG)
                                                    .strong(),
                                            )
                                            .fill(OK)
                                            .min_size(Vec2::new(0.0, 32.0))
                                            .corner_radius(CornerRadius::same(8)),
                                        )
                                        .on_hover_text("应用内运行官方安装脚本并自动重新检测")
                                        .clicked()
                                    {
                                        self.unity.install_cli();
                                    }
                                    if self.unity.status == CliStatus::Installing {
                                        ui.spinner();
                                        ui.label(
                                            RichText::new("正在安装…")
                                                .size(12.0)
                                                .color(UNITY_ACCENT),
                                        );
                                    }
                                });
                                self.unity_install_log_tail(ui);
                            }
                            SetupStep::DetectCli => {
                                ui.add_space(8.0);
                                ui.horizontal(|ui| {
                                    if ui
                                        .add_enabled(
                                            !busy,
                                            egui::Button::new(
                                                RichText::new("重新检测 CLI")
                                                    .size(12.5)
                                                    .color(BG)
                                                    .strong(),
                                            )
                                            .fill(UNITY_ACCENT)
                                            .min_size(Vec2::new(0.0, 32.0))
                                            .corner_radius(CornerRadius::same(8)),
                                        )
                                        .clicked()
                                    {
                                        self.unity.run_action(UnityAction::RefreshDetect);
                                    }
                                    if self.unity.status == CliStatus::Ready {
                                        ui.label(
                                            RichText::new("已就绪，可进入下一步")
                                                .size(12.5)
                                                .color(OK),
                                        );
                                    }
                                });
                            }
                            SetupStep::PickProject => {
                                ui.add_space(8.0);
                                let is_unity = crate::unity::is_unity_project_root(
                                    &self.unity.project_path,
                                );
                                ui.label(
                                    RichText::new(format!(
                                        "当前绑定：{}",
                                        self.unity.project_path.display()
                                    ))
                                    .size(12.0)
                                    .monospace()
                                    .color(if is_unity { TEXT } else { DANGER }),
                                );
                                if !is_unity {
                                    ui.add_space(4.0);
                                    ui.label(
                                        RichText::new(
                                            "这是 agent 任务目录或其它非 Unity 路径，不能用于 pipeline install。请改选工程根。",
                                        )
                                        .size(12.0)
                                        .color(DANGER),
                                    );
                                }
                                ui.add_space(8.0);
                                ui.horizontal(|ui| {
                                    if ui
                                        .add(
                                            egui::Button::new(
                                                RichText::new("选择 Unity 工程根目录…")
                                                    .size(12.5)
                                                    .color(BG)
                                                    .strong(),
                                            )
                                            .fill(UNITY_ACCENT)
                                            .min_size(Vec2::new(0.0, 32.0))
                                            .corner_radius(CornerRadius::same(8)),
                                        )
                                        .clicked()
                                    {
                                        self.pick_unity_project(ui.ctx());
                                    }
                                });
                                ui.add_space(6.0);
                                ui.label(
                                    RichText::new(
                                        "例：C:\\Users\\…\\设置指南编辑器嵌入式教程（不要选到 Assets\\子目录，也不要用 .bony-worktrees\\task-*）",
                                    )
                                    .size(11.5)
                                    .color(MUTED),
                                );
                            }
                            SetupStep::InstallPipeline
                            | SetupStep::ProbeEditor
                            | SetupStep::RunLoop => {
                                ui.add_space(8.0);
                                if matches!(focus, SetupStep::ProbeEditor) {
                                    ui.label(
                                        RichText::new(
                                            "请先用 Unity 6.0+ 打开同一项目并等待编译完成，再点探测。",
                                        )
                                        .size(12.0)
                                        .color(MUTED),
                                    );
                                    ui.add_space(6.0);
                                }
                                let label = focus.primary_label();
                                if ui
                                    .add_enabled(
                                        !busy,
                                        egui::Button::new(
                                            RichText::new(label)
                                                .size(13.0)
                                                .color(BG)
                                                .strong(),
                                        )
                                        .fill(UNITY_ACCENT)
                                        .min_size(Vec2::new(160.0, 34.0))
                                        .corner_radius(CornerRadius::same(8)),
                                    )
                                    .clicked()
                                {
                                    self.unity.run_setup_primary();
                                }
                                if busy {
                                    ui.add_space(6.0);
                                    ui.horizontal(|ui| {
                                        ui.spinner();
                                        ui.label(
                                            RichText::new("执行中，请稍候…")
                                                .size(12.0)
                                                .color(MUTED),
                                        );
                                    });
                                }
                            }
                        }
                    });
            });
    }

    fn unity_pipeline_card(&mut self, ui: &mut egui::Ui) {
        let busy = self.unity.busy || self.unity.is_guiding();
        let installing = self.unity.pipeline_status == PipelineStatus::Installing;
        Frame::new()
            .fill(PANEL)
            .stroke(Stroke::new(
                1.0,
                if self.unity.pipeline_ready_for_commands() {
                    Color32::from_rgb(50, 90, 70)
                } else {
                    BORDER
                },
            ))
            .corner_radius(CornerRadius::same(12))
            .inner_margin(Margin::same(14))
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.horizontal(|ui| {
                    ui.label(
                        RichText::new("Pipeline · command / eval 前提")
                            .size(14.0)
                            .strong()
                            .color(TEXT),
                    );
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        let status_color = match self.unity.pipeline_status {
                            PipelineStatus::Installed => OK,
                            PipelineStatus::Installing
                            | PipelineStatus::PendingImport
                            | PipelineStatus::Checking => UNITY_ACCENT,
                            PipelineStatus::NotInstalled | PipelineStatus::Error => DANGER,
                            PipelineStatus::Unknown => MUTED,
                        };
                        ui.label(
                            RichText::new(self.unity.pipeline_status.label())
                                .size(12.0)
                                .color(status_color),
                        );
                    });
                });
                ui.add_space(4.0);
                ui.label(
                    RichText::new(
                        "编辑器要响应 unity command / eval，需先在项目中安装 com.unity.pipeline",
                    )
                    .size(12.0)
                    .color(MUTED),
                );
                ui.add_space(10.0);

                for (label, ok, detail) in self.unity.checklist() {
                    ui.horizontal_wrapped(|ui| {
                        ui.label(
                            RichText::new(if ok { "●" } else { "○" })
                                .size(12.0)
                                .color(if ok { OK } else { MUTED }),
                        );
                        ui.label(RichText::new(label).size(12.5).strong().color(TEXT));
                    });
                    ui.add(
                        egui::Label::new(
                            RichText::new(detail).size(11.5).color(MUTED).monospace(),
                        )
                        .wrap(),
                    );
                    ui.add_space(3.0);
                }

                ui.add_space(10.0);
                ui.horizontal_wrapped(|ui| {
                    if !crate::unity::is_unity_project_root(&self.unity.project_path)
                        && ui
                            .add(
                                egui::Button::new(
                                    RichText::new("先选 Unity 工程…")
                                        .size(12.5)
                                        .color(BG)
                                        .strong(),
                                )
                                .fill(DANGER)
                                .min_size(Vec2::new(0.0, 32.0))
                                .corner_radius(CornerRadius::same(8)),
                            )
                            .clicked()
                    {
                        self.pick_unity_project(ui.ctx());
                    }
                    if ui
                        .add_enabled(
                            !busy,
                            egui::Button::new(
                                RichText::new(if installing {
                                    "安装中…"
                                } else if self.unity.pipeline_status == PipelineStatus::PendingImport {
                                    "等待 Unity 加载"
                                } else {
                                    "安装 Pipeline"
                                })
                                .size(12.5)
                                .color(BG)
                                .strong(),
                            )
                            .fill(UNITY_ACCENT)
                            .min_size(Vec2::new(0.0, 32.0))
                            .corner_radius(CornerRadius::same(8)),
                        )
                        .on_hover_text("在当前项目目录执行：unity pipeline install")
                        .clicked()
                    {
                        self.unity.run_action(UnityAction::InstallPipeline);
                    }
                    if ui
                        .add_enabled(
                            !busy,
                            egui::Button::new(RichText::new("刷新列表").size(12.5).color(TEXT))
                                .fill(PANEL_2)
                                .min_size(Vec2::new(0.0, 32.0))
                                .corner_radius(CornerRadius::same(8)),
                        )
                        .on_hover_text("unity pipeline list")
                        .clicked()
                    {
                        self.unity.run_action(UnityAction::ListPipeline);
                    }
                    if ui
                        .add_enabled(
                            !busy,
                            egui::Button::new(RichText::new("探测编辑器").size(12.5).color(TEXT))
                                .fill(PANEL_2)
                                .min_size(Vec2::new(0.0, 32.0))
                                .corner_radius(CornerRadius::same(8)),
                        )
                        .on_hover_text("unity command --project-path=…（需编辑器已打开项目）")
                        .clicked()
                    {
                        self.unity.run_action(UnityAction::ProbeEditor);
                    }
                });

                ui.add_space(8.0);
                let link_color = match self.unity.editor_link {
                    EditorLinkStatus::Connected => OK,
                    EditorLinkStatus::Disconnected | EditorLinkStatus::Checking => UNITY_ACCENT,
                    EditorLinkStatus::Unknown => MUTED,
                };
                ui.label(
                    RichText::new(format!(
                        "编辑器：{} · {}",
                        self.unity.editor_link.label(),
                        self.unity.commands_summary
                    ))
                    .size(12.0)
                    .color(link_color),
                );

                if !self.unity.pipeline_detail.trim().is_empty() {
                    ui.add_space(6.0);
                    egui::CollapsingHeader::new(
                        RichText::new("Pipeline 输出").size(11.5).color(MUTED),
                    )
                    .id_salt("unity_pipeline_detail")
                    .show(ui, |ui| {
                        ui.add(
                            egui::Label::new(
                                RichText::new(&self.unity.pipeline_detail).monospace(),
                            )
                            .wrap(),
                        );
                    });
                }

                ui.add_space(8.0);
                Frame::new()
                    .fill(PANEL_2)
                    .corner_radius(CornerRadius::same(8))
                    .inner_margin(Margin::same(10))
                    .show(ui, |ui| {
                        ui.set_width(ui.available_width());
                        ui.label(
                            RichText::new("步骤提示")
                                .size(12.0)
                                .strong()
                                .color(TEXT),
                        );
                        ui.label(
                            RichText::new(
                                "1. 用 Unity 6.0 LTS 或更新版本打开同一工程  ·  2. 安装 Pipeline  ·  3. 等 Package Manager 下载并完成脚本编译  ·  4. 探测编辑器  ·  5. 仅当 eval 返回未授权时再执行 unity auth login",
                            )
                            .size(11.5)
                            .color(MUTED),
                        );
                    });
            });
    }

    fn unity_status_card(&mut self, ui: &mut egui::Ui) {
        Frame::new()
            .fill(PANEL)
            .stroke(Stroke::new(1.0, BORDER))
            .corner_radius(CornerRadius::same(12))
            .inner_margin(Margin::same(14))
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.horizontal(|ui| {
                    ui.label(RichText::new("CLI 状态").size(14.0).strong().color(TEXT));
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        let busy = self.unity.busy;
                        if ui
                            .add_enabled(
                                !busy,
                                egui::Button::new(RichText::new("重新检测").size(12.0)),
                            )
                            .clicked()
                        {
                            self.unity.run_action(UnityAction::RefreshDetect);
                        }
                    });
                });
                ui.add_space(8.0);

                let path_text = self
                    .unity
                    .cli_path
                    .as_ref()
                    .map(|p| p.display().to_string())
                    .unwrap_or_else(|| "未找到 unity 二进制".into());
                ui.label(RichText::new(path_text).size(12.5).color(MUTED).monospace());
                if !self.unity.version_line.is_empty() {
                    ui.add_space(4.0);
                    ui.label(
                        RichText::new(&self.unity.version_line)
                            .size(12.0)
                            .color(TEXT)
                            .monospace(),
                    );
                }
                if let Some(err) = &self.unity.last_error {
                    ui.add_space(4.0);
                    ui.label(RichText::new(err).size(12.0).color(DANGER));
                }

                ui.add_space(10.0);
                for (label, value) in [
                    ("编辑器", self.unity.editors_summary.as_str()),
                    ("Pipeline", self.unity.pipeline_summary.as_str()),
                    ("已注册命令", self.unity.commands_summary.as_str()),
                ] {
                    ui.label(RichText::new(label).size(12.0).color(MUTED));
                    ui.add(egui::Label::new(RichText::new(value).size(12.5).color(TEXT)).wrap());
                    ui.add_space(6.0);
                }

                if matches!(self.unity.status, CliStatus::Missing | CliStatus::Error) {
                    ui.add_space(12.0);
                    ui.label(
                        RichText::new("本机未检测到 Unity CLI，可先装 beta 通道：")
                            .size(12.5)
                            .color(MUTED),
                    );
                    ui.add_space(6.0);
                    let hint = UnityState::install_hint();
                    Frame::new()
                        .fill(PANEL_2)
                        .corner_radius(CornerRadius::same(8))
                        .inner_margin(Margin::same(10))
                        .show(ui, |ui| {
                            ui.set_width(ui.available_width());
                            ui.horizontal(|ui| {
                                ui.label(RichText::new(hint).size(11.5).monospace().color(TEXT));
                                if ui.small_button("复制").clicked() {
                                    ui.ctx().copy_text(hint.to_string());
                                    self.unity.toast = Some("安装命令已复制".into());
                                }
                            });
                        });
                    ui.add_space(8.0);
                    ui.horizontal(|ui| {
                        if ui
                            .add_enabled(
                                self.unity.can_install_cli(),
                                egui::Button::new(
                                    RichText::new("⚡ 一键自动安装")
                                        .size(12.5)
                                        .color(BG)
                                        .strong(),
                                )
                                .fill(OK)
                                .corner_radius(CornerRadius::same(8)),
                            )
                            .on_hover_text("应用内运行官方安装脚本并自动重新检测")
                            .clicked()
                        {
                            self.unity.install_cli();
                        }
                        if self.unity.status == CliStatus::Installing {
                            ui.spinner();
                            ui.label(RichText::new("正在安装…").size(12.0).color(UNITY_ACCENT));
                        }
                    });
                    self.unity_install_log_tail(ui);
                    ui.add_space(8.0);
                    ui.label(
                        RichText::new("未安装时下方操作为演示模式，可预览 AI 闭环可视化。")
                            .size(12.0)
                            .color(UNITY_ACCENT),
                    );
                }
            });
    }

    fn unity_scene_card(&mut self, ui: &mut egui::Ui) {
        let scene = self.unity.scene.clone();
        Frame::new()
            .fill(PANEL)
            .stroke(Stroke::new(1.0, BORDER))
            .corner_radius(CornerRadius::same(12))
            .inner_margin(Margin::same(14))
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.horizontal(|ui| {
                    ui.label(RichText::new("场景快照").size(14.0).strong().color(TEXT));
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui.small_button("重置").clicked() {
                            self.unity.reset_scene();
                        }
                    });
                });
                ui.add_space(4.0);
                ui.label(RichText::new(&scene.note).size(12.5).color(MUTED));
                ui.add_space(10.0);

                let (response, painter) = ui
                    .allocate_painter(Vec2::new(ui.available_width(), 150.0), egui::Sense::hover());
                let rect = response.rect;
                painter.rect_filled(rect, CornerRadius::same(10), PANEL_2);
                painter.rect_stroke(
                    rect,
                    CornerRadius::same(10),
                    Stroke::new(1.0, BORDER),
                    egui::StrokeKind::Outside,
                );

                // Ground
                let ground_y = rect.bottom() - 36.0;
                let ground_color = if scene.ground_collider_enabled {
                    Color32::from_rgb(70, 140, 100)
                } else {
                    Color32::from_rgb(90, 70, 70)
                };
                painter.rect_filled(
                    egui::Rect::from_min_max(
                        egui::pos2(rect.left() + 24.0, ground_y),
                        egui::pos2(rect.right() - 24.0, ground_y + 14.0),
                    ),
                    CornerRadius::same(4),
                    ground_color,
                );
                painter.text(
                    egui::pos2(rect.left() + 32.0, ground_y + 1.0),
                    egui::Align2::LEFT_TOP,
                    if scene.ground_collider_enabled {
                        "GroundCollider ON"
                    } else {
                        "GroundCollider OFF"
                    },
                    egui::FontId::proportional(11.0),
                    TEXT,
                );

                // Player
                let player_size = Vec2::new(22.0, 28.0);
                let cx = rect.center().x;
                let floor = ground_y - 2.0;
                let py = if scene.ground_collider_enabled {
                    floor - player_size.y - scene.player_y.max(0.0) * 8.0
                } else {
                    // Falling below ground
                    floor - player_size.y + (-scene.player_y).clamp(0.0, 4.0) * 18.0
                };
                let player_rect = egui::Rect::from_center_size(
                    egui::pos2(cx, py + player_size.y * 0.5),
                    player_size,
                );
                painter.rect_filled(player_rect, CornerRadius::same(5), UNITY_ACCENT);
                painter.text(
                    egui::pos2(cx, player_rect.top() - 4.0),
                    egui::Align2::CENTER_BOTTOM,
                    "Player",
                    egui::FontId::proportional(11.0),
                    MUTED,
                );

                // Play badge
                let play_label = if scene.is_playing { "PLAY" } else { "EDIT" };
                let play_color = if scene.is_playing { OK } else { MUTED };
                painter.text(
                    egui::pos2(rect.right() - 16.0, rect.top() + 12.0),
                    egui::Align2::RIGHT_TOP,
                    play_label,
                    egui::FontId::proportional(12.0),
                    play_color,
                );

                ui.add_space(8.0);
                ui.label(
                    RichText::new(scene.status_line())
                        .size(12.0)
                        .monospace()
                        .color(TEXT),
                );
                ui.label(
                    RichText::new(format!("last eval → {}", scene.last_eval_result))
                        .size(11.5)
                        .monospace()
                        .color(MUTED),
                );
            });
    }

    fn unity_loop_card(&mut self, ui: &mut egui::Ui) {
        Frame::new()
            .fill(PANEL)
            .stroke(Stroke::new(1.0, BORDER))
            .corner_radius(CornerRadius::same(12))
            .inner_margin(Margin::same(14))
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.label(RichText::new("AI 反馈闭环").size(14.0).strong().color(TEXT));
                ui.add_space(4.0);
                ui.label(
                    RichText::new("对应 Unity CLI + com.unity.pipeline + command eval")
                        .size(12.0)
                        .color(MUTED),
                );
                ui.add_space(12.0);

                let phases = [LoopPhase::Observe, LoopPhase::Act, LoopPhase::Verify];
                let avail = ui.available_width();
                let gap = 10.0;
                let cell_w = ((avail - gap * 2.0) / 3.0).max(120.0);
                ui.horizontal(|ui| {
                    for (i, phase) in phases.iter().enumerate() {
                        let active = self.unity.loop_phase == *phase;
                        let stroke = if active {
                            Stroke::new(1.5, UNITY_ACCENT)
                        } else {
                            Stroke::new(1.0, BORDER)
                        };
                        let fill = if active { PANEL_2 } else { PANEL };
                        Frame::new()
                            .fill(fill)
                            .stroke(stroke)
                            .corner_radius(CornerRadius::same(10))
                            .inner_margin(Margin::same(10))
                            .show(ui, |ui| {
                                ui.set_width(cell_w - 4.0);
                                ui.label(
                                    RichText::new(format!("0{}", i + 1))
                                        .size(11.0)
                                        .color(if active { UNITY_ACCENT } else { MUTED }),
                                );
                                ui.label(
                                    RichText::new(phase.label()).size(15.0).strong().color(TEXT),
                                );
                                ui.add_space(4.0);
                                ui.label(RichText::new(phase.blurb()).size(11.5).color(MUTED));
                            });
                        if i + 1 < phases.len() {
                            ui.add_space(gap);
                        }
                    }
                });
            });
    }

    fn unity_actions_card(&mut self, ui: &mut egui::Ui) {
        let busy = self.unity.busy || self.unity.is_guiding();
        let need_pipeline = !self.unity.pipeline_ready_for_commands();
        Frame::new()
            .fill(PANEL)
            .stroke(Stroke::new(1.0, BORDER))
            .corner_radius(CornerRadius::same(12))
            .inner_margin(Margin::same(14))
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.label(RichText::new("可视化操作").size(14.0).strong().color(TEXT));
                ui.add_space(8.0);

                ui.label(RichText::new("创作").size(12.0).color(MUTED));
                ui.add_space(4.0);
                ui.horizontal_wrapped(|ui| {
                    for action in [
                        UnityAction::ScaffoldMiniGame,
                        UnityAction::ScaffoldRpg,
                        UnityAction::ScaffoldMmo,
                        UnityAction::ScaffoldRoguelike,
                        UnityAction::SetupSkyDay,
                        UnityAction::SetupSkySunset,
                        UnityAction::SetupSkyNight,
                        UnityAction::CreateGround,
                        UnityAction::CreateDirectionalLight,
                        UnityAction::SetupMainCamera,
                        UnityAction::CreatePlayerCapsule,
                        UnityAction::CreateNpc,
                        UnityAction::CreateNpcVendor,
                        UnityAction::CreateNpcQuest,
                        UnityAction::CreateSpawnPoint,
                        UnityAction::CreatePortalZone,
                        UnityAction::CreateEnemySpawn,
                        UnityAction::EnableNpcAi,
                        UnityAction::InstallNpcAi,
                        UnityAction::AttachNpcAi,
                        UnityAction::LayoutRpg,
                        UnityAction::LayoutMmo,
                        UnityAction::LayoutRoguelike,
                        UnityAction::SaveNamedScene,
                        UnityAction::NewScene,
                    ] {
                        let emphasize = matches!(
                            action,
                            UnityAction::ScaffoldMiniGame
                                | UnityAction::ScaffoldRpg
                                | UnityAction::ScaffoldMmo
                                | UnityAction::ScaffoldRoguelike
                                | UnityAction::EnableNpcAi
                        );
                        if ui
                            .add_enabled(
                                !busy,
                                egui::Button::new(
                                    RichText::new(action.label())
                                        .size(12.5)
                                        .color(if emphasize { BG } else { TEXT }),
                                )
                                .fill(if emphasize { UNITY_ACCENT } else { PANEL_2 })
                                .min_size(Vec2::new(0.0, 30.0))
                                .corner_radius(CornerRadius::same(8)),
                            )
                            .clicked()
                        {
                            self.unity.run_action(action);
                        }
                    }
                });

                ui.add_space(10.0);
                ui.label(RichText::new("快捷操作").size(12.0).color(MUTED));
                ui.add_space(4.0);
                ui.horizontal_wrapped(|ui| {
                    for action in [
                        UnityAction::SaveScene,
                        UnityAction::RefreshAssets,
                        UnityAction::RequestScriptReload,
                        UnityAction::ClearConsole,
                        UnityAction::UndoLast,
                        UnityAction::RedoLast,
                        UnityAction::EnterPlayMode,
                        UnityAction::ExitPlayMode,
                        UnityAction::PausePlayMode,
                        UnityAction::StepPlayMode,
                        UnityAction::FrameSelection,
                        UnityAction::FocusGameView,
                        UnityAction::FocusSceneView,
                        UnityAction::DuplicateSelection,
                        UnityAction::DeleteSelection,
                    ] {
                        if ui
                            .add_enabled(
                                !busy,
                                egui::Button::new(RichText::new(action.label()).size(12.5).color(TEXT))
                                    .fill(PANEL_2)
                                    .min_size(Vec2::new(0.0, 30.0))
                                    .corner_radius(CornerRadius::same(8)),
                            )
                            .clicked()
                        {
                            self.unity.run_action(action);
                        }
                    }
                });

                ui.add_space(10.0);
                ui.label(RichText::new("场景 / 对象").size(12.0).color(MUTED));
                ui.add_space(4.0);
                ui.horizontal_wrapped(|ui| {
                    for action in [
                        UnityAction::ListScenes,
                        UnityAction::ActiveSceneInfo,
                        UnityAction::NewScene,
                        UnityAction::LoadFirstScene,
                        UnityAction::HierarchyRoots,
                        UnityAction::CreatePlane,
                        UnityAction::CreateDirectionalLight,
                    ] {
                        if ui
                            .add_enabled(
                                !busy,
                                egui::Button::new(RichText::new(action.label()).size(12.5).color(TEXT))
                                    .fill(PANEL_2)
                                    .min_size(Vec2::new(0.0, 30.0))
                                    .corner_radius(CornerRadius::same(8)),
                            )
                            .clicked()
                        {
                            self.unity.run_action(action);
                        }
                    }
                });

                ui.add_space(10.0);
                ui.label(RichText::new("资源 / 包").size(12.0).color(MUTED));
                ui.add_space(4.0);
                ui.horizontal_wrapped(|ui| {
                    for action in [
                        UnityAction::SaveAssets,
                        UnityAction::FindAssets,
                        UnityAction::ConsoleErrors,
                        UnityAction::FindMissingScripts,
                        UnityAction::ListPackages,
                        UnityAction::AddPackage,
                    ] {
                        if ui
                            .add_enabled(
                                !busy,
                                egui::Button::new(RichText::new(action.label()).size(12.5).color(TEXT))
                                    .fill(PANEL_2)
                                    .min_size(Vec2::new(0.0, 30.0))
                                    .corner_radius(CornerRadius::same(8)),
                            )
                            .clicked()
                        {
                            self.unity.run_action(action);
                        }
                    }
                });

                ui.add_space(10.0);
                ui.label(RichText::new("工程连接").size(12.0).color(MUTED));
                ui.add_space(4.0);
                ui.horizontal_wrapped(|ui| {
                    for action in [
                        UnityAction::EditorStatus,
                        UnityAction::ListProjects,
                        UnityAction::ProjectInfo,
                        UnityAction::OpenProject,
                        UnityAction::RegisterProject,
                        UnityAction::PinProject,
                        UnityAction::RequireEditor,
                        UnityAction::ProbeEditor,
                        UnityAction::ListEditors,
                        UnityAction::ListPipeline,
                        UnityAction::InstallPipeline,
                        UnityAction::UpgradePipeline,
                        UnityAction::ListLtsReleases,
                        UnityAction::HubLogs,
                        UnityAction::CacheInfo,
                    ] {
                        let emphasize =
                            matches!(action, UnityAction::InstallPipeline) && need_pipeline;
                        if ui
                            .add_enabled(
                                !busy,
                                egui::Button::new(
                                    RichText::new(action.label())
                                        .size(12.5)
                                        .color(if emphasize { BG } else { TEXT }),
                                )
                                .fill(if emphasize { UNITY_ACCENT } else { PANEL_2 })
                                .min_size(Vec2::new(0.0, 30.0))
                                .corner_radius(CornerRadius::same(8)),
                            )
                            .clicked()
                        {
                            self.unity.run_action(action);
                        }
                    }
                });

                ui.add_space(10.0);
                ui.label(RichText::new("测试 / 构建 / 闭环").size(12.0).color(MUTED));
                ui.add_space(4.0);
                ui.horizontal(|ui| {
                    ui.label(RichText::new("闭环对象").size(12.0).color(MUTED));
                    ui.add(
                        egui::TextEdit::singleline(&mut self.unity.loop_object)
                            .desired_width(120.0)
                            .hint_text("Ground"),
                    );
                    if ui
                        .add_enabled(
                            !busy,
                            egui::Button::new(
                                RichText::new(UnityAction::SelectLoopObject.label())
                                    .size(12.5)
                                    .color(TEXT),
                            )
                            .fill(PANEL_2)
                            .min_size(Vec2::new(0.0, 30.0))
                            .corner_radius(CornerRadius::same(8)),
                        )
                        .clicked()
                    {
                        self.unity.run_action(UnityAction::SelectLoopObject);
                    }
                });
                ui.add_space(4.0);
                ui.horizontal_wrapped(|ui| {
                    for action in [
                        UnityAction::RunEditModeTests,
                        UnityAction::RunPlayModeTests,
                        UnityAction::BuildWindowsPlayer,
                        UnityAction::ObserveCollider,
                        UnityAction::FixCollider,
                        UnityAction::RunFullLoop,
                        UnityAction::ListCommands,
                    ] {
                        if ui
                            .add_enabled(
                                !busy,
                                egui::Button::new(RichText::new(action.label()).size(12.5).color(TEXT))
                                    .fill(PANEL_2)
                                    .min_size(Vec2::new(0.0, 30.0))
                                    .corner_radius(CornerRadius::same(8)),
                            )
                            .clicked()
                        {
                            self.unity.run_action(action);
                        }
                    }
                });

                ui.add_space(12.0);
                ui.label(RichText::new("unity command eval").size(12.0).color(MUTED));
                ui.add_space(4.0);
                ui.horizontal_wrapped(|ui| {
                    for (label, expr) in EVAL_PRESETS {
                        if ui
                            .add_enabled(
                                !busy,
                                egui::Button::new(RichText::new(*label).size(11.5).color(MUTED))
                                    .fill(Color32::TRANSPARENT)
                                    .stroke(Stroke::new(1.0, BORDER))
                                    .corner_radius(CornerRadius::same(6)),
                            )
                            .clicked()
                        {
                            self.unity.eval_input = (*expr).into();
                        }
                    }
                });
                ui.add_space(6.0);
                ui.add(
                    egui::TextEdit::multiline(&mut self.unity.eval_input)
                        .desired_width(f32::INFINITY)
                        .desired_rows(3)
                        .font(egui::TextStyle::Monospace)
                        .hint_text("return Application.version;"),
                );
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    if ui
                        .add_enabled(
                            !busy,
                            egui::Button::new(
                                RichText::new("运行 Eval").size(13.0).color(BG).strong(),
                            )
                            .fill(UNITY_ACCENT)
                            .min_size(Vec2::new(110.0, 32.0))
                            .corner_radius(CornerRadius::same(8)),
                        )
                        .clicked()
                    {
                        self.unity.run_action(UnityAction::Eval);
                    }
                    if busy {
                        ui.spinner();
                        ui.label(RichText::new("执行中…").size(12.0).color(MUTED));
                    }
                });
            });
    }

    fn unity_history_card(&mut self, ui: &mut egui::Ui) {
        Frame::new()
            .fill(PANEL)
            .stroke(Stroke::new(1.0, BORDER))
            .corner_radius(CornerRadius::same(12))
            .inner_margin(Margin::same(14))
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.horizontal(|ui| {
                    ui.label(RichText::new("操作时间线").size(14.0).strong().color(TEXT));
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if !self.unity.history.is_empty() && ui.small_button("清空").clicked() {
                            self.unity.clear_history();
                        }
                    });
                });
                ui.add_space(8.0);

                if self.unity.history.is_empty() {
                    ui.label(
                        RichText::new("还没有操作。点「跑完整闭环」或逐步执行观察/修复/验证。")
                            .size(12.5)
                            .color(MUTED),
                    );
                    return;
                }

                let records = self.unity.history.clone();
                for rec in &records {
                    let border = if rec.ok {
                        Stroke::new(1.0, Color32::from_rgb(50, 90, 70))
                    } else {
                        Stroke::new(1.0, Color32::from_rgb(110, 50, 50))
                    };
                    Frame::new()
                        .fill(PANEL_2)
                        .stroke(border)
                        .corner_radius(CornerRadius::same(8))
                        .inner_margin(Margin::same(10))
                        .show(ui, |ui| {
                            ui.set_width(ui.available_width());
                            ui.horizontal(|ui| {
                                ui.label(
                                    RichText::new(rec.phase.label())
                                        .size(11.0)
                                        .color(UNITY_ACCENT),
                                );
                                ui.label(RichText::new(&rec.title).size(13.0).strong().color(TEXT));
                                ui.with_layout(
                                    egui::Layout::right_to_left(egui::Align::Center),
                                    |ui| {
                                        ui.label(
                                            RichText::new(format!("{} ms", rec.elapsed_ms))
                                                .size(11.0)
                                                .color(MUTED),
                                        );
                                        ui.label(
                                            RichText::new(format_relative(rec.at_unix))
                                                .size(10.5)
                                                .color(MUTED),
                                        );
                                        ui.label(
                                            RichText::new(if rec.ok { "OK" } else { "ERR" })
                                                .size(11.0)
                                                .color(if rec.ok { OK } else { DANGER }),
                                        );
                                    },
                                );
                            });
                            ui.add_space(4.0);
                            ui.label(
                                RichText::new(&rec.command)
                                    .size(11.0)
                                    .monospace()
                                    .color(MUTED),
                            );
                            ui.add_space(4.0);
                            ui.label(RichText::new(&rec.summary).size(12.5).color(TEXT));
                            if !rec.detail.trim().is_empty()
                                && rec.detail.trim() != rec.summary.trim()
                            {
                                ui.add_space(6.0);
                                egui::CollapsingHeader::new(
                                    RichText::new("详情").size(11.5).color(MUTED),
                                )
                                .id_salt(format!("unity_op_{}", rec.id))
                                .show(ui, |ui| {
                                    ui.monospace(&rec.detail);
                                });
                            }
                        });
                    ui.add_space(8.0);
                }
            });
    }

    fn about_modal(&mut self, ctx: &egui::Context) {
        if !self.model.show_about {
            return;
        }
        // Dim the app behind the sheet (same pattern as permission / usage modals).
        egui::Area::new(egui::Id::new("about_dim"))
            .order(egui::Order::Middle)
            .interactable(true)
            .fixed_pos(Pos2::ZERO)
            .show(ctx, |ui| {
                let screen = ctx.screen_rect();
                let resp = ui.allocate_rect(screen, egui::Sense::click());
                ui.painter()
                    .rect_filled(screen, 0.0, Color32::from_black_alpha(160));
                if resp.clicked() {
                    self.model.show_about = false;
                }
            });

        let mut open = true;
        egui::Window::new(self.t("about.title"))
            .collapsible(false)
            .resizable(false)
            .title_bar(false)
            .anchor(Align2::CENTER_CENTER, [0.0, 0.0])
            .order(egui::Order::Foreground)
            .frame(
                Frame::new()
                    .fill(PANEL)
                    .corner_radius(CornerRadius::same(16))
                    .stroke(Stroke::new(1.0, BORDER))
                    .inner_margin(Margin::symmetric(22, 18))
                    .shadow(Shadow {
                        offset: [0, 16],
                        blur: 48,
                        spread: 0,
                        color: Color32::from_black_alpha(160),
                    }),
            )
            .open(&mut open)
            .show(ctx, |ui| {
                ui.set_min_width(440.0);
                ui.horizontal(|ui| {
                    ui.label(
                        RichText::new(self.t("app.name"))
                            .size(18.0)
                            .strong()
                            .color(TEXT),
                    );
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if icon_btn(ui, SidebarGlyph::Close, self.t("common.close"), false)
                            .clicked()
                        {
                            self.model.show_about = false;
                        }
                    });
                });
                ui.add_space(6.0);
                ui.label(
                    RichText::new(self.t("about.tagline"))
                        .size(13.0)
                        .color(MUTED),
                );
                ui.add_space(10.0);
                ui.label(
                    RichText::new(format!(
                        "{} {}",
                        self.t("about.version"),
                        env!("CARGO_PKG_VERSION")
                    ))
                    .size(12.5)
                    .color(MUTED),
                );
                ui.add_space(12.0);
                ui.label(
                    RichText::new(self.t("about.body"))
                        .size(12.5)
                        .color(TEXT),
                );
                ui.add_space(10.0);
                ui.label(
                    RichText::new(self.t("about.unity"))
                        .size(12.5)
                        .strong()
                        .color(TEXT),
                );
                ui.add_space(4.0);
                for key in ["about.u1", "about.u2", "about.u3"] {
                    ui.label(RichText::new(self.t(key)).size(12.0).color(MUTED));
                }
                ui.add_space(10.0);
                ui.label(
                    RichText::new(self.t("about.other"))
                        .size(12.5)
                        .strong()
                        .color(TEXT),
                );
                ui.add_space(4.0);
                for key in ["about.o1", "about.o2", "about.o3"] {
                    ui.label(RichText::new(self.t(key)).size(12.0).color(MUTED));
                }
                ui.add_space(10.0);
                ui.label(
                    RichText::new(self.t("about.footer"))
                        .size(11.5)
                        .color(MUTED),
                );
                ui.add_space(14.0);
                if ui
                    .add(
                        egui::Button::new(
                            RichText::new(self.t("common.close"))
                                .size(13.0)
                                .color(TEXT),
                        )
                        .fill(PANEL_2)
                        .stroke(Stroke::new(1.0, BORDER))
                        .corner_radius(CornerRadius::same(8))
                        .min_size(Vec2::new(72.0, 30.0)),
                    )
                    .clicked()
                {
                    self.model.show_about = false;
                }
            });
        if !open {
            self.model.show_about = false;
        }
    }

    fn composer_can_send(&self) -> bool {
        let draft = self.model.draft.trim();
        let has_content = !draft.is_empty() || !self.attachments.is_empty();
        let unity_local = !draft.is_empty()
            && (parse_unity_chat_command(draft).is_some() || wants_unity_help(draft));
        (self.model.connected
            && !self.model.busy
            && !self.model.needs_login
            && has_content)
            || unity_local
    }

    fn composer_send_hint(&self, can_send: bool) -> &str {
        if can_send {
            return self.t("composer.send_hint");
        }
        if self.model.needs_login {
            return self.t("composer.need_login");
        }
        if self.model.busy {
            return self.t("composer.busy");
        }
        if !self.model.connected {
            return self.t("composer.connecting");
        }
        if self.model.draft.trim().is_empty() && self.attachments.is_empty() {
            return self.t("composer.empty");
        }
        self.t("composer.cant_send")
    }

    fn floating_composer(&mut self, ui: &mut egui::Ui) {
        Frame::new()
            .fill(PANEL)
            .corner_radius(CornerRadius::same(16))
            .stroke(Stroke::new(1.0, BORDER))
            .shadow(Shadow {
                offset: [0, 4],
                blur: 16,
                spread: 0,
                color: Color32::from_black_alpha(80),
            })
            .inner_margin(Margin::symmetric(14, 12))
            .show(ui, |ui| {
                ui.set_width(ui.available_width());

                // Context chips: current project (when chosen) + plugins + files.
                let show_project = !self.awaiting_project_choice;
                let unity_on = self.unity_chat_mode && self.plugin_prefs.unity_enabled;
                let has_context = show_project || unity_on || !self.attachments.is_empty();
                if has_context {
                    self.composer_context_chips(ui);
                    ui.add_space(8.0);
                }

                let hint = if self.model.needs_login {
                    self.t("composer.hint_login")
                } else if unity_on {
                    self.t("composer.hint_unity")
                } else if !self.model.connected {
                    self.t("composer.hint_connecting")
                } else if self.model.is_viewing_history() {
                    self.t("composer.hint_history")
                } else {
                    self.t("composer.hint")
                }
                .to_owned();

                let edit = egui::TextEdit::multiline(&mut self.model.draft)
                    .desired_width(f32::INFINITY)
                    .desired_rows(2)
                    .frame(false)
                    .interactive(!self.model.needs_login)
                    .hint_text(RichText::new(hint).size(14.0).color(MUTED));
                let response = ui.add(edit);
                if self.model.focus_composer {
                    response.request_focus();
                    self.model.focus_composer = false;
                }

                let can_send = self.composer_can_send();
                let enter_send = response.has_focus()
                    && can_send
                    && ui.input(|i| {
                        i.key_pressed(egui::Key::Enter)
                            && !i.modifiers.shift
                            && !i.modifiers.ctrl
                            && !i.modifiers.command
                    });
                if enter_send {
                    self.model.draft = self
                        .model
                        .draft
                        .trim_end_matches('\n')
                        .trim_end_matches('\r')
                        .to_string();
                    self.send_prompt();
                }

                if unity_on {
                    ui.add_space(8.0);
                    self.unity_composer_shortcuts(ui);
                }

                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    let plus_tip = if self.show_composer_plus {
                        self.t("composer.plus_open")
                    } else {
                        self.t("composer.plus_closed")
                    };
                    let plus = composer_plus_btn(ui, self.show_composer_plus, plus_tip);
                    if plus.clicked() {
                        self.show_composer_plus = !self.show_composer_plus;
                        self.composer_plus_just_opened = self.show_composer_plus;
                    }
                    if self.show_composer_plus {
                        self.composer_plus_anchor = Some(plus.rect);
                    }

                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        let send_hint = self.composer_send_hint(can_send);
                        let send = ui
                            .add(
                                egui::Button::new(
                                    RichText::new(self.t("composer.send"))
                                        .size(12.5)
                                        .color(if can_send { BG } else { MUTED })
                                        .strong(),
                                )
                                .fill(if can_send {
                                    ACCENT
                                } else {
                                    Color32::from_rgb(48, 48, 56)
                                })
                                .stroke(Stroke::new(
                                    1.0,
                                    if can_send { ACCENT } else { BORDER },
                                ))
                                .corner_radius(CornerRadius::same(10))
                                .min_size(Vec2::new(72.0, 32.0)),
                            )
                            .on_hover_text(send_hint);
                        if can_send && send.clicked() {
                            self.send_prompt();
                        }

                        if (self.model.busy || self.stop_armed_force)
                            && ui
                                .add(
                                    egui::Button::new(
                                        RichText::new(if self.stop_armed_force {
                                            self.t("composer.force_stop")
                                        } else {
                                            self.t("composer.stop")
                                        })
                                        .size(12.0)
                                        .color(if self.stop_armed_force { DANGER } else { TEXT }),
                                    )
                                    .fill(PANEL_2)
                                    .stroke(Stroke::new(
                                        1.0,
                                        if self.stop_armed_force {
                                            DANGER
                                        } else {
                                            BORDER
                                        },
                                    ))
                                    .corner_radius(CornerRadius::same(10))
                                    .min_size(Vec2::new(64.0, 28.0)),
                                )
                                .on_hover_text(if self.stop_armed_force {
                                    "结束卡住的 agent 子进程并重连"
                                } else {
                                    "请求停止当前回合；若无效再点一次强制停止"
                                })
                                .clicked()
                        {
                            if self.stop_armed_force {
                                self.send_cmd(UiCommand::ForceStop);
                                self.model.busy = false;
                                self.model.status = "强制停止，正在重连…".into();
                                self.stop_armed_force = false;
                            } else {
                                self.send_cmd(UiCommand::Cancel);
                                // Unlock immediately — cancel used to sit behind a
                                // blocked prompt await and never reach the bridge.
                                self.model.busy = false;
                                self.model.status = "已请求停止（仍卡住再点「强制停止」）".into();
                                self.stop_armed_force = true;
                            }
                        }

                        let u = &self.model.usage.cumulative;
                        let usage_label = format!("Σ {}", format_tokens(u.total_tokens));
                        if soft_chip(ui, &usage_label, true) {
                            self.model.show_usage_detail = true;
                            self.model.show_user_menu = false;
                            self.user_menu_anchor = None;
                        }
                        ui.add_space(4.0);

                        let mode_label = self
                            .model
                            .available_modes
                            .iter()
                            .find(|m| m.id == self.model.current_mode_id)
                            .map(|m| m.name.as_str())
                            .unwrap_or("执行模式");
                        if !self.model.available_modes.is_empty()
                            && soft_chip(ui, mode_label, self.model.connected && !self.model.busy)
                        {
                            let next = self
                                .model
                                .available_modes
                                .iter()
                                .find(|m| m.id != self.model.current_mode_id)
                                .map(|m| m.id.clone());
                            if let Some(mode_id) = next {
                                self.send_cmd(UiCommand::SetMode { mode_id });
                            }
                        }
                        ui.add_space(4.0);

                        let model_label = if self.model.current_model_name.is_empty() {
                            "选择模型"
                        } else {
                            self.model.current_model_name.as_str()
                        };
                        if soft_chip(
                            ui,
                            model_label,
                            self.model.connected && !self.model.needs_login,
                        ) {
                            self.model.show_model_picker = true;
                        }
                    });
                });
            });
    }

    /// Dismissible pills for active plugins / attachments (Codex-like).
    fn composer_context_chips(&mut self, ui: &mut egui::Ui) {
        let unity_on = self.unity_chat_mode && self.plugin_prefs.unity_enabled;
        let mut drop_unity = false;
        let mut clear_files = false;
        let mut drop_file: Option<usize> = None;
        let mut pick_project = false;

        ui.horizontal_wrapped(|ui| {
            if !self.awaiting_project_choice {
                let cwd = self
                    .model
                    .cwd
                    .clone()
                    .unwrap_or_else(|| self.config.cwd.clone());
                let root = canonical_project_root(&cwd);
                let name = AppModel::project_label(&root);
                let full_path = root.display().to_string();
                let resp = Frame::new()
                    .fill(PANEL_2)
                    .corner_radius(CornerRadius::same(10))
                    .stroke(Stroke::new(1.0, BORDER))
                    .inner_margin(Margin::symmetric(8, 4))
                    .show(ui, |ui| {
                        ui.horizontal(|ui| {
                            paint_sidebar_glyph(ui, SidebarGlyph::Folder, MUTED);
                            ui.add_space(5.0);
                            ui.label(RichText::new(name).size(12.0).color(TEXT));
                        });
                    })
                    .response
                    .interact(egui::Sense::click())
                    .on_hover_text(format!("{full_path}\n点击切换项目"));
                if resp.hovered() {
                    ui.ctx().set_cursor_icon(CursorIcon::PointingHand);
                }
                if resp.clicked() {
                    pick_project = true;
                }
            }

            if unity_on {
                let pill = Frame::new()
                    .fill(PANEL_2)
                    .corner_radius(CornerRadius::same(10))
                    .stroke(Stroke::new(1.0, BORDER))
                    .inner_margin(Margin::symmetric(8, 4))
                    .show(ui, |ui| {
                        ui.horizontal(|ui| {
                            paint_sidebar_glyph(ui, SidebarGlyph::Unity, MUTED);
                            ui.add_space(5.0);
                            ui.label(RichText::new("Unity").size(12.0).color(TEXT));
                            ui.add_space(4.0);
                            if icon_btn(ui, SidebarGlyph::Close, "移除此对话中的 Unity", false)
                                .clicked()
                            {
                                drop_unity = true;
                            }
                        });
                    })
                    .response
                    .on_hover_text("此对话使用 Unity CLI；点 × 取消");
                let _ = pill;
            }

            let names: Vec<String> = self.attachments.iter().map(|a| a.name.clone()).collect();
            for (idx, name) in names.iter().enumerate() {
                Frame::new()
                    .fill(PANEL_2)
                    .corner_radius(CornerRadius::same(10))
                    .stroke(Stroke::new(1.0, BORDER))
                    .inner_margin(Margin::symmetric(8, 4))
                    .show(ui, |ui| {
                        ui.horizontal(|ui| {
                            paint_sidebar_glyph(ui, SidebarGlyph::Doc, MUTED);
                            ui.add_space(5.0);
                            ui.label(
                                RichText::new(truncate_chip_label(name, 18))
                                    .size(12.0)
                                    .color(TEXT),
                            );
                            ui.add_space(4.0);
                            if icon_btn(ui, SidebarGlyph::Close, "移除附件", false).clicked() {
                                drop_file = Some(idx);
                            }
                        });
                    });
            }

            if names.len() > 1
                && ui
                    .add(
                        egui::Button::new(RichText::new("清除文件").size(11.0).color(MUTED))
                            .fill(Color32::TRANSPARENT)
                            .frame(false),
                    )
                    .clicked()
            {
                clear_files = true;
            }
        });

        if pick_project {
            self.pick_project(ui.ctx());
        }
        if drop_unity {
            self.set_chat_interaction(ChatInteraction::Agent);
        }
        if clear_files {
            self.attachments.clear();
        } else if let Some(i) = drop_file
            && i < self.attachments.len()
        {
            self.attachments.remove(i);
        }
    }

    fn unity_composer_shortcuts(&mut self, ui: &mut egui::Ui) {
        let busy = self.unity.busy || self.unity.is_guiding() || self.model.busy;
        ui.horizontal_wrapped(|ui| {
            for cmd in UNITY_CHAT_CHIPS.iter().take(5) {
                let clicked = ui
                    .add_enabled(
                        !busy,
                        egui::Button::new(RichText::new(cmd.chip).size(11.0).color(MUTED))
                            .fill(Color32::TRANSPARENT)
                            .stroke(Stroke::new(1.0, BORDER))
                            .corner_radius(CornerRadius::same(8))
                            .min_size(Vec2::new(0.0, 22.0)),
                    )
                    .on_hover_text(cmd.slash)
                    .clicked();
                if clicked {
                    self.dispatch_unity_chat_cmd(cmd, None);
                }
            }
            if ui
                .add(
                    egui::Button::new(RichText::new("说明").size(11.0).color(MUTED))
                        .fill(Color32::TRANSPARENT)
                        .frame(false),
                )
                .clicked()
            {
                self.show_unity_docs_window = true;
            }
        });
    }

    /// Floating 「+」 menu: files + installable plugins for this conversation.
    fn composer_plus_popup(&mut self, ctx: &egui::Context) {
        if !self.show_composer_plus {
            return;
        }
        let Some(anchor) = self.composer_plus_anchor else {
            self.show_composer_plus = false;
            return;
        };

        let mut close = false;
        let mut add_files = false;
        let mut toggle_unity = false;
        let mut open_plugins = false;
        let unity_installed = self.plugin_prefs.unity_enabled;
        let unity_active = self.unity_chat_mode && unity_installed;

        // Open upward from the + button so the menu does not bury the draft.
        let area = egui::Area::new(egui::Id::new("composer_plus_menu"))
            .order(egui::Order::Foreground)
            .pivot(Align2::LEFT_BOTTOM)
            .fixed_pos(egui::pos2(anchor.left(), anchor.top() - 8.0))
            .constrain(true)
            .show(ctx, |ui| {
                Frame::new()
                    .fill(Color32::from_rgb(24, 24, 28))
                    .stroke(Stroke::new(1.0, Color32::from_rgb(62, 62, 72)))
                    .corner_radius(CornerRadius::same(12))
                    .shadow(Shadow {
                        offset: [0, 10],
                        blur: 32,
                        spread: 0,
                        color: Color32::from_black_alpha(140),
                    })
                    .inner_margin(Margin::symmetric(6, 6))
                    .show(ui, |ui| {
                        ui.set_width(272.0);

                        if plus_menu_row(
                            ui,
                            SidebarGlyph::Doc,
                            self.t("plus.add_file"),
                            self.t("plus.add_file_sub"),
                            false,
                        ) {
                            add_files = true;
                            close = true;
                        }

                        ui.add_space(2.0);
                        plus_menu_divider(ui, self.t("plus.section_plugins"));
                        ui.add_space(2.0);

                        if unity_installed {
                            let sub = if unity_active {
                                self.t("plus.unity_on")
                            } else {
                                self.t("plus.unity_off")
                            };
                            if plus_menu_row(
                                ui,
                                SidebarGlyph::Unity,
                                self.t("plus.unity"),
                                sub,
                                unity_active,
                            ) {
                                toggle_unity = true;
                                close = true;
                            }
                        } else if plus_menu_row(
                            ui,
                            SidebarGlyph::Unity,
                            self.t("plus.unity"),
                            self.t("plus.unity_disabled"),
                            false,
                        ) {
                            open_plugins = true;
                            close = true;
                        }

                        ui.add_space(4.0);
                        ui.painter().hline(
                            ui.max_rect().x_range().shrink(6.0),
                            ui.cursor().top() + 0.5,
                            Stroke::new(1.0, Color32::from_rgb(48, 48, 56)),
                        );
                        ui.add_space(8.0);

                        if plus_menu_row(
                            ui,
                            SidebarGlyph::Plug,
                            self.t("plus.manage"),
                            self.t("plus.manage_sub"),
                            false,
                        ) {
                            open_plugins = true;
                            close = true;
                        }
                    });
            });

        if add_files {
            self.pick_attachments();
        }
        if toggle_unity {
            if unity_active {
                self.set_chat_interaction(ChatInteraction::Agent);
            } else {
                self.set_chat_interaction(ChatInteraction::Unity);
            }
        }
        if open_plugins {
            self.model.main_nav = MainNav::Plugins;
        }

        if self.composer_plus_just_opened {
            self.composer_plus_just_opened = false;
        } else {
            let pointer = ctx.pointer_interact_pos().unwrap_or(egui::pos2(-999.0, -999.0));
            let clicked_elsewhere = ctx.input(|i| i.pointer.any_click())
                && !area.response.contains_pointer()
                && !anchor.contains(pointer);
            if close || clicked_elsewhere || ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
                self.show_composer_plus = false;
                self.composer_plus_anchor = None;
            }
        }
        if close {
            self.show_composer_plus = false;
            self.composer_plus_anchor = None;
        }
    }

    fn user_menu_popup(&mut self, ctx: &egui::Context) {
        if !self.model.show_user_menu {
            return;
        }
        let Some(anchor) = self.user_menu_anchor else {
            self.model.show_user_menu = false;
            return;
        };

        let mut close = false;
        let mut open_usage = false;
        let mut open_config = false;
        let mut do_login = false;
        let mut next_lang: Option<Language> = None;
        let needs_login = self.model.needs_login;
        let display_name = self.model.display_name.clone();
        let initials = self.model.initials();
        let subtitle = if needs_login {
            self.t("user.signed_out")
        } else {
            self.t("user.local_account")
        };
        let usage_label = self.t("user.usage");
        let config_label = self.t("user.edit_config");
        let lang_label = self.t("user.language");
        let auth_label = if needs_login {
            self.t("user.login")
        } else {
            self.t("user.relogin")
        };
        let current_lang = self.lang();

        let area = egui::Area::new(egui::Id::new("user_account_menu"))
            .order(egui::Order::Foreground)
            .pivot(Align2::LEFT_BOTTOM)
            .fixed_pos(egui::pos2(anchor.left(), anchor.top() - 8.0))
            .constrain(true)
            .show(ctx, |ui| {
                Frame::new()
                    .fill(Color32::from_rgb(24, 24, 28))
                    .stroke(Stroke::new(1.0, Color32::from_rgb(62, 62, 72)))
                    .corner_radius(CornerRadius::same(12))
                    .shadow(Shadow {
                        offset: [0, 10],
                        blur: 28,
                        spread: 0,
                        color: Color32::from_black_alpha(140),
                    })
                    .inner_margin(Margin::symmetric(8, 8))
                    .show(ui, |ui| {
                        ui.set_width(248.0);

                        // Header
                        ui.horizontal(|ui| {
                            avatar_circle(ui, &initials);
                            ui.add_space(10.0);
                            ui.vertical(|ui| {
                                ui.label(
                                    RichText::new(&display_name)
                                        .size(13.5)
                                        .strong()
                                        .color(TEXT),
                                );
                                ui.label(RichText::new(subtitle).size(11.5).color(MUTED));
                            });
                        });

                        ui.add_space(8.0);
                        thin_menu_rule(ui);
                        ui.add_space(4.0);

                        if account_menu_row(ui, usage_label, AccountRowKind::Chevron) {
                            open_usage = true;
                            close = true;
                        }
                        if account_menu_row(ui, config_label, AccountRowKind::Plain) {
                            open_config = true;
                            close = true;
                        }

                        ui.add_space(4.0);
                        thin_menu_rule(ui);
                        ui.add_space(6.0);

                        // Language: quiet label + compact segmented control
                        ui.horizontal(|ui| {
                            ui.add_space(8.0);
                            ui.label(RichText::new(lang_label).size(12.5).color(TEXT));
                            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                if let Some(lang) = language_segment(ui, current_lang) {
                                    next_lang = Some(lang);
                                }
                            });
                        });

                        ui.add_space(6.0);
                        thin_menu_rule(ui);
                        ui.add_space(4.0);

                        if account_menu_row(ui, auth_label, AccountRowKind::Muted) {
                            do_login = true;
                            close = true;
                        }
                    });
            });

        if let Some(lang) = next_lang {
            self.set_language(lang);
        }
        if open_usage {
            self.model.show_usage_detail = true;
        }
        if open_config {
            if let Err(e) = crate::config_io::open_config_in_editor() {
                self.model.apply(AgentEvent::Error(format!(
                    "{}: {e}",
                    self.t("user.open_failed")
                )));
            }
        }
        if do_login {
            self.send_cmd(UiCommand::Login);
        }

        if self.user_menu_just_opened {
            self.user_menu_just_opened = false;
        } else {
            let pointer = ctx.pointer_interact_pos().unwrap_or(egui::pos2(-999.0, -999.0));
            let clicked_elsewhere = ctx.input(|i| i.pointer.any_click())
                && !area.response.contains_pointer()
                && !anchor.contains(pointer);
            if close || clicked_elsewhere || ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
                self.model.show_user_menu = false;
                self.user_menu_anchor = None;
            }
        }
        if close {
            self.model.show_user_menu = false;
            self.user_menu_anchor = None;
        }
    }

    fn task_error_modal(&mut self, ctx: &egui::Context) {
        let Some(message) = self.task_error.clone() else {
            return;
        };
        egui::Window::new("操作未完成")
            .collapsible(false)
            .resizable(true)
            .anchor(Align2::CENTER_CENTER, [0.0, 0.0])
            .show(ctx, |ui| {
                ui.set_min_width(420.0);
                ui.label(RichText::new(message).color(DANGER));
                ui.add_space(12.0);
                if ui.button("关闭").clicked() {
                    self.task_error = None;
                }
            });
    }

    fn git_confirmation_modal(&mut self, ctx: &egui::Context) {
        let Some((stage, path)) = self.pending_git_action.clone() else {
            return;
        };
        egui::Window::new(if stage {
            "确认暂存"
        } else {
            "确认取消暂存"
        })
        .collapsible(false)
        .resizable(false)
        .anchor(Align2::CENTER_CENTER, [0.0, 0.0])
        .show(ctx, |ui| {
            ui.label(format!("将对 {} 执行显式 Git 写操作。", path.display()));
            ui.add_space(10.0);
            ui.horizontal(|ui| {
                if ui.button("取消").clicked() {
                    self.pending_git_action = None;
                }
                if ui.button("确认").clicked() {
                    let result = if stage {
                        GitWorkspaceService::stage(&self.config.cwd, &path)
                    } else {
                        GitWorkspaceService::unstage(&self.config.cwd, &path)
                    };
                    match result {
                        Ok(()) => {
                            self.changes =
                                GitWorkspaceService::changes(&self.config.cwd).unwrap_or_default()
                        }
                        Err(e) => self.task_error = Some(e),
                    }
                    self.pending_git_action = None;
                }
            });
        });
    }

    /// Centered usage sheet: clean single card, tabs, no nested sidebar.
    fn usage_detail_window(&mut self, ctx: &egui::Context) {
        if !self.model.show_usage_detail {
            return;
        }

        let screen = ctx.screen_rect();
        let panel_w = (screen.width() * 0.58).clamp(560.0, 820.0);
        let panel_h = (screen.height() * 0.78).clamp(480.0, 720.0);

        let mut close = false;
        egui::Area::new(egui::Id::new("usage_dim"))
            .fixed_pos(screen.min)
            .order(egui::Order::Middle)
            .show(ctx, |ui| {
                let resp = ui.allocate_rect(screen, egui::Sense::click());
                ui.painter()
                    .rect_filled(screen, 0.0, Color32::from_black_alpha(170));
                if resp.clicked() {
                    close = true;
                }
            });

        let model_stats = aggregate_model_usage(&self.model.history_turns);
        let turns: Vec<_> = self.model.history_turns.iter().rev().cloned().collect();
        // Prefer history totals so chips match charts (session cumulative can stay 0
        // after "new task" clears local session turns).
        let hist_total: u64 = self
            .model
            .history_turns
            .iter()
            .map(|t| t.usage_delta.total_tokens)
            .sum();
        let hist_in: u64 = self
            .model
            .history_turns
            .iter()
            .map(|t| t.usage_delta.input_tokens)
            .sum();
        let hist_out: u64 = self
            .model
            .history_turns
            .iter()
            .map(|t| t.usage_delta.output_tokens)
            .sum();
        let sess = &self.model.usage.cumulative;
        let chip_total = hist_total.max(sess.total_tokens);
        let chip_in = hist_in.max(sess.input_tokens);
        let chip_out = hist_out.max(sess.output_tokens);
        let sess_turns = self
            .model
            .history_turns
            .len()
            .max(self.model.usage.turns.len());
        let ctx_used = sess.context_used;
        let ctx_size = sess.context_size;
        let mut open = true;
        let tab = self.model.usage_tab;

        egui::Window::new("使用统计")
            .id(egui::Id::new("usage_sheet"))
            .collapsible(false)
            .resizable(false)
            .title_bar(false)
            .anchor(Align2::CENTER_CENTER, [0.0, 0.0])
            .fixed_size([panel_w, panel_h])
            .order(egui::Order::Foreground)
            .frame(
                Frame::new()
                    .fill(PANEL)
                    .corner_radius(CornerRadius::same(16))
                    .stroke(Stroke::new(1.0, BORDER))
                    .inner_margin(Margin::symmetric(22, 18))
                    .shadow(Shadow {
                        offset: [0, 16],
                        blur: 48,
                        spread: 0,
                        color: Color32::from_black_alpha(160),
                    }),
            )
            .open(&mut open)
            .show(ctx, |ui| {
                ui.set_min_size(Vec2::new(panel_w - 8.0, panel_h - 8.0));

                // Header
                ui.horizontal(|ui| {
                    ui.vertical(|ui| {
                        ui.label(RichText::new("使用统计").size(18.0).strong().color(TEXT));
                        ui.label(
                            RichText::new("折线 / 柱状统计 · 模型与轮次明细")
                                .size(12.5)
                                .color(MUTED),
                        );
                    });
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui
                            .add(
                                egui::Button::new(RichText::new("关闭").size(12.5).color(TEXT))
                                    .fill(PANEL_2)
                                    .stroke(Stroke::new(1.0, BORDER))
                                    .corner_radius(CornerRadius::same(8))
                                    .min_size(Vec2::new(64.0, 30.0)),
                            )
                            .clicked()
                        {
                            close = true;
                        }
                    });
                });

                ui.add_space(14.0);

                // Summary chips row
                ui.horizontal(|ui| {
                    stat_chip(ui, "轮次", &sess_turns.to_string());
                    ui.add_space(8.0);
                    stat_chip(ui, "合计", &format_tokens(chip_total));
                    ui.add_space(8.0);
                    stat_chip(ui, "输入", &format_tokens(chip_in));
                    ui.add_space(8.0);
                    stat_chip(ui, "输出", &format_tokens(chip_out));
                    if let (Some(used), Some(size)) = (ctx_used, ctx_size) {
                        ui.add_space(8.0);
                        stat_chip(
                            ui,
                            "上下文",
                            &format!("{}/{}", format_tokens(used), format_tokens(size)),
                        );
                    }
                });

                ui.add_space(14.0);

                // Tabs
                ui.horizontal(|ui| {
                    if segment_tab(ui, "统计图", tab == UsageTab::Charts) {
                        self.model.usage_tab = UsageTab::Charts;
                    }
                    ui.add_space(6.0);
                    if segment_tab(ui, "模型", tab == UsageTab::Models) {
                        self.model.usage_tab = UsageTab::Models;
                    }
                    ui.add_space(6.0);
                    if segment_tab(
                        ui,
                        &format!("轮次 ({})", turns.len()),
                        tab == UsageTab::Turns,
                    ) {
                        self.model.usage_tab = UsageTab::Turns;
                    }
                });

                ui.add_space(10.0);

                let list_h = (panel_h - 200.0).max(180.0);
                egui::ScrollArea::vertical()
                    .id_salt("usage_sheet_scroll")
                    .max_height(list_h)
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        ui.set_width(ui.available_width());
                        match self.model.usage_tab {
                            UsageTab::Charts => {
                                // Chronological for charts (history_turns is oldest→newest).
                                charts::draw_usage_charts(
                                    ui,
                                    &self.model.history_turns,
                                    &model_stats,
                                );
                            }
                            UsageTab::Models => {
                                if model_stats.is_empty() {
                                    empty_hint(ui, "还没有模型用量。发送一条消息后会出现在这里。");
                                } else {
                                    for m in &model_stats {
                                        let pct = if chip_total > 0 {
                                            (m.total_tokens as f32 / chip_total as f32)
                                                .clamp(0.0, 1.0)
                                        } else if model_stats.len() == 1 {
                                            1.0
                                        } else {
                                            0.0
                                        };
                                        Frame::new()
                                            .fill(BG)
                                            .corner_radius(CornerRadius::same(12))
                                            .stroke(Stroke::new(1.0, BORDER))
                                            .inner_margin(Margin::symmetric(14, 12))
                                            .show(ui, |ui| {
                                                ui.set_width(ui.available_width());
                                                ui.horizontal(|ui| {
                                                    ui.vertical(|ui| {
                                                        ui.label(
                                                            RichText::new(&m.model_name)
                                                                .size(14.5)
                                                                .strong()
                                                                .color(TEXT),
                                                        );
                                                        if !m.model_id.is_empty()
                                                            && m.model_id != m.model_name
                                                        {
                                                            ui.label(
                                                                RichText::new(&m.model_id)
                                                                    .size(11.5)
                                                                    .monospace()
                                                                    .color(MUTED),
                                                            );
                                                        }
                                                    });
                                                    ui.with_layout(
                                                        egui::Layout::right_to_left(
                                                            egui::Align::Center,
                                                        ),
                                                        |ui| {
                                                            ui.label(
                                                                RichText::new(format!(
                                                                    "Σ {}",
                                                                    format_tokens(m.total_tokens)
                                                                ))
                                                                .size(14.0)
                                                                .strong()
                                                                .color(ACCENT_BAR),
                                                            );
                                                        },
                                                    );
                                                });
                                                ui.add_space(8.0);
                                                // Progress bar for share of session tokens
                                                let bar_w = ui.available_width();
                                                let (rect, _) = ui.allocate_exact_size(
                                                    Vec2::new(bar_w, 4.0),
                                                    egui::Sense::hover(),
                                                );
                                                ui.painter().rect_filled(
                                                    rect,
                                                    CornerRadius::same(2),
                                                    PANEL_2,
                                                );
                                                if pct > 0.0 {
                                                    let mut fill = rect;
                                                    fill.set_width((rect.width() * pct).max(4.0));
                                                    ui.painter().rect_filled(
                                                        fill,
                                                        CornerRadius::same(2),
                                                        ACCENT_BAR,
                                                    );
                                                }
                                                ui.add_space(8.0);
                                                ui.label(
                                                    RichText::new(format!(
                                                        "{} 轮  ·  in {}  ·  out {}",
                                                        m.turn_count,
                                                        format_tokens(m.input_tokens),
                                                        format_tokens(m.output_tokens),
                                                    ))
                                                    .size(12.5)
                                                    .color(MUTED),
                                                );
                                            });
                                        ui.add_space(10.0);
                                    }
                                }
                            }
                            UsageTab::Turns => {
                                if turns.is_empty() {
                                    empty_hint(ui, "还没有对话轮次记录。");
                                } else {
                                    for turn in &turns {
                                        let expanded = self.model.is_history_expanded(&turn.id);
                                        let model = if turn.model_name.is_empty() {
                                            turn.model_id.as_str()
                                        } else {
                                            turn.model_name.as_str()
                                        };
                                        let chevron = if expanded { "▾" } else { "▸" };
                                        let resp = Frame::new()
                                            .fill(BG)
                                            .corner_radius(CornerRadius::same(12))
                                            .stroke(Stroke::new(
                                                1.0,
                                                if expanded { ACCENT_BAR } else { BORDER },
                                            ))
                                            .inner_margin(Margin::symmetric(14, 12))
                                            .show(ui, |ui| {
                                                ui.set_width(ui.available_width());
                                                ui.horizontal(|ui| {
                                                    ui.label(
                                                        RichText::new(chevron)
                                                            .size(13.0)
                                                            .color(MUTED),
                                                    );
                                                    ui.label(
                                                        RichText::new(model)
                                                            .size(13.5)
                                                            .strong()
                                                            .color(TEXT),
                                                    );
                                                    ui.with_layout(
                                                        egui::Layout::right_to_left(
                                                            egui::Align::Center,
                                                        ),
                                                        |ui| {
                                                            ui.label(
                                                                RichText::new(format!(
                                                                    "Δ {}",
                                                                    format_tokens(
                                                                        turn.usage_delta
                                                                            .total_tokens
                                                                    )
                                                                ))
                                                                .size(13.0)
                                                                .color(ACCENT_BAR),
                                                            );
                                                        },
                                                    );
                                                });
                                                ui.add_space(4.0);
                                                ui.label(
                                                    RichText::new(truncate_chars(
                                                        &turn.user_text,
                                                        90,
                                                    ))
                                                    .size(13.0)
                                                    .color(MUTED),
                                                );
                                                if expanded {
                                                    ui.add_space(10.0);
                                                    ui.label(
                                                        RichText::new(format!(
                                                            "in {} · out {} · 停止 {}",
                                                            format_tokens(
                                                                turn.usage_delta.input_tokens
                                                            ),
                                                            format_tokens(
                                                                turn.usage_delta.output_tokens
                                                            ),
                                                            turn.stop_reason,
                                                        ))
                                                        .size(12.0)
                                                        .color(MUTED),
                                                    );
                                                    if !turn.tool_titles.is_empty() {
                                                        ui.label(
                                                            RichText::new(format!(
                                                                "工具 · {}",
                                                                turn.tool_titles.join(" · ")
                                                            ))
                                                            .size(12.0)
                                                            .color(MUTED),
                                                        );
                                                    }
                                                    ui.add_space(6.0);
                                                    ui.label(
                                                        RichText::new("助手回复")
                                                            .size(11.5)
                                                            .strong()
                                                            .color(OK),
                                                    );
                                                    ui.label(
                                                        RichText::new(truncate_chars(
                                                            &turn.assistant_text,
                                                            500,
                                                        ))
                                                        .size(12.5)
                                                        .color(TEXT),
                                                    );
                                                } else {
                                                    ui.label(
                                                        RichText::new("点击展开详情")
                                                            .size(11.5)
                                                            .color(MUTED),
                                                    );
                                                }
                                            })
                                            .response
                                            .interact(egui::Sense::click());
                                        if resp.hovered() {
                                            ui.ctx()
                                                .set_cursor_icon(egui::CursorIcon::PointingHand);
                                        }
                                        if resp.clicked() {
                                            self.model.toggle_history_expanded(&turn.id);
                                        }
                                        ui.add_space(10.0);
                                    }
                                }
                            }
                        }
                    });
            });

        if !open || close || ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
            self.model.show_usage_detail = false;
        }
    }

    fn empty_state(&mut self, ui: &mut egui::Ui, _ctx: &egui::Context) {
        let width = ui.available_width();
        let height = ui.available_height().max(240.0);
        let (rect, _) = ui.allocate_exact_size(Vec2::new(width, height), egui::Sense::hover());

        let line = if self.model.needs_login {
            self.t("empty.login")
        } else {
            self.t("empty.prompt")
        }
        .to_owned();

        ui.allocate_new_ui(
            egui::UiBuilder::new()
                .max_rect(rect)
                .layout(egui::Layout::top_down(egui::Align::Center)),
            |ui| {
                ui.add_space((height * 0.38).clamp(64.0, 200.0));
                ui.label(RichText::new(line).size(22.0).strong().color(TEXT));
            },
        );
    }

    /// Sidebar「最近对话」分栏 — always under the project list.
    fn sidebar_recent_inbox(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        let recent_title = self.t("empty.recent").to_owned();
        let expand_label = self.t("empty.expand_more").to_owned();
        let collapse_label = self.t("empty.collapse").to_owned();
        let no_recent = self.t("empty.no_recent").to_owned();
        let delete_tip = self.t("empty.delete_chat").to_owned();
        let active_id = self.active_task_id.clone();

        let mut keyed: Vec<(i64, String, String, String)> = self
            .tasks
            .iter()
            // Only top-level「新建对话」entries — never per-project「新建」.
            .filter(|t| t.status != TaskStatus::Archived && t.from_new_chat)
            .map(|t| {
                (
                    t.updated_at,
                    t.id.clone(),
                    display_task_title(t),
                    AppModel::project_label(&canonical_project_root(&t.project_path)),
                )
            })
            .collect();
        keyed.sort_by(|a, b| b.0.cmp(&a.0));
        let recent: Vec<(String, String, String)> = keyed
            .into_iter()
            .map(|(_, id, title, project)| (id, title, project))
            .collect();
        let total = recent.len();
        let show_n = if self.recent_inbox_expanded {
            total
        } else {
            total.min(3)
        };

        let mut activate_id: Option<String> = None;
        let mut delete_id: Option<String> = None;
        let mut toggle_expand = false;

        ui.horizontal(|ui| {
            ui.label(RichText::new(&recent_title).size(11.5).color(MUTED));
            if total > 0 {
                ui.label(
                    RichText::new(format!("· {total}"))
                        .size(11.0)
                        .color(MUTED),
                );
            }
        });
        ui.add_space(6.0);

        if recent.is_empty() {
            ui.label(RichText::new(&no_recent).size(12.0).color(MUTED));
        } else {
            // Avoid nested ScrollArea for the default 3 rows (click/scroll glitches).
            let mut draw_rows = |ui: &mut egui::Ui| {
                for (id, title, project) in recent.iter().take(show_n) {
                    let selected = active_id.as_ref() == Some(id);
                    let (act, del) = render_recent_inbox_row(
                        ui,
                        id,
                        title,
                        project,
                        &delete_tip,
                        selected,
                    );
                    if act {
                        activate_id = Some(id.clone());
                    }
                    if del {
                        delete_id = Some(id.clone());
                    }
                    ui.add_space(2.0);
                }
            };
            if self.recent_inbox_expanded && total > 3 {
                let list_h = (show_n as f32 * 48.0 + 4.0).min(280.0);
                egui::ScrollArea::vertical()
                    .id_salt("sidebar_recent_inbox")
                    .max_height(list_h)
                    .auto_shrink([false, true])
                    .show(ui, |ui| {
                        ui.set_width(ui.available_width());
                        draw_rows(ui);
                    });
            } else {
                draw_rows(ui);
            }
        }

        if total > 3 {
            ui.add_space(4.0);
            let label = if self.recent_inbox_expanded {
                collapse_label
            } else {
                format!("{expand_label} ({})", total - 3)
            };
            if ui
                .add(
                    egui::Button::new(RichText::new(label).size(12.0).color(MUTED))
                        .fill(Color32::TRANSPARENT)
                        .stroke(Stroke::NONE)
                        .frame(false),
                )
                .on_hover_cursor(CursorIcon::PointingHand)
                .clicked()
            {
                toggle_expand = true;
            }
        }

        if toggle_expand {
            self.recent_inbox_expanded = !self.recent_inbox_expanded;
        }
        if let Some(id) = delete_id {
            self.delete_task = Some(id);
            self.delete_modal_ignore_click = true;
            ctx.request_repaint();
        }
        // Resolve by id from the live task list so we never activate a stale clone
        // that might point at the wrong project.
        if let Some(id) = activate_id
            && let Some(task) = self.tasks.iter().find(|t| t.id == id).cloned()
        {
            self.activate_task(ctx, task);
        }
    }

    fn render_task_row(
        &mut self,
        ui: &mut egui::Ui,
        ctx: &egui::Context,
        task: &TaskState,
        active_id: &Option<String>,
        archived: bool,
    ) {
        let selected = active_id.as_ref() == Some(&task.id);
        let title = display_task_title(task);
        let meta = task_row_meta(task);
        let mut archive = false;
        let mut unarchive = false;
        let mut rename = false;
        let mut request_delete = false;
        let mut activate = false;

        // Allocate first → paint from same-frame hover (no deferred temp / one-frame lag).
        let width = ui.available_width();
        let row_h = 44.0;
        let (rect, resp) = ui.allocate_exact_size(Vec2::new(width, row_h), egui::Sense::click());
        let hovered = resp.hovered() || resp.contains_pointer();
        let show_chrome = selected || hovered;

        // Reserved action hit-target (always same size → no layout jump).
        let close_rect = egui::Rect::from_center_size(
            egui::pos2(rect.right() - 16.0, rect.center().y),
            Vec2::splat(26.0),
        );
        let close_resp = ui.interact(
            close_rect,
            ui.id().with(("task_row_close", task.id.as_str())),
            egui::Sense::click(),
        );

        // Background
        if show_chrome {
            ui.painter().rect_filled(
                rect,
                CornerRadius::same(8),
                if selected {
                    SELECTED
                } else {
                    Color32::from_rgb(44, 46, 56)
                },
            );
            ui.painter().rect_stroke(
                rect,
                CornerRadius::same(8),
                Stroke::new(
                    1.0,
                    if selected {
                        Color32::from_rgb(70, 72, 84)
                    } else {
                        Color32::from_rgb(62, 64, 76)
                    },
                ),
                egui::StrokeKind::Inside,
            );
        }

        // Leading accent: solid when selected, soft when hoverable.
        let bar = egui::Rect::from_min_size(
            egui::pos2(rect.left() + 4.0, rect.top() + 10.0),
            Vec2::new(3.0, rect.height() - 20.0),
        );
        if selected {
            ui.painter()
                .rect_filled(bar, CornerRadius::same(2), ACCENT_BAR);
        } else if hovered {
            ui.painter().rect_filled(
                bar,
                CornerRadius::same(2),
                Color32::from_rgb(90, 100, 130),
            );
        }

        if hovered && !close_resp.hovered() {
            ui.ctx().set_cursor_icon(CursorIcon::PointingHand);
        }

        let fg = if show_chrome { TEXT } else { MUTED };
        let icon = if archived {
            SidebarGlyph::Archive
        } else {
            SidebarGlyph::Chat
        };
        paint_sidebar_glyph_at(
            ui.painter(),
            egui::pos2(rect.left() + 22.0, rect.center().y),
            icon,
            fg,
        );

        // Title + meta (fixed two-line slot so height never jumps).
        let text_left = rect.left() + 36.0;
        let text_right = close_rect.left() - 4.0;
        let text_w = (text_right - text_left).max(40.0);
        let title_pos = egui::pos2(text_left, rect.top() + 8.0);
        ui.painter().text(
            title_pos,
            egui::Align2::LEFT_TOP,
            truncate_chip_label(&title, ((text_w / 7.0) as usize).max(8)),
            egui::FontId::proportional(12.5),
            fg,
        );
        let meta_line = if meta.is_empty() { "\u{00A0}" } else { meta.as_str() };
        ui.painter().text(
            egui::pos2(text_left, rect.top() + 24.0),
            egui::Align2::LEFT_TOP,
            if meta.is_empty() {
                meta_line.to_string()
            } else {
                truncate_chip_label(meta_line, ((text_w / 6.5) as usize).max(8))
            },
            egui::FontId::proportional(10.5),
            if meta.is_empty() {
                Color32::TRANSPARENT
            } else {
                MUTED
            },
        );

        // Delete affordance only when hovered/selected (space already reserved).
        if show_chrome {
            if close_resp.hovered() {
                ui.painter().rect_filled(
                    close_rect,
                    CornerRadius::same(6),
                    Color32::from_rgb(72, 36, 36),
                );
                ui.ctx().set_cursor_icon(CursorIcon::PointingHand);
            }
            paint_sidebar_glyph_at(
                ui.painter(),
                close_rect.center(),
                SidebarGlyph::Close,
                if close_resp.hovered() { DANGER } else { MUTED },
            );
        }
        let _ = close_resp.clone().on_hover_text("删除对话");

        if close_resp.clicked() {
            request_delete = true;
        } else if resp.clicked() {
            activate = true;
        }

        resp.context_menu(|ui| {
            if ui.button("重命名").clicked() {
                rename = true;
                ui.close_menu();
            }
            if archived {
                if ui.button("取消归档").clicked() {
                    unarchive = true;
                    ui.close_menu();
                }
            } else if ui.button("归档到项目").clicked() {
                archive = true;
                ui.close_menu();
            }
            if ui.button("删除记录").clicked() {
                request_delete = true;
                ui.close_menu();
            }
        });

        if activate {
            self.activate_task(ctx, task.clone());
        }
        if archive {
            if let Some(found) = self.tasks.iter_mut().find(|t| t.id == task.id) {
                found.status = TaskStatus::Archived;
                found.updated_at = unix_time();
                found.project_path = canonical_project_root(&found.project_path);
                if let Some(repo) = &self.task_repo {
                    let _ = repo.save(found);
                }
            }
            self.expanded_archived
                .insert(project_group_key(&canonical_project_root(&task.project_path)));
            self.sidebar_groups_cache = None;
        }
        if unarchive {
            if let Some(found) = self.tasks.iter_mut().find(|t| t.id == task.id) {
                found.status = TaskStatus::Draft;
                found.updated_at = unix_time();
                if let Some(repo) = &self.task_repo {
                    let _ = repo.save(found);
                }
            }
            self.sidebar_groups_cache = None;
        }
        if rename {
            self.rename_task = Some((task.id.clone(), title));
        }
        if request_delete {
            self.delete_task = Some(task.id.clone());
            self.delete_modal_ignore_click = true;
            ctx.request_repaint();
        }
        ui.add_space(2.0);
    }

    fn sidebar_groups_fingerprint(&self, title_filter: &str) -> u64 {
        use std::collections::hash_map::DefaultHasher;
        use std::hash::{Hash, Hasher};
        let mut h = DefaultHasher::new();
        title_filter.hash(&mut h);
        self.task_list_filter.hash(&mut h);
        self.model.cwd.hash(&mut h);
        self.model.recent_projects.hash(&mut h);
        self.active_task_id.hash(&mut h);
        self.awaiting_project_choice.hash(&mut h);
        self.tasks.len().hash(&mut h);
        for t in &self.tasks {
            t.id.hash(&mut h);
            t.title.hash(&mut h);
            t.project_path.hash(&mut h);
            t.status.hash(&mut h);
            t.updated_at.hash(&mut h);
        }
        h.finish()
    }

    fn conversation_groups_cached(&mut self, title_filter: &str) -> &[ConversationGroup] {
        let fp = self.sidebar_groups_fingerprint(title_filter);
        let needs_rebuild = self
            .sidebar_groups_cache
            .as_ref()
            .is_none_or(|(cached_fp, _)| *cached_fp != fp);
        if needs_rebuild {
            let groups = self.conversation_groups(title_filter);
            self.sidebar_groups_cache = Some((fp, groups));
        }
        &self.sidebar_groups_cache.as_ref().unwrap().1
    }

    fn conversation_groups(&self, title_filter: &str) -> Vec<ConversationGroup> {
        let mut order: Vec<std::path::PathBuf> = Vec::new();
        let mut map: std::collections::HashMap<String, ConversationGroup> =
            std::collections::HashMap::new();

        for path in self
            .model
            .recent_projects
            .iter()
            .chain(self.tasks.iter().map(|t| &t.project_path))
        {
            let root = canonical_project_root(path);
            let key = project_group_key(&root);
            map.entry(key).or_insert_with(|| {
                order.push(root.clone());
                ConversationGroup {
                    project_path: root,
                    is_current: false,
                    tasks: Vec::new(),
                    archived: Vec::new(),
                }
            });
        }

        // One active context only:
        // - conversation selected → that conversation's project is "current"
        // - no conversation (switched project / empty) → cwd project
        // - awaiting_project_choice → nothing
        let current_key = if self.awaiting_project_choice {
            None
        } else if let Some(id) = &self.active_task_id {
            self.tasks
                .iter()
                .find(|t| &t.id == id)
                .map(|t| project_group_key(&canonical_project_root(&t.project_path)))
        } else {
            self.model
                .cwd
                .as_ref()
                .map(|p| project_group_key(&canonical_project_root(p)))
        };

        for (idx, task) in self.tasks.iter().enumerate() {
            if !title_filter.is_empty()
                && !task.title.to_lowercase().contains(title_filter)
                && !display_task_title(task).to_lowercase().contains(title_filter)
            {
                continue;
            }
            let key = project_group_key(&canonical_project_root(&task.project_path));
            let Some(group) = map.get_mut(&key) else {
                continue;
            };
            if task.status == TaskStatus::Archived {
                if matches!(
                    self.task_list_filter,
                    TaskListFilter::All | TaskListFilter::Archived
                ) {
                    group.archived.push(idx);
                }
            } else if self.task_list_filter.matches(task.status) {
                // Top-level「新建对话」lives only in「最近对话」, not under a project.
                if !task.from_new_chat {
                    group.tasks.push(idx);
                }
            }
        }

        let mut groups: Vec<ConversationGroup> = order
            .into_iter()
            .filter_map(|path| map.remove(&project_group_key(&path)))
            .collect();

        if !title_filter.is_empty() || self.task_list_filter == TaskListFilter::Archived {
            groups.retain(|g| !g.tasks.is_empty() || !g.archived.is_empty());
        }

        for group in &mut groups {
            group.is_current = current_key.as_ref() == Some(&project_group_key(&group.project_path));
            // Keep task order stable (enumeration order). Do not sort by updated_at —
            // selecting a conversation must not jump it to the top.
        }

        groups
    }

    fn timeline(&mut self, ui: &mut egui::Ui) {
        let len = self.model.timeline.len();
        for idx in 0..len {
            match self.model.timeline.get(idx).cloned() {
                Some(TimelineItem::Message(msg)) => self.message_block(ui, &msg),
                Some(TimelineItem::Tool(card)) => {
                    let mut open = card.open;
                    self.tool_block(ui, &card, &mut open);
                    if let Some(TimelineItem::Tool(c)) = self.model.timeline.get_mut(idx) {
                        c.open = open;
                    }
                }
                None => {}
            }
            ui.add_space(16.0);
        }
    }

    fn message_block(&self, ui: &mut egui::Ui, msg: &crate::model::ChatMessage) {
        match msg.role {
            Role::User => {
                let display_text = if msg.text.contains("只读分析")
                    && msg.text.contains("Unity 面板状态")
                {
                    "分析当前 Unity 状态"
                } else {
                    msg.text.as_str()
                };
                ui.with_layout(egui::Layout::right_to_left(egui::Align::TOP), |ui| {
                    Frame::new()
                        .fill(USER_BG)
                        .corner_radius(CornerRadius {
                            nw: 14,
                            ne: 14,
                            sw: 4,
                            se: 14,
                        })
                        .inner_margin(Margin::symmetric(14, 11))
                        .show(ui, |ui| {
                            ui.set_max_width(MAX_CHAT_W * 0.72);
                            ui.label(RichText::new(display_text).size(14.5).color(TEXT));
                        });
                });
            }
            Role::Assistant => {
                Frame::new()
                    .fill(ASSIST_BG)
                    .corner_radius(CornerRadius::same(12))
                    .stroke(Stroke::new(1.0, BORDER))
                    .inner_margin(Margin::symmetric(14, 12))
                    .show(ui, |ui| {
                        ui.set_width(ui.available_width());
                        ui.horizontal_top(|ui| {
                            let (bar, _) =
                                ui.allocate_exact_size(Vec2::new(3.0, 28.0), egui::Sense::hover());
                            ui.painter()
                                .rect_filled(bar, CornerRadius::same(2), ACCENT_BAR);
                            ui.add_space(10.0);
                            // Pin an explicit content width so wrapped inline code
                            // never collapses to a 1-glyph column.
                            let content_w = ui.available_width().max(40.0);
                            ui.allocate_ui_with_layout(
                                Vec2::new(content_w, 0.0),
                                egui::Layout::top_down(egui::Align::LEFT),
                                |ui| {
                                    ui.set_width(content_w);
                                    markdown::render(ui, &msg.text, TEXT);
                                },
                            );
                        });
                    });
                if let Some(u) = &msg.turn_usage {
                    ui.add_space(4.0);
                    ui.label(
                        RichText::new(format!(
                            "本轮 · Σ {} · in {} · out {}{}",
                            format_tokens(u.total_tokens),
                            format_tokens(u.input_tokens),
                            format_tokens(u.output_tokens),
                            if u.thought_tokens > 0 {
                                format!(" · think {}", format_tokens(u.thought_tokens))
                            } else {
                                String::new()
                            }
                        ))
                        .size(11.5)
                        .color(MUTED),
                    );
                }
            }
            Role::System => {
                Frame::new()
                    .fill(PANEL)
                    .corner_radius(CornerRadius::same(10))
                    .stroke(Stroke::new(1.0, Color32::from_rgb(90, 50, 50)))
                    .inner_margin(Margin::symmetric(12, 10))
                    .show(ui, |ui| {
                        ui.label(RichText::new("系统").size(11.5).color(DANGER));
                        ui.add_space(4.0);
                        ui.label(RichText::new(&msg.text).size(13.0).color(MUTED));
                    });
            }
        }
    }

    fn tool_block(&self, ui: &mut egui::Ui, card: &crate::model::ToolCard, open: &mut bool) {
        let status_color = if card.status.contains("Completed") || card.status.contains("completed")
        {
            OK
        } else if card.status.contains("Failed") || card.status.contains("Error") {
            DANGER
        } else {
            MUTED
        };

        Frame::new()
            .fill(TOOL_BG)
            .corner_radius(CornerRadius::same(10))
            .stroke(Stroke::new(1.0, BORDER))
            .inner_margin(Margin::symmetric(10, 8))
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    let chevron = if *open { "▾" } else { "▸" };
                    if ui
                        .add(
                            egui::Button::new(
                                RichText::new(format!("{chevron}  工具 · {}", card.title))
                                    .size(12.5)
                                    .color(TEXT),
                            )
                            .fill(Color32::TRANSPARENT)
                            .stroke(Stroke::NONE),
                        )
                        .clicked()
                    {
                        *open = !*open;
                    }
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.label(
                            RichText::new(short_status(&card.status))
                                .size(11.5)
                                .color(status_color),
                        );
                    });
                });
                if *open && !card.detail.is_empty() {
                    ui.add_space(6.0);
                    ui.separator();
                    ui.add_space(6.0);
                    ui.add(
                        egui::Label::new(
                            RichText::new(&card.detail)
                                .size(12.0)
                                .monospace()
                                .color(MUTED),
                        )
                        .wrap(),
                    );
                }
            });
    }

    fn model_picker_modal(&mut self, ctx: &egui::Context) {
        if !self.model.show_model_picker {
            return;
        }

        let mut open = true;
        egui::Window::new("选择模型")
            .collapsible(false)
            .resizable(false)
            .anchor(Align2::CENTER_CENTER, [0.0, 0.0])
            .open(&mut open)
            .frame(
                Frame::new()
                    .fill(PANEL)
                    .corner_radius(CornerRadius::same(16))
                    .stroke(Stroke::new(1.0, BORDER))
                    .inner_margin(Margin::same(16))
                    .shadow(Shadow {
                        offset: [0, 8],
                        blur: 28,
                        spread: 0,
                        color: Color32::from_black_alpha(120),
                    }),
            )
            .show(ctx, |ui| {
                ui.set_min_width(420.0);
                ui.set_max_height(480.0);
                ui.label(
                    RichText::new("切换当前会话使用的模型")
                        .size(13.0)
                        .color(MUTED),
                );
                ui.add_space(10.0);

                let models = self.model.available_models.clone();
                let current = self.model.current_model_id.clone();
                let busy = self.model.busy;

                if models.is_empty() {
                    ui.label(
                        RichText::new("暂无可用模型。可在 config.toml 的 [models] 里配置。")
                            .size(13.0)
                            .color(MUTED),
                    );
                } else {
                    egui::ScrollArea::vertical()
                        .max_height(320.0)
                        .show(ui, |ui| {
                            for m in &models {
                                let selected = m.id == current;
                                let fill = if selected { SELECTED } else { PANEL_2 };
                                let stroke = if selected {
                                    Stroke::new(1.0, ACCENT_BAR)
                                } else {
                                    Stroke::new(1.0, BORDER)
                                };
                                let resp = Frame::new()
                                    .fill(fill)
                                    .corner_radius(CornerRadius::same(10))
                                    .stroke(stroke)
                                    .inner_margin(Margin::symmetric(12, 10))
                                    .show(ui, |ui| {
                                        ui.set_width(ui.available_width());
                                        ui.horizontal(|ui| {
                                            ui.label(
                                                RichText::new(&m.name)
                                                    .size(14.0)
                                                    .strong()
                                                    .color(TEXT),
                                            );
                                            if selected {
                                                ui.label(
                                                    RichText::new("当前").size(11.5).color(OK),
                                                );
                                            }
                                        });
                                        ui.label(
                                            RichText::new(&m.id)
                                                .size(11.5)
                                                .monospace()
                                                .color(MUTED),
                                        );
                                        if !m.description.is_empty() {
                                            ui.label(
                                                RichText::new(&m.description)
                                                    .size(12.0)
                                                    .color(MUTED),
                                            );
                                        }
                                    })
                                    .response
                                    .interact(egui::Sense::click());

                                if !busy && !selected && resp.clicked() {
                                    self.send_cmd(UiCommand::SetModel {
                                        model_id: m.id.clone(),
                                    });
                                    self.model.status = format!("切换模型 {}…", m.name);
                                }
                                if resp.hovered() && !busy && !selected {
                                    ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
                                }
                                ui.add_space(8.0);
                            }
                        });
                }

                ui.add_space(8.0);
                ui.separator();
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    if ui
                        .add(
                            egui::Button::new(RichText::new("编辑 config.toml").color(TEXT))
                                .fill(PANEL_2)
                                .stroke(Stroke::new(1.0, BORDER))
                                .corner_radius(CornerRadius::same(10))
                                .min_size(Vec2::new(140.0, 32.0)),
                        )
                        .clicked()
                    {
                        if let Err(e) = crate::config_io::open_config_in_editor() {
                            self.model
                                .apply(AgentEvent::Error(format!("无法打开配置: {e}")));
                        }
                    }
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui
                            .add(
                                egui::Button::new(RichText::new("关闭").color(TEXT))
                                    .fill(PANEL_2)
                                    .stroke(Stroke::new(1.0, BORDER))
                                    .corner_radius(CornerRadius::same(10))
                                    .min_size(Vec2::new(72.0, 32.0)),
                            )
                            .clicked()
                        {
                            self.model.show_model_picker = false;
                        }
                    });
                });
            });

        if !open {
            self.model.show_model_picker = false;
        }
    }

    fn unity_permission_modal(&mut self, ctx: &egui::Context) {
        let Some(approval) = self.pending_unity_approval.clone() else {
            return;
        };
        egui::Window::new("Unity 操作需要权限")
            .collapsible(false)
            .resizable(true)
            .anchor(Align2::CENTER_CENTER, [0.0, 0.0])
            .show(ctx, |ui| {
                ui.set_min_width(520.0);
                ui.label(RichText::new(&approval.summary).size(16.0).strong());
                ui.add_space(8.0);
                if approval.risks.is_empty() {
                    ui.label("该计划会修改 Unity 编辑器、场景或项目资源。");
                } else {
                    ui.colored_label(
                        DANGER,
                        format!("计划请求高风险能力：{}", approval.risks.join("、")),
                    );
                    ui.label("高风险能力即使在完全控制模式下也会逐次询问。");
                }
                ui.add_space(10.0);
                egui::CollapsingHeader::new("查看生成的 Unity C#")
                    .show(ui, |ui| ui.monospace(&approval.csharp));
                ui.add_space(14.0);
                ui.horizontal_wrapped(|ui| {
                    if ui.button("允许一次").clicked() {
                        self.pending_unity_approval = None;
                        self.execute_unity_plan(approval.summary.clone(), approval.csharp.clone());
                    }
                    if approval.risks.is_empty() && ui.button("当前任务完全控制").clicked()
                    {
                        if let Some(id) = self.active_task_id.clone()
                            && let Some(task) = self.tasks.iter_mut().find(|task| task.id == id)
                        {
                            task.permission_mode = PermissionMode::FullControl;
                            task.updated_at = unix_time();
                            if let Some(repo) = &self.task_repo {
                                let _ = repo.save(task);
                            }
                        }
                        self.pending_unity_approval = None;
                        self.execute_unity_plan(approval.summary.clone(), approval.csharp.clone());
                    }
                    if ui.button("拒绝").clicked() {
                        self.pending_unity_approval = None;
                        self.model.replace_latest_assistant(format!(
                            "已拒绝 Unity 计划：{}。没有执行任何操作。",
                            approval.summary
                        ));
                        self.model.status = "Unity 操作已拒绝".into();
                    }
                });
            });
    }

    fn permission_modal(&mut self, ctx: &egui::Context) {
        let Some(perm) = self.model.pending_permission.clone() else {
            return;
        };

        egui::Area::new(egui::Id::new("perm_dim"))
            .fixed_pos(egui::pos2(0.0, 0.0))
            .order(egui::Order::Middle)
            .show(ctx, |ui| {
                let screen = ctx.screen_rect();
                ui.painter()
                    .rect_filled(screen, 0.0, Color32::from_black_alpha(160));
            });

        egui::Window::new("需要批准")
            .collapsible(false)
            .resizable(false)
            .anchor(Align2::CENTER_CENTER, [0.0, 0.0])
            .frame(
                Frame::new()
                    .fill(PANEL)
                    .corner_radius(CornerRadius::same(16))
                    .stroke(Stroke::new(1.0, BORDER))
                    .inner_margin(Margin::same(18))
                    .shadow(Shadow {
                        offset: [0, 8],
                        blur: 28,
                        spread: 0,
                        color: Color32::from_black_alpha(120),
                    }),
            )
            .show(ctx, |ui| {
                ui.set_min_width(420.0);
                ui.label(RichText::new(&perm.title).size(16.0).strong().color(TEXT));
                ui.add_space(6.0);
                ui.label(
                    RichText::new("助手想执行需要你确认的工具。")
                        .size(13.0)
                        .color(MUTED),
                );
                ui.add_space(12.0);
                for opt in &perm.options {
                    ui.label(
                        RichText::new(format!("· {}", opt.name))
                            .size(12.5)
                            .color(MUTED),
                    );
                }
                ui.add_space(16.0);
                ui.horizontal_wrapped(|ui| {
                    for opt in &perm.options {
                        let allow = opt.kind.contains("Allow");
                        if ui
                            .add(
                                egui::Button::new(RichText::new(&opt.name).color(if allow {
                                    BG
                                } else {
                                    TEXT
                                }))
                                .fill(if allow { ACCENT } else { PANEL_2 })
                                .stroke(Stroke::new(1.0, BORDER))
                                .corner_radius(CornerRadius::same(10))
                                .min_size(Vec2::new(100.0, 34.0)),
                            )
                            .clicked()
                        {
                            self.model.pending_permission = None;
                            self.send_cmd(UiCommand::PermissionResponse {
                                option_id: Some(opt.id.clone()),
                            });
                            self.model.status = if allow { "Working…" } else { "Ready" }.into();
                        }
                    }
                    if ui.button("取消").clicked() {
                        self.model.pending_permission = None;
                        self.send_cmd(UiCommand::PermissionResponse { option_id: None });
                    }
                });
            });
    }
}

/// Horizontally center a fixed-width chat column in the main pane.
fn centered_column(ui: &mut egui::Ui, add: impl FnOnce(&mut egui::Ui)) {
    let avail = ui.available_width();
    let width = if avail > MAX_CHAT_W + 48.0 {
        MAX_CHAT_W
    } else {
        (avail - 32.0).clamp(240.0, MAX_CHAT_W)
    };
    let pad = ((avail - width) * 0.5).max(0.0);

    ui.horizontal(|ui| {
        ui.add_space(pad);
        ui.allocate_ui_with_layout(
            Vec2::new(width, ui.available_height()),
            egui::Layout::top_down(egui::Align::Min).with_cross_justify(true),
            |ui| {
                ui.set_width(width);
                ui.set_max_width(width);
                add(ui);
            },
        );
    });
}

fn plugin_divider(ui: &mut egui::Ui) {
    let w = ui.available_width();
    let (rect, _) = ui.allocate_exact_size(Vec2::new(w, 1.0), egui::Sense::hover());
    ui.painter()
        .hline(rect.x_range(), rect.center().y, Stroke::new(1.0, BORDER));
}

/// Store grid card. Returns `(open_detail, install_clicked)`.
fn plugin_store_card(
    ui: &mut egui::Ui,
    glyph: SidebarGlyph,
    accent: Color32,
    title: &str,
    blurb: &str,
    enabled: bool,
    install_label: &str,
    configure_label: &str,
) -> (bool, bool) {
    let mut open = false;
    let mut install = false;
    Frame::new()
        .fill(Color32::from_rgb(36, 38, 46))
        .corner_radius(CornerRadius::same(12))
        .stroke(Stroke::new(1.0, Color32::from_rgb(56, 58, 68)))
        .inner_margin(Margin {
            left: 14,
            right: 16,
            top: 12,
            bottom: 12,
        })
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                let icon_size = Vec2::splat(36.0);
                let (icon_rect, _) = ui.allocate_exact_size(icon_size, egui::Sense::hover());
                ui.painter().rect_filled(
                    icon_rect,
                    CornerRadius::same(9),
                    Color32::from_rgb(48, 50, 60),
                );
                paint_sidebar_glyph_at(ui.painter(), icon_rect.center(), glyph, accent);
                ui.add_space(12.0);

                // Action first (right-to-left) so text gets remaining width with inset.
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.add_space(4.0);
                    if enabled {
                        if plugin_secondary_btn(ui, configure_label).clicked() {
                            open = true;
                        }
                    } else if plugin_primary_btn(ui, install_label, accent).clicked() {
                        install = true;
                    }
                    ui.add_space(10.0);
                    let text_w = ui.available_width();
                    let text_resp = ui
                        .allocate_ui_with_layout(
                            Vec2::new(text_w, 0.0),
                            egui::Layout::top_down(egui::Align::Min),
                            |ui| {
                                ui.set_max_width(text_w);
                                ui.label(RichText::new(title).size(14.0).strong().color(TEXT));
                                ui.add_space(3.0);
                                ui.add(
                                    egui::Label::new(
                                        RichText::new(blurb).size(12.0).color(MUTED),
                                    )
                                    .wrap(),
                                );
                            },
                        )
                        .response;
                    if text_resp.interact(egui::Sense::click()).clicked() {
                        open = true;
                    }
                });
            });
        });
    (open, install)
}

fn plugin_section(ui: &mut egui::Ui, add: impl FnOnce(&mut egui::Ui)) {
    ui.add_space(16.0);
    add(ui);
    ui.add_space(16.0);
    plugin_divider(ui);
}

fn plugin_enable_switch(ui: &mut egui::Ui, on: bool, accent: Color32, tip: &str) -> egui::Response {
    let size = Vec2::new(38.0, 22.0);
    let (rect, resp) = ui.allocate_exact_size(size, egui::Sense::click());
    let resp = resp.on_hover_text(tip);
    let mut track = if on { accent } else { Color32::from_rgb(58, 58, 66) };
    if resp.hovered() {
        track = if on {
            Color32::from_rgb(
                accent.r().saturating_add(18),
                accent.g().saturating_add(18),
                accent.b().saturating_add(18),
            )
        } else {
            Color32::from_rgb(70, 70, 78)
        };
    }
    ui.painter()
        .rect_filled(rect, CornerRadius::same(11), track);
    let knob_x = if on {
        rect.right() - 11.0
    } else {
        rect.left() + 11.0
    };
    ui.painter().circle_filled(
        Pos2::new(knob_x, rect.center().y),
        7.5,
        Color32::from_rgb(245, 245, 248),
    );
    resp
}

fn plugin_header(
    ui: &mut egui::Ui,
    glyph: SidebarGlyph,
    icon_color: Color32,
    title: &str,
    blurb: &str,
    enable: Option<(bool, Color32, &str)>,
    mut on_toggle: impl FnMut(bool),
) {
    ui.horizontal(|ui| {
        paint_sidebar_glyph(ui, glyph, icon_color);
        ui.add_space(10.0);
        ui.vertical(|ui| {
            ui.label(RichText::new(title).size(15.0).strong().color(TEXT));
            ui.add_space(2.0);
            ui.label(RichText::new(blurb).size(12.0).color(MUTED));
        });
        if let Some((on, accent, tip)) = enable {
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if plugin_enable_switch(ui, on, accent, tip).clicked() {
                    on_toggle(!on);
                }
            });
        }
    });
}

fn plugin_status_line(ui: &mut egui::Ui, color: Color32, label: &str, hint: Option<&str>) {
    ui.horizontal(|ui| {
        let (dot, _) = ui.allocate_exact_size(Vec2::splat(8.0), egui::Sense::hover());
        ui.painter()
            .circle_filled(dot.center(), 3.2, color);
        ui.add_space(4.0);
        ui.label(RichText::new(label).size(12.5).color(color));
        if let Some(hint) = hint {
            ui.label(RichText::new(format!("· {hint}")).size(11.5).color(MUTED));
        }
    });
}

fn plugin_path_row(
    ui: &mut egui::Ui,
    label: &str,
    path: &str,
    action: Option<&str>,
    mut on_action: impl FnMut(),
) {
    ui.horizontal(|ui| {
        ui.label(RichText::new(label).size(12.0).color(MUTED));
        ui.add_space(6.0);
        let action_reserve = if action.is_some() { 52.0 } else { 0.0 };
        let path_w = (ui.available_width() - action_reserve).max(72.0);
        let path_resp = ui
            .allocate_ui_with_layout(
                Vec2::new(path_w, 18.0),
                egui::Layout::left_to_right(egui::Align::Center),
                |ui| {
                    ui.add(
                        egui::Label::new(RichText::new(path).size(12.0).monospace().color(TEXT))
                            .truncate(),
                    )
                },
            )
            .inner;
        let _ = path_resp.on_hover_text(path);
        if let Some(action) = action {
            if plugin_link_btn(ui, action).clicked() {
                on_action();
            }
        }
    });
}

fn plugin_link_btn(ui: &mut egui::Ui, text: &str) -> egui::Response {
    let resp = ui.add(
        egui::Button::new(RichText::new(text).size(12.0).color(MUTED))
            .fill(Color32::TRANSPARENT)
            .stroke(Stroke::NONE)
            .frame(false),
    );
    if resp.hovered() {
        ui.ctx().set_cursor_icon(CursorIcon::PointingHand);
    }
    resp
}

fn plugin_primary_btn(ui: &mut egui::Ui, text: &str, accent: Color32) -> egui::Response {
    ui.add(plugin_primary_btn_widget(text, accent))
}

fn plugin_primary_btn_widget(text: &str, accent: Color32) -> egui::Button<'static> {
    egui::Button::new(RichText::new(text.to_owned()).size(12.5).color(BG).strong())
        .fill(accent)
        .corner_radius(CornerRadius::same(7))
        .min_size(Vec2::new(0.0, 28.0))
}

fn plugin_secondary_btn(ui: &mut egui::Ui, text: &str) -> egui::Response {
    ui.add(plugin_secondary_btn_widget(text))
}

fn plugin_secondary_btn_widget(text: &str) -> egui::Button<'static> {
    egui::Button::new(RichText::new(text.to_owned()).size(12.5).color(TEXT))
        .fill(PANEL_2)
        .stroke(Stroke::NONE)
        .corner_radius(CornerRadius::same(7))
        .min_size(Vec2::new(0.0, 28.0))
}

fn plugin_danger_btn(ui: &mut egui::Ui, text: &str) -> egui::Response {
    ui.add(
        egui::Button::new(RichText::new(text).size(12.5).color(DANGER))
            .fill(PANEL_2)
            .stroke(Stroke::NONE)
            .corner_radius(CornerRadius::same(7))
            .min_size(Vec2::new(0.0, 28.0)),
    )
}

fn stat_chip(ui: &mut egui::Ui, label: &str, value: &str) {
    Frame::new()
        .fill(BG)
        .corner_radius(CornerRadius::same(10))
        .stroke(Stroke::new(1.0, BORDER))
        .inner_margin(Margin::symmetric(14, 10))
        .show(ui, |ui| {
            ui.set_min_width(72.0);
            ui.vertical(|ui| {
                ui.label(RichText::new(label).size(11.0).color(MUTED));
                ui.add_space(2.0);
                ui.label(RichText::new(value).size(15.0).strong().color(TEXT));
            });
        });
}

fn segment_tab(ui: &mut egui::Ui, label: &str, selected: bool) -> bool {
    let fill = if selected {
        SELECTED
    } else {
        Color32::TRANSPARENT
    };
    let stroke = if selected {
        Stroke::new(1.0, ACCENT_BAR)
    } else {
        Stroke::new(1.0, BORDER)
    };
    let resp = Frame::new()
        .fill(fill)
        .corner_radius(CornerRadius::same(8))
        .stroke(stroke)
        .inner_margin(Margin::symmetric(12, 7))
        .show(ui, |ui| {
            ui.label(
                RichText::new(label)
                    .size(12.5)
                    .color(if selected { TEXT } else { MUTED }),
            );
        })
        .response
        .interact(egui::Sense::click());
    if resp.hovered() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    resp.clicked()
}

fn empty_hint(ui: &mut egui::Ui, text: &str) {
    Frame::new()
        .fill(BG)
        .corner_radius(CornerRadius::same(12))
        .stroke(Stroke::new(1.0, BORDER))
        .inner_margin(Margin::symmetric(16, 20))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.vertical_centered(|ui| {
                ui.label(RichText::new(text).size(13.5).color(MUTED));
            });
        });
}

#[derive(Clone, Copy)]
enum NavDir {
    Back,
    Forward,
}

#[derive(Clone, Copy)]
enum PanelSide {
    Left,
    Right,
}

fn panel_toggle_btn(ui: &mut egui::Ui, side: PanelSide, tip: &str, active: bool) -> bool {
    let (rect, resp) = ui.allocate_exact_size(Vec2::splat(26.0), egui::Sense::click());
    let resp = resp.on_hover_text(tip);
    if resp.hovered() || active {
        ui.painter()
            .rect_filled(rect, CornerRadius::same(6), SELECTED);
    }
    let color = if active || resp.hovered() {
        TEXT
    } else {
        MUTED
    };
    let stroke = Stroke::new(1.3, color);
    let outer = egui::Rect::from_center_size(rect.center(), Vec2::new(14.0, 11.0));
    ui.painter().rect_stroke(
        outer,
        CornerRadius::same(1),
        stroke,
        egui::StrokeKind::Outside,
    );
    match side {
        PanelSide::Left => {
            let x = outer.left() + 4.5;
            ui.painter().line_segment(
                [
                    egui::pos2(x, outer.top() + 1.0),
                    egui::pos2(x, outer.bottom() - 1.0),
                ],
                stroke,
            );
            let pane = egui::Rect::from_min_max(
                egui::pos2(outer.left() + 1.0, outer.top() + 1.0),
                egui::pos2(x, outer.bottom() - 1.0),
            );
            ui.painter()
                .rect_filled(pane, CornerRadius::ZERO, color.linear_multiply(0.35));
        }
        PanelSide::Right => {
            let x = outer.right() - 4.5;
            ui.painter().line_segment(
                [
                    egui::pos2(x, outer.top() + 1.0),
                    egui::pos2(x, outer.bottom() - 1.0),
                ],
                stroke,
            );
            let pane = egui::Rect::from_min_max(
                egui::pos2(x, outer.top() + 1.0),
                egui::pos2(outer.right() - 1.0, outer.bottom() - 1.0),
            );
            ui.painter()
                .rect_filled(pane, CornerRadius::ZERO, color.linear_multiply(0.35));
        }
    }
    resp.clicked()
}

fn nav_chevron_btn(ui: &mut egui::Ui, dir: NavDir, tip: &str, enabled: bool) -> bool {
    let (rect, resp) = ui.allocate_exact_size(Vec2::new(28.0, 28.0), egui::Sense::click());
    let resp = resp.on_hover_text(tip);
    // Soft rounded plate on hover (Codex-style).
    if resp.hovered() {
        ui.painter().rect_filled(
            rect.shrink(1.0),
            CornerRadius::same(7),
            Color32::from_rgb(48, 48, 54),
        );
    }

    let color = if !enabled {
        Color32::from_rgb(88, 88, 96)
    } else if resp.hovered() {
        Color32::from_rgb(230, 230, 234)
    } else {
        Color32::from_rgb(168, 168, 176)
    };
    let stroke = Stroke::new(1.6, color);
    let c = rect.center();

    // Shaft + arrowhead (← / →), not a bare chevron.
    let half_len = 6.5;
    let head = 4.2;
    match dir {
        NavDir::Back => {
            let tip = egui::pos2(c.x - half_len, c.y);
            let tail = egui::pos2(c.x + half_len, c.y);
            ui.painter().line_segment([tip, tail], stroke);
            ui.painter()
                .line_segment([egui::pos2(tip.x + head, tip.y - head), tip], stroke);
            ui.painter()
                .line_segment([egui::pos2(tip.x + head, tip.y + head), tip], stroke);
        }
        NavDir::Forward => {
            let tip = egui::pos2(c.x + half_len, c.y);
            let tail = egui::pos2(c.x - half_len, c.y);
            ui.painter().line_segment([tail, tip], stroke);
            ui.painter()
                .line_segment([egui::pos2(tip.x - head, tip.y - head), tip], stroke);
            ui.painter()
                .line_segment([egui::pos2(tip.x - head, tip.y + head), tip], stroke);
        }
    }
    enabled && resp.clicked()
}

fn search_icon_btn(ui: &mut egui::Ui, active: bool) -> bool {
    let (rect, resp) = ui.allocate_exact_size(Vec2::splat(26.0), egui::Sense::click());
    let resp = resp.on_hover_text("搜索任务");
    if resp.hovered() || active {
        ui.painter()
            .rect_filled(rect, CornerRadius::same(6), SELECTED);
    }
    let color = if active || resp.hovered() {
        TEXT
    } else {
        MUTED
    };
    let stroke = Stroke::new(1.4, color);
    let c = egui::pos2(rect.center().x - 1.2, rect.center().y - 1.2);
    let r = 5.2;
    ui.painter().circle_stroke(c, r, stroke);
    let handle_start = egui::pos2(c.x + r * 0.72, c.y + r * 0.72);
    let handle_end = egui::pos2(rect.center().x + 6.2, rect.center().y + 6.2);
    ui.painter()
        .line_segment([handle_start, handle_end], stroke);
    resp.clicked()
}

#[derive(Clone, Copy)]
enum WinChrome {
    Minimize,
    Maximize,
    Restore,
    Close,
}

/// Vector window controls — glyphs often fail with CJK UI fonts.
fn win_chrome_btn(ui: &mut egui::Ui, kind: WinChrome) -> bool {
    let danger = matches!(kind, WinChrome::Close);
    let tip = match kind {
        WinChrome::Minimize => "最小化",
        WinChrome::Maximize => "最大化",
        WinChrome::Restore => "还原",
        WinChrome::Close => "关闭",
    };
    let (rect, resp) = ui.allocate_exact_size(Vec2::new(46.0, TITLE_BAR_H), egui::Sense::click());
    let resp = resp.on_hover_text(tip);

    let hover_bg = if danger {
        Color32::from_rgb(196, 64, 64)
    } else {
        Color32::from_rgb(52, 52, 58)
    };
    if resp.hovered() {
        ui.painter().rect_filled(rect, CornerRadius::ZERO, hover_bg);
    }

    let icon = if resp.hovered() && danger {
        TEXT
    } else if resp.hovered() {
        TEXT
    } else {
        Color32::from_rgb(200, 200, 206)
    };
    let stroke = Stroke::new(1.35, icon);
    let c = rect.center();

    match kind {
        WinChrome::Minimize => {
            let half = 5.5;
            ui.painter().line_segment(
                [egui::pos2(c.x - half, c.y), egui::pos2(c.x + half, c.y)],
                stroke,
            );
        }
        WinChrome::Maximize => {
            let half = 5.0;
            let r = egui::Rect::from_center_size(c, Vec2::splat(half * 2.0));
            ui.painter()
                .rect_stroke(r, CornerRadius::ZERO, stroke, egui::StrokeKind::Outside);
        }
        WinChrome::Restore => {
            let s = 4.2;
            let back = egui::Rect::from_min_size(
                egui::pos2(c.x - s + 1.5, c.y - s - 1.0),
                Vec2::splat(s * 1.7),
            );
            let front = egui::Rect::from_min_size(
                egui::pos2(c.x - s - 1.0, c.y - s + 1.5),
                Vec2::splat(s * 1.7),
            );
            ui.painter()
                .rect_stroke(back, CornerRadius::ZERO, stroke, egui::StrokeKind::Outside);
            ui.painter().rect_filled(front, CornerRadius::ZERO, SIDEBAR);
            ui.painter()
                .rect_stroke(front, CornerRadius::ZERO, stroke, egui::StrokeKind::Outside);
        }
        WinChrome::Close => {
            let half = 5.0;
            ui.painter().line_segment(
                [
                    egui::pos2(c.x - half, c.y - half),
                    egui::pos2(c.x + half, c.y + half),
                ],
                stroke,
            );
            ui.painter().line_segment(
                [
                    egui::pos2(c.x + half, c.y - half),
                    egui::pos2(c.x - half, c.y + half),
                ],
                stroke,
            );
        }
    }

    resp.clicked()
}

#[derive(Clone)]
struct ConversationGroup {
    project_path: std::path::PathBuf,
    is_current: bool,
    /// Indices into `BonyBuildApp::tasks` (avoids cloning TaskState every frame).
    tasks: Vec<usize>,
    archived: Vec<usize>,
}

/// Normalize path strings for cache / group keys (no disk I/O).
fn normalize_path_key(path: &std::path::Path) -> String {
    path.to_string_lossy()
        .replace('\\', "/")
        .trim_end_matches('/')
        .to_lowercase()
}

/// Resolve the primary git checkout for a path.
///
/// Results are memoized: the sidebar used to call this for every project/task
/// on every egui frame, each time spawning `git rev-parse` — on Windows that
/// alone turns hover into a slideshow.
fn canonical_project_root(path: &std::path::Path) -> std::path::PathBuf {
    use std::sync::{Mutex, OnceLock};
    static CACHE: OnceLock<Mutex<std::collections::HashMap<String, std::path::PathBuf>>> =
        OnceLock::new();
    let cache = CACHE.get_or_init(|| Mutex::new(std::collections::HashMap::new()));
    let key = normalize_path_key(path);
    if let Ok(guard) = cache.lock() {
        if let Some(hit) = guard.get(&key) {
            return hit.clone();
        }
    }
    let root = GitWorkspaceService::primary_repo_root(path)
        .ok()
        .flatten()
        .or_else(|| GitWorkspaceService::repo_root(path).ok().flatten())
        .unwrap_or_else(|| path.to_path_buf());
    if let Ok(mut guard) = cache.lock() {
        guard.insert(key, root.clone());
        guard.insert(normalize_path_key(&root), root.clone());
    }
    root
}

fn project_group_key(path: &std::path::Path) -> String {
    // Pure string normalize — never canonicalize (disk) on the hot UI path.
    normalize_path_key(path)
}

fn is_placeholder_task_title(title: &str) -> bool {
    matches!(
        title.trim(),
        "" | "新任务" | "新对话" | "未命名对话" | "未命名任务"
    )
}

fn display_task_title(task: &TaskState) -> String {
    let title = task.title.trim();
    if is_placeholder_task_title(title) {
        return "未命名对话".into();
    }
    // Old data sometimes stored worktree short ids as titles.
    if looks_like_short_id(title) {
        return "未命名对话".into();
    }
    title.chars().take(36).collect()
}

fn looks_like_short_id(s: &str) -> bool {
    let s = s.trim();
    (8..=12).contains(&s.len())
        && s.chars()
            .all(|c| c.is_ascii_hexdigit())
}

fn suggest_task_title(text: &str) -> String {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return "未命名对话".into();
    }
    if let Some(cmd) = parse_unity_chat_command(trimmed) {
        return cmd.chip.to_string();
    }
    if let Some((label, _)) = compile_unity_scene_command(trimmed) {
        return label;
    }
    if wants_unity_help(trimmed)
        || trimmed.contains("对话控制 Unity")
        || trimmed.starts_with("### ")
    {
        return "Unity 说明".into();
    }

    let line = trimmed
        .lines()
        .map(str::trim)
        .find(|l| !l.is_empty())
        .unwrap_or(trimmed);

    // Prefer a short, human line — drop heavy slash prefixes when spoken form exists.
    let mut cleaned = line.trim();
    if let Some(rest) = cleaned.strip_prefix("/unity") {
        cleaned = rest.trim();
        if cleaned.is_empty() {
            return "Unity 操作".into();
        }
    }

    let mut out = String::new();
    for (i, ch) in cleaned.chars().enumerate() {
        if i >= 28 {
            out.push('…');
            break;
        }
        if ch == '\n' || ch == '\r' {
            break;
        }
        out.push(ch);
    }
    let out = out.trim().to_string();
    if out.is_empty() || looks_like_short_id(&out) {
        "未命名对话".into()
    } else {
        out
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SidebarGlyph {
    Plus,
    Chat,
    Unity,
    Doc,
    Plug,
    Folder,
    Archive,
    ChevronRight,
    ChevronDown,
    Close,
}

fn paint_sidebar_glyph(ui: &mut egui::Ui, glyph: SidebarGlyph, color: Color32) {
    let (rect, _) = ui.allocate_exact_size(Vec2::splat(16.0), egui::Sense::hover());
    paint_sidebar_glyph_at(ui.painter(), rect.center(), glyph, color);
}

/// Larger, clearer 「+」 control (ChatGPT / Codex style).
fn composer_plus_btn(ui: &mut egui::Ui, open: bool, tip: &str) -> egui::Response {
    let (rect, resp) = ui.allocate_exact_size(Vec2::splat(30.0), egui::Sense::click());
    let resp = resp.on_hover_text(tip);
    let fill = if open {
        Color32::from_rgb(52, 54, 64)
    } else if resp.hovered() {
        SELECTED
    } else {
        PANEL_2
    };
    ui.painter().circle_filled(rect.center(), 13.0, fill);
    ui.painter().circle_stroke(
        rect.center(),
        13.0,
        Stroke::new(1.0, if open { TEXT } else { BORDER }),
    );
    // Rotate + into × when open for clearer affordance.
    let color = if open || resp.hovered() { TEXT } else { MUTED };
    let stroke = Stroke::new(1.4, color);
    if open {
        let o = 4.2;
        ui.painter().line_segment(
            [
                egui::pos2(rect.center().x - o, rect.center().y - o),
                egui::pos2(rect.center().x + o, rect.center().y + o),
            ],
            stroke,
        );
        ui.painter().line_segment(
            [
                egui::pos2(rect.center().x + o, rect.center().y - o),
                egui::pos2(rect.center().x - o, rect.center().y + o),
            ],
            stroke,
        );
    } else {
        paint_sidebar_glyph_at(ui.painter(), rect.center(), SidebarGlyph::Plus, color);
    }
    resp
}

fn icon_btn(ui: &mut egui::Ui, glyph: SidebarGlyph, tip: &str, active: bool) -> egui::Response {
    let (rect, resp) = ui.allocate_exact_size(Vec2::splat(22.0), egui::Sense::click());
    let resp = resp.on_hover_text(tip);
    if resp.hovered() || active {
        ui.painter()
            .rect_filled(rect, CornerRadius::same(6), SELECTED);
    }
    let color = if active || resp.hovered() { TEXT } else { MUTED };
    paint_sidebar_glyph_at(ui.painter(), rect.center(), glyph, color);
    resp
}

fn danger_icon_btn(ui: &mut egui::Ui, tip: &str) -> egui::Response {
    let (rect, resp) = ui.allocate_exact_size(Vec2::splat(26.0), egui::Sense::click());
    let resp = resp.on_hover_text(tip);
    if resp.hovered() {
        ui.painter().rect_filled(
            rect,
            CornerRadius::same(6),
            Color32::from_rgb(72, 36, 36),
        );
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    let color = if resp.hovered() { DANGER } else { MUTED };
    paint_sidebar_glyph_at(ui.painter(), rect.center(), SidebarGlyph::Close, color);
    resp
}

fn paint_sidebar_glyph_at(
    painter: &egui::Painter,
    c: egui::Pos2,
    glyph: SidebarGlyph,
    color: Color32,
) {
    let stroke = Stroke::new(1.35, color);
    match glyph {
        SidebarGlyph::Plus => {
            painter.line_segment(
                [egui::pos2(c.x - 5.0, c.y), egui::pos2(c.x + 5.0, c.y)],
                stroke,
            );
            painter.line_segment(
                [egui::pos2(c.x, c.y - 5.0), egui::pos2(c.x, c.y + 5.0)],
                stroke,
            );
        }
        SidebarGlyph::Chat => {
            let bubble =
                egui::Rect::from_center_size(c + Vec2::new(0.0, -0.5), Vec2::new(12.0, 9.0));
            painter.rect_stroke(
                bubble,
                CornerRadius::same(3),
                stroke,
                egui::StrokeKind::Outside,
            );
            painter.line_segment(
                [
                    egui::pos2(bubble.left() + 3.0, bubble.bottom()),
                    egui::pos2(bubble.left() + 1.0, bubble.bottom() + 3.0),
                ],
                stroke,
            );
            painter.line_segment(
                [
                    egui::pos2(bubble.left() + 1.0, bubble.bottom() + 3.0),
                    egui::pos2(bubble.left() + 6.0, bubble.bottom()),
                ],
                stroke,
            );
        }
        SidebarGlyph::Unity => {
            let pts = [
                egui::pos2(c.x, c.y - 6.0),
                egui::pos2(c.x + 5.5, c.y),
                egui::pos2(c.x, c.y + 6.0),
                egui::pos2(c.x - 5.5, c.y),
            ];
            painter.line_segment([pts[0], pts[1]], stroke);
            painter.line_segment([pts[1], pts[2]], stroke);
            painter.line_segment([pts[2], pts[3]], stroke);
            painter.line_segment([pts[3], pts[0]], stroke);
            painter.line_segment(
                [pts[0], pts[2]],
                Stroke::new(1.0, color.linear_multiply(0.7)),
            );
        }
        SidebarGlyph::Doc => {
            let page = egui::Rect::from_center_size(c, Vec2::new(10.0, 12.0));
            painter.rect_stroke(
                page,
                CornerRadius::same(1),
                stroke,
                egui::StrokeKind::Outside,
            );
            painter.line_segment(
                [
                    egui::pos2(page.left() + 2.5, page.top() + 3.5),
                    egui::pos2(page.right() - 2.5, page.top() + 3.5),
                ],
                stroke,
            );
            painter.line_segment(
                [
                    egui::pos2(page.left() + 2.5, page.top() + 6.5),
                    egui::pos2(page.right() - 2.5, page.top() + 6.5),
                ],
                stroke,
            );
        }
        SidebarGlyph::Plug => {
            // Simple puzzle / plug: rounded square with a tab.
            let body = egui::Rect::from_center_size(c + Vec2::new(-0.5, 0.5), Vec2::new(9.0, 9.0));
            painter.rect_stroke(
                body,
                CornerRadius::same(2),
                stroke,
                egui::StrokeKind::Outside,
            );
            painter.line_segment(
                [
                    egui::pos2(body.right(), c.y - 2.0),
                    egui::pos2(body.right() + 3.5, c.y - 2.0),
                ],
                stroke,
            );
            painter.line_segment(
                [
                    egui::pos2(body.right(), c.y + 2.0),
                    egui::pos2(body.right() + 3.5, c.y + 2.0),
                ],
                stroke,
            );
            painter.circle_filled(egui::pos2(c.x - 1.5, c.y + 0.5), 1.2, color);
        }
        SidebarGlyph::Folder => {
            let tab =
                egui::Rect::from_min_size(egui::pos2(c.x - 6.0, c.y - 4.5), Vec2::new(5.0, 3.0));
            let body = egui::Rect::from_center_size(c + Vec2::new(0.0, 1.0), Vec2::new(12.0, 8.5));
            painter.rect_stroke(tab, CornerRadius::same(1), stroke, egui::StrokeKind::Outside);
            painter.rect_stroke(
                body,
                CornerRadius::same(1),
                stroke,
                egui::StrokeKind::Outside,
            );
        }
        SidebarGlyph::Archive => {
            let box_r =
                egui::Rect::from_center_size(c + Vec2::new(0.0, 1.0), Vec2::new(11.0, 8.0));
            painter.rect_stroke(
                box_r,
                CornerRadius::same(1),
                stroke,
                egui::StrokeKind::Outside,
            );
            painter.line_segment(
                [
                    egui::pos2(box_r.left(), box_r.top() + 2.5),
                    egui::pos2(box_r.right(), box_r.top() + 2.5),
                ],
                stroke,
            );
            painter.line_segment(
                [
                    egui::pos2(c.x, box_r.top() + 3.5),
                    egui::pos2(c.x, box_r.bottom() - 1.5),
                ],
                stroke,
            );
        }
        SidebarGlyph::ChevronRight => {
            painter.line_segment(
                [
                    egui::pos2(c.x - 2.5, c.y - 4.0),
                    egui::pos2(c.x + 2.0, c.y),
                ],
                stroke,
            );
            painter.line_segment(
                [
                    egui::pos2(c.x + 2.0, c.y),
                    egui::pos2(c.x - 2.5, c.y + 4.0),
                ],
                stroke,
            );
        }
        SidebarGlyph::ChevronDown => {
            painter.line_segment(
                [
                    egui::pos2(c.x - 4.0, c.y - 2.0),
                    egui::pos2(c.x, c.y + 2.5),
                ],
                stroke,
            );
            painter.line_segment(
                [
                    egui::pos2(c.x, c.y + 2.5),
                    egui::pos2(c.x + 4.0, c.y - 2.0),
                ],
                stroke,
            );
        }
        SidebarGlyph::Close => {
            let half = 3.8;
            painter.line_segment(
                [
                    egui::pos2(c.x - half, c.y - half),
                    egui::pos2(c.x + half, c.y + half),
                ],
                stroke,
            );
            painter.line_segment(
                [
                    egui::pos2(c.x + half, c.y - half),
                    egui::pos2(c.x - half, c.y + half),
                ],
                stroke,
            );
        }
    }
}

fn nav_row(ui: &mut egui::Ui, glyph: SidebarGlyph, label: &str, selected: bool) -> bool {
    let hover_id = ui.make_persistent_id(("nav_row_hover", label));
    let hovered = ui
        .ctx()
        .data(|d| d.get_temp::<bool>(hover_id))
        .unwrap_or(false);
    let fill = if selected {
        SELECTED
    } else if hovered {
        HOVER
    } else {
        Color32::TRANSPARENT
    };
    let resp = Frame::new()
        .fill(fill)
        .corner_radius(CornerRadius::same(8))
        .stroke(Stroke::new(
            1.0,
            if selected {
                Color32::from_rgb(70, 72, 84)
            } else {
                Color32::TRANSPARENT
            },
        ))
        .inner_margin(Margin::symmetric(10, 8))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                paint_sidebar_glyph(ui, glyph, if selected || hovered { TEXT } else { MUTED });
                ui.add_space(8.0);
                ui.label(
                    RichText::new(label)
                        .size(13.5)
                        .color(if selected || hovered { TEXT } else { MUTED }),
                );
            });
        })
        .response
        .interact(egui::Sense::click());
    ui.ctx()
        .data_mut(|d| d.insert_temp(hover_id, resp.hovered()));
    if resp.hovered() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    resp.clicked()
}

fn render_recent_inbox_row(
    ui: &mut egui::Ui,
    task_id: &str,
    title: &str,
    project: &str,
    delete_tip: &str,
    selected: bool,
) -> (bool, bool) {
    let width = ui.available_width();
    let row_h = 44.0;
    let (rect, resp) = ui.allocate_exact_size(Vec2::new(width, row_h), egui::Sense::click());
    let hovered = resp.hovered() || resp.contains_pointer();
    let show_chrome = selected || hovered;

    let close_rect = egui::Rect::from_center_size(
        egui::pos2(rect.right() - 16.0, rect.center().y),
        Vec2::splat(26.0),
    );
    let close_resp = ui.interact(
        close_rect,
        ui.id().with(("inbox_row_close", task_id)),
        egui::Sense::click(),
    );

    if show_chrome {
        ui.painter().rect_filled(
            rect,
            CornerRadius::same(8),
            if selected {
                SELECTED
            } else {
                Color32::from_rgb(44, 46, 56)
            },
        );
        ui.painter().rect_stroke(
            rect,
            CornerRadius::same(8),
            Stroke::new(
                1.0,
                if selected {
                    Color32::from_rgb(70, 72, 84)
                } else {
                    Color32::from_rgb(62, 64, 76)
                },
            ),
            egui::StrokeKind::Inside,
        );
        let bar = egui::Rect::from_min_size(
            egui::pos2(rect.left() + 4.0, rect.top() + 10.0),
            Vec2::new(3.0, rect.height() - 20.0),
        );
        ui.painter().rect_filled(
            bar,
            CornerRadius::same(2),
            if selected {
                ACCENT_BAR
            } else {
                Color32::from_rgb(90, 100, 130)
            },
        );
        if !close_resp.hovered() {
            ui.ctx().set_cursor_icon(CursorIcon::PointingHand);
        }
    }

    let fg = if show_chrome { TEXT } else { MUTED };
    paint_sidebar_glyph_at(
        ui.painter(),
        egui::pos2(rect.left() + 22.0, rect.center().y),
        SidebarGlyph::Chat,
        fg,
    );

    let text_left = rect.left() + 36.0;
    let text_right = close_rect.left() - 4.0;
    let text_w = (text_right - text_left).max(40.0);
    ui.painter().text(
        egui::pos2(text_left, rect.top() + 8.0),
        egui::Align2::LEFT_TOP,
        truncate_chip_label(title, ((text_w / 7.0) as usize).max(8)),
        egui::FontId::proportional(12.5),
        fg,
    );
    ui.painter().text(
        egui::pos2(text_left, rect.top() + 24.0),
        egui::Align2::LEFT_TOP,
        truncate_chip_label(project, ((text_w / 6.5) as usize).max(8)),
        egui::FontId::proportional(10.5),
        MUTED,
    );

    if show_chrome {
        if close_resp.hovered() {
            ui.painter().rect_filled(
                close_rect,
                CornerRadius::same(6),
                Color32::from_rgb(72, 36, 36),
            );
            ui.ctx().set_cursor_icon(CursorIcon::PointingHand);
        }
        paint_sidebar_glyph_at(
            ui.painter(),
            close_rect.center(),
            SidebarGlyph::Close,
            if close_resp.hovered() { DANGER } else { MUTED },
        );
    }
    let _ = close_resp.clone().on_hover_text(delete_tip);

    let delete = close_resp.clicked();
    let activate = !delete && resp.clicked();
    (activate, delete)
}

fn task_row_meta(task: &TaskState) -> String {
    let mut parts = Vec::new();
    if task.isolated {
        parts.push("隔离工作区");
    }
    let status = match task.status {
        TaskStatus::Draft => None,
        other => Some(other.label()),
    };
    if let Some(s) = status {
        parts.push(s);
    }
    parts.join(" · ")
}

fn plus_menu_divider(ui: &mut egui::Ui, label: &str) {
    ui.add_space(4.0);
    ui.horizontal(|ui| {
        ui.add_space(8.0);
        ui.label(
            RichText::new(label)
                .size(11.0)
                .color(Color32::from_rgb(120, 122, 132)),
        );
    });
    ui.add_space(2.0);
}

fn plus_menu_row(
    ui: &mut egui::Ui,
    glyph: SidebarGlyph,
    title: &str,
    subtitle: &str,
    active: bool,
) -> bool {
    let width = ui.available_width();
    let (rect, resp) = ui.allocate_exact_size(Vec2::new(width, 44.0), egui::Sense::click());
    let hovered = resp.hovered();
    if hovered || active {
        ui.painter().rect_filled(
            rect,
            CornerRadius::same(9),
            if hovered {
                Color32::from_rgb(38, 40, 48)
            } else {
                Color32::from_rgb(32, 34, 42)
            },
        );
    }
    if active {
        let bar = egui::Rect::from_min_size(
            egui::pos2(rect.left() + 2.0, rect.top() + 10.0),
            Vec2::new(2.5, rect.height() - 20.0),
        );
        ui.painter()
            .rect_filled(bar, CornerRadius::same(2), UNITY_ACCENT);
    }
    if hovered {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }

    let icon_c = egui::pos2(rect.left() + 22.0, rect.center().y);
    let icon_bg = egui::Rect::from_center_size(icon_c, Vec2::splat(28.0));
    ui.painter().rect_filled(
        icon_bg,
        CornerRadius::same(8),
        if active {
            Color32::from_rgb(20, 48, 56)
        } else {
            Color32::from_rgb(34, 36, 44)
        },
    );
    paint_sidebar_glyph_at(
        ui.painter(),
        icon_c,
        glyph,
        if active { UNITY_ACCENT } else { MUTED },
    );

    let text_left = rect.left() + 44.0;
    ui.painter().text(
        egui::pos2(text_left, rect.top() + 10.0),
        Align2::LEFT_TOP,
        title,
        egui::FontId::proportional(13.5),
        TEXT,
    );
    ui.painter().text(
        egui::pos2(text_left, rect.top() + 26.0),
        Align2::LEFT_TOP,
        subtitle,
        egui::FontId::proportional(11.0),
        MUTED,
    );

    if active {
        // Small check on the right.
        let c = egui::pos2(rect.right() - 16.0, rect.center().y);
        let s = Stroke::new(1.6, UNITY_ACCENT);
        ui.painter().line_segment(
            [egui::pos2(c.x - 4.0, c.y), egui::pos2(c.x - 1.0, c.y + 3.5)],
            s,
        );
        ui.painter().line_segment(
            [egui::pos2(c.x - 1.0, c.y + 3.5), egui::pos2(c.x + 5.0, c.y - 4.0)],
            s,
        );
    }

    resp.clicked()
}

fn truncate_chip_label(s: &str, max: usize) -> String {
    let mut out = String::new();
    for (i, ch) in s.chars().enumerate() {
        if i >= max {
            out.push('…');
            break;
        }
        out.push(ch);
    }
    out
}

fn soft_chip(ui: &mut egui::Ui, text: &str, enabled: bool) -> bool {
    let color = if enabled { TEXT } else { MUTED };
    let resp = Frame::new()
        .fill(PANEL_2)
        .corner_radius(CornerRadius::same(10))
        .inner_margin(Margin::symmetric(10, 5))
        .stroke(Stroke::new(1.0, BORDER))
        .show(ui, |ui| {
            ui.label(RichText::new(text).size(12.0).color(color));
        })
        .response
        .interact(egui::Sense::click());
    if enabled && resp.hovered() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    enabled && resp.clicked()
}

#[allow(dead_code)]
fn menu_row(ui: &mut egui::Ui, label: &str, with_chevron: bool) -> bool {
    let resp = Frame::new()
        .fill(Color32::TRANSPARENT)
        .corner_radius(CornerRadius::same(8))
        .inner_margin(Margin::symmetric(8, 8))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                ui.label(RichText::new(label).size(13.5).color(TEXT));
                if with_chevron {
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.label(RichText::new(">").size(13.0).color(MUTED));
                    });
                }
            });
        })
        .response
        .interact(egui::Sense::click());
    if resp.hovered() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    resp.clicked()
}

#[derive(Clone, Copy)]
enum AccountRowKind {
    Plain,
    Chevron,
    Muted,
}

fn thin_menu_rule(ui: &mut egui::Ui) {
    let y = ui.cursor().top();
    ui.painter().hline(
        ui.max_rect().x_range().shrink(4.0),
        y,
        Stroke::new(1.0, Color32::from_rgb(42, 42, 50)),
    );
    ui.add_space(1.0);
}

fn account_menu_row(ui: &mut egui::Ui, label: &str, kind: AccountRowKind) -> bool {
    let width = ui.available_width();
    let (rect, resp) = ui.allocate_exact_size(Vec2::new(width, 34.0), egui::Sense::click());
    if resp.hovered() {
        ui.painter()
            .rect_filled(rect, CornerRadius::same(8), Color32::from_rgb(36, 38, 46));
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    let color = match kind {
        AccountRowKind::Muted => MUTED,
        _ => TEXT,
    };
    ui.painter().text(
        egui::pos2(rect.left() + 10.0, rect.center().y),
        Align2::LEFT_CENTER,
        label,
        egui::FontId::proportional(13.0),
        color,
    );
    if matches!(kind, AccountRowKind::Chevron) {
        ui.painter().text(
            egui::pos2(rect.right() - 12.0, rect.center().y),
            Align2::RIGHT_CENTER,
            "›",
            egui::FontId::proportional(16.0),
            MUTED,
        );
    }
    resp.clicked()
}

/// Compact language segmented control. Returns newly selected language if changed.
fn language_segment(ui: &mut egui::Ui, current: Language) -> Option<Language> {
    let langs = Language::all();
    let seg_w = 44.0;
    let seg_h = 24.0;
    let pad = 2.0;
    let total = Vec2::new(seg_w * langs.len() as f32 + pad * 2.0, seg_h + pad * 2.0);
    let (rect, _) = ui.allocate_exact_size(total, egui::Sense::hover());
    ui.painter().rect_filled(
        rect,
        CornerRadius::same(7),
        Color32::from_rgb(32, 34, 42),
    );

    let mut chosen = None;
    for (i, lang) in langs.iter().enumerate() {
        let x = rect.left() + pad + i as f32 * seg_w;
        let seg = egui::Rect::from_min_size(
            egui::pos2(x, rect.top() + pad),
            Vec2::new(seg_w, seg_h),
        );
        let id = ui.id().with("lang_seg").with(lang.native_name());
        let resp = ui.interact(seg, id, egui::Sense::click());
        let selected = *lang == current;
        if selected || resp.hovered() {
            ui.painter().rect_filled(
                seg,
                CornerRadius::same(5),
                if selected {
                    Color32::from_rgb(48, 50, 60)
                } else {
                    Color32::from_rgb(40, 42, 50)
                },
            );
        }
        ui.painter().text(
            seg.center(),
            Align2::CENTER_CENTER,
            match lang {
                Language::Zh => "中",
                Language::En => "EN",
            },
            egui::FontId::proportional(11.5),
            if selected { TEXT } else { MUTED },
        );
        if resp.hovered() {
            ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
        }
        if resp.clicked() && !selected {
            chosen = Some(*lang);
        }
    }
    chosen
}

fn avatar_circle(ui: &mut egui::Ui, initials: &str) {
    let (rect, _) = ui.allocate_exact_size(Vec2::splat(28.0), egui::Sense::hover());
    ui.painter().circle_filled(rect.center(), 14.0, AVATAR);
    ui.painter().text(
        rect.center(),
        egui::Align2::CENTER_CENTER,
        initials,
        egui::FontId::proportional(11.0),
        TEXT,
    );
}

fn truncate_chars(s: &str, max: usize) -> String {
    let mut out = String::new();
    for (i, ch) in s.chars().enumerate() {
        if i >= max {
            out.push('…');
            break;
        }
        out.push(ch);
    }
    out
}

/// Screen-space cursor in egui points (monitor coordinates).
/// Used for title-bar dragging so the window stays glued to the cursor.
#[cfg(target_os = "windows")]
fn screen_cursor_pos_points(pixels_per_point: f32) -> Option<Pos2> {
    windows_sys_get_cursor_pos().map(|(x, y)| {
        Pos2::new(x as f32 / pixels_per_point, y as f32 / pixels_per_point)
    })
}

#[cfg(target_os = "windows")]
fn windows_sys_get_cursor_pos() -> Option<(i32, i32)> {
    #[repr(C)]
    struct Point {
        x: i32,
        y: i32,
    }
    #[link(name = "user32")]
    unsafe extern "system" {
        fn GetCursorPos(lp_point: *mut Point) -> i32;
    }
    let mut pt = Point { x: 0, y: 0 };
    // SAFETY: GetCursorPos writes a POINT; failure returns 0.
    let ok = unsafe { GetCursorPos(&mut pt) };
    if ok != 0 {
        Some((pt.x, pt.y))
    } else {
        None
    }
}

#[cfg(not(target_os = "windows"))]
fn screen_cursor_pos_points(_pixels_per_point: f32) -> Option<Pos2> {
    None
}

fn short_status(status: &str) -> &str {
    if status.contains("Completed") || status.contains("completed") {
        "完成"
    } else if status.contains("InProgress") || status.contains("started") {
        "运行中"
    } else if status.contains("Failed") || status.contains("Error") {
        "失败"
    } else {
        status
    }
}

fn configure_style(ctx: &egui::Context) {
    let mut visuals = egui::Visuals::dark();
    visuals.panel_fill = BG;
    visuals.window_fill = PANEL;
    visuals.extreme_bg_color = PANEL_2;
    visuals.widgets.inactive.bg_fill = PANEL;
    visuals.widgets.hovered.bg_fill = PANEL_2;
    visuals.widgets.active.bg_fill = PANEL_2;
    visuals.selection.bg_fill = Color32::from_rgb(70, 70, 82);
    visuals.override_text_color = Some(TEXT);
    visuals.widgets.noninteractive.bg_fill = Color32::from_rgb(40, 42, 50);
    visuals.widgets.inactive.bg_stroke = Stroke::new(1.0, BORDER);
    ctx.set_visuals(visuals);

    let mut style = (*ctx.style()).clone();
    style.spacing.item_spacing = Vec2::new(8.0, 6.0);
    style.spacing.button_padding = Vec2::new(12.0, 6.0);
    style.spacing.scroll.bar_width = 10.0;
    style.spacing.scroll.handle_min_length = 32.0;
    // Sidebar rows use deferred hover chrome — keep tooltips snappy too.
    style.interaction.tooltip_delay = 0.05;
    style.interaction.show_tooltips_only_when_still = false;
    ctx.set_style(style);
}
