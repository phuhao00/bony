//! Desktop UI localization (Chinese / English).

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

/// Supported UI languages.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Language {
    /// Simplified Chinese (product default).
    #[default]
    #[serde(alias = "zh", alias = "zh-cn", alias = "zh_cn", alias = "cn")]
    Zh,
    #[serde(alias = "en", alias = "en-us", alias = "en_us")]
    En,
}

impl Language {
    pub fn code(self) -> &'static str {
        match self {
            Self::Zh => "zh",
            Self::En => "en",
        }
    }

    pub fn native_name(self) -> &'static str {
        match self {
            Self::Zh => "中文",
            Self::En => "English",
        }
    }

    pub fn all() -> [Self; 2] {
        [Self::Zh, Self::En]
    }

    /// Best-effort OS locale detection; falls back to Chinese.
    pub fn detect() -> Self {
        if let Ok(lang) = std::env::var("BONY_LANG") {
            return Self::from_tag(&lang).unwrap_or_default();
        }
        for key in ["LANG", "LC_ALL", "LC_MESSAGES"] {
            if let Ok(v) = std::env::var(key) {
                if let Some(l) = Self::from_tag(&v) {
                    return l;
                }
            }
        }
        #[cfg(target_os = "windows")]
        {
            if let Some(l) = windows_ui_language() {
                return l;
            }
        }
        Self::Zh
    }

    pub fn from_tag(tag: &str) -> Option<Self> {
        let t = tag.trim().to_ascii_lowercase();
        if t.is_empty() {
            return None;
        }
        if t.starts_with("zh") || t.contains("chinese") || t == "cn" {
            return Some(Self::Zh);
        }
        if t.starts_with("en") || t.contains("english") {
            return Some(Self::En);
        }
        None
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UiPrefs {
    #[serde(default)]
    pub language: Language,
}

impl Default for UiPrefs {
    fn default() -> Self {
        Self {
            language: Language::detect(),
        }
    }
}

fn ui_prefs_path() -> PathBuf {
    crate::usage::usage_dir().join("ui.json")
}

pub fn load_ui_prefs() -> UiPrefs {
    let Ok(text) = std::fs::read_to_string(ui_prefs_path()) else {
        return UiPrefs::default();
    };
    serde_json::from_str(&text).unwrap_or_default()
}

pub fn save_ui_prefs(prefs: &UiPrefs) {
    let dir = crate::usage::usage_dir();
    if std::fs::create_dir_all(&dir).is_err() {
        return;
    }
    if let Ok(text) = serde_json::to_string_pretty(prefs) {
        let _ = std::fs::write(ui_prefs_path(), text);
    }
}

#[cfg(target_os = "windows")]
fn windows_ui_language() -> Option<Language> {
    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn GetUserDefaultUILanguage() -> u16;
    }
    // SAFETY: Win32 getter, no pointers.
    let langid = unsafe { GetUserDefaultUILanguage() };
    let primary = langid & 0x3ff;
    // LANG_CHINESE = 0x04, LANG_ENGLISH = 0x09
    match primary {
        0x04 => Some(Language::Zh),
        0x09 => Some(Language::En),
        _ => None,
    }
}

#[inline]
fn tr(lang: Language, zh: &'static str, en: &'static str) -> &'static str {
    match lang {
        Language::Zh => zh,
        Language::En => en,
    }
}

/// Translate a UI message key.
pub fn t<'a>(lang: Language, key: &'a str) -> &'a str {
    match key {
        // —— App chrome / menus ——
        "app.name" => "Bony Build",
        "menu.file" => tr(lang, "文件", "File"),
        "menu.edit" => tr(lang, "编辑", "Edit"),
        "menu.view" => tr(lang, "视图", "View"),
        "menu.help" => tr(lang, "帮助", "Help"),
        "menu.new_task" => tr(lang, "新建对话", "New chat"),
        "menu.open_project" => tr(lang, "打开项目…", "Open project…"),
        "menu.open_project_short" => tr(lang, "打开项目", "Open project"),
        "menu.quit" => tr(lang, "退出", "Quit"),
        "menu.focus_composer" => tr(lang, "聚焦输入框", "Focus input"),
        "menu.clear_draft" => tr(lang, "清空草稿", "Clear draft"),
        "menu.hide_sidebar" => tr(lang, "隐藏侧栏", "Hide sidebar"),
        "menu.show_sidebar" => tr(lang, "显示侧栏", "Show sidebar"),
        "menu.hide_right" => tr(lang, "隐藏右侧栏", "Hide right panel"),
        "menu.show_right" => tr(lang, "显示右侧栏", "Show right panel"),
        "menu.usage" => tr(lang, "使用统计", "Usage"),
        "menu.plugins" => tr(lang, "插件管理", "Plugins"),
        "menu.about" => tr(lang, "关于 Bony Build", "About Bony Build"),
        "tip.toggle_sidebar" => tr(lang, "切换侧栏", "Toggle sidebar"),
        "tip.back_live" => tr(lang, "返回当前会话", "Back to live session"),
        "tip.forward" => tr(lang, "前进", "Forward"),
        "tip.hide_right" => tr(lang, "隐藏右侧栏", "Hide right panel"),
        "tip.show_right" => tr(lang, "显示右侧栏", "Show right panel"),

        // —— Sidebar / nav ——
        "nav.chat" => tr(lang, "聊天", "Chat"),
        "nav.unity" => tr(lang, "Unity 控制", "Unity control"),
        "nav.scheduled" => tr(lang, "已安排", "Scheduled"),
        "nav.plugins" => tr(lang, "插件", "Plugins"),
        "nav.sites" => tr(lang, "站点", "Sites"),
        "nav.prs" => tr(lang, "拉取请求", "Pull requests"),
        "nav.new_task" => tr(lang, "新建对话", "New chat"),
        "sidebar.filter_tasks" => tr(lang, "筛选任务…", "Filter tasks…"),
        "sidebar.by_project" => tr(lang, "项目", "Projects"),
        "sidebar.no_chats" => tr(lang, "暂无对话", "No conversations"),
        "sidebar.no_history" => tr(lang, "还没有对话记录", "No conversations yet"),
        "sidebar.no_match" => tr(lang, "没有匹配的对话", "No matching conversations"),
        "sidebar.switch_project" => tr(lang, "切换到此项目", "Switch to project"),
        "sidebar.remove_from_list" => tr(lang, "从列表移除", "Remove from list"),
        "sidebar.search" => tr(lang, "搜索任务", "Search tasks"),
        "empty.prompt" => tr(lang, "接下来做什么？", "What's next?"),
        "empty.login" => tr(lang, "请先登录或配置 API Key", "Sign in or configure an API key"),
        "empty.recent" => tr(lang, "最近对话", "Recent chats"),
        "empty.expand_more" => tr(lang, "展开更多", "Show more"),
        "empty.collapse" => tr(lang, "收起", "Show less"),
        "empty.no_recent" => tr(lang, "还没有最近对话", "No recent chats yet"),
        "empty.delete_chat" => tr(lang, "删除对话", "Delete chat"),

        // —— Composer ——
        "composer.hint" => tr(lang, "随便问…", "Do anything"),
        "composer.hint_login" => tr(
            lang,
            "请先登录或配置 API Key…",
            "Sign in or configure an API key…",
        ),
        "composer.hint_unity" => tr(
            lang,
            "描述要对编辑器做的事…",
            "Describe what to do in the editor…",
        ),
        "composer.hint_connecting" => tr(lang, "正在连接…", "Connecting…"),
        "composer.hint_history" => tr(
            lang,
            "要求后续变更（将回到当前会话）…",
            "Request changes (returns to live session)…",
        ),
        "composer.send" => tr(lang, "发送", "Send"),
        "composer.stop" => tr(lang, "停止", "Stop"),
        "composer.force_stop" => tr(lang, "强制停止", "Force stop"),
        "composer.send_hint" => tr(lang, "发送 · Enter", "Send · Enter"),
        "composer.need_login" => tr(
            lang,
            "请先登录或配置 API Key",
            "Sign in or configure an API key first",
        ),
        "composer.busy" => tr(
            lang,
            "正在处理上一轮，请稍候或点停止",
            "Busy — wait or stop the current turn",
        ),
        "composer.connecting" => tr(
            lang,
            "正在连接 agent，连上后即可发送",
            "Connecting to agent — send when ready",
        ),
        "composer.empty" => tr(
            lang,
            "先输入消息，或用 + 添加文件",
            "Type a message, or use + to attach files",
        ),
        "composer.cant_send" => tr(lang, "暂时无法发送", "Cannot send right now"),
        "composer.plus_open" => tr(lang, "关闭菜单", "Close menu"),
        "composer.plus_closed" => tr(lang, "添加文件或插件", "Add files or plugins"),
        "composer.processing" => tr(lang, "处理中…", "Working…"),
        "composer.readonly_history" => tr(
            lang,
            "只读历史 · 发送新消息将回到当前会话",
            "Read-only history · Sending returns to the live session",
        ),

        // —— Process flow (thought / plan / tool) ——
        "flow.thought" => tr(lang, "思考", "Thinking"),
        "flow.plan" => tr(lang, "计划", "Plan"),
        "flow.tool" => tr(lang, "工具", "Tool"),
        "flow.plan_pending" => tr(lang, "待办", "Pending"),
        "flow.plan_running" => tr(lang, "进行中", "In progress"),
        "flow.plan_done" => tr(lang, "完成", "Done"),
        "flow.plan_steps" => tr(lang, "{} 步", "{} steps"),

        // —— Plus menu ——
        "plus.add_file" => tr(lang, "添加文件", "Add Files"),
        "plus.add_file_sub" => tr(lang, "加入当前对话上下文", "Attach to this conversation"),
        "plus.menu_title" => tr(lang, "添加到对话", "Add to Chat"),
        "plus.section_plugins" => tr(lang, "插件", "Plugins"),
        "plus.unity" => tr(lang, "Unity 控制", "Unity Control"),
        "plus.unity_on" => tr(lang, "此对话已启用", "On for this chat"),
        "plus.unity_off" => tr(lang, "本地 CLI · 不经 Agent", "Local CLI · not via Agent"),
        "plus.unity_disabled" => tr(lang, "去插件页启用", "Enable in Plugins"),
        "plus.openmontage" => tr(lang, "OpenMontage", "OpenMontage"),
        "plus.openmontage_on" => tr(lang, "Skill 已启用", "Skill enabled"),
        "plus.openmontage_off" => tr(lang, "视频管线 · Agent Skill", "Video pipelines · Agent skill"),
        "plus.openmontage_setup" => tr(lang, "需要先安装并启用", "Install & enable first"),
        "plus.bevy" => tr(lang, "Bevy 游戏引擎", "Bevy Engine"),
        "plus.bevy_on" => tr(lang, "Skill 已启用", "Skill enabled"),
        "plus.bevy_off" => tr(lang, "Rust ECS · Agent Skill", "Rust ECS · Agent skill"),
        "plus.bevy_setup" => tr(lang, "需要先创建/选择项目", "Create or pick a project first"),
        "plus.status_on" => tr(lang, "开", "On"),
        "plus.status_off" => tr(lang, "关", "Off"),
        "plus.status_setup" => tr(lang, "设置", "Setup"),
        "plus.manage" => tr(lang, "管理插件…", "Manage Plugins…"),
        "plus.manage_sub" => tr(lang, "安装、配置与文档", "Install, configure, and docs"),
        "plus.chip_unity_tip" => tr(
            lang,
            "此对话使用 Unity CLI；点 × 取消",
            "Unity CLI for this chat · click × to leave",
        ),
        "plus.chip_om_tip" => tr(
            lang,
            "OpenMontage Skill 已启用；点 × 关闭",
            "OpenMontage skill on · click × to disable",
        ),
        "plus.chip_bevy_tip" => tr(
            lang,
            "Bevy Skill 已启用；点 × 关闭",
            "Bevy skill on · click × to disable",
        ),
        "plus.chip_project_tip" => tr(lang, "点击切换项目", "Click to switch project"),
        "plus.clear_files" => tr(lang, "清除文件", "Clear files"),

        // —— User menu / settings ——
        "user.usage" => tr(lang, "使用统计", "Usage"),
        "user.edit_config" => tr(lang, "编辑 config.toml", "Edit config.toml"),
        "user.language" => tr(lang, "语言", "Language"),
        "user.full_control" => tr(lang, "完全控制", "Full Control"),
        "user.full_control_tip" => tr(
            lang,
            "开启后自动批准工具执行（含终端 / 写文件），可随时关闭",
            "Auto-approve tool runs (shell, file writes). Turn off anytime.",
        ),
        "user.login" => tr(lang, "登录", "Sign in"),
        "user.relogin" => tr(lang, "重新登录", "Sign in again"),
        "user.open_failed" => tr(lang, "无法打开配置", "Could not open config"),
        "user.local_account" => tr(lang, "本机账号", "Local account"),
        "user.signed_out" => tr(lang, "未登录", "Not signed in"),
        "user.settings_section" => tr(lang, "设置", "Settings"),

        "perm.title" => tr(lang, "需要批准", "Approval needed"),
        "perm.blurb" => tr(
            lang,
            "助手想执行需要你确认的工具。",
            "The assistant wants to run a tool that needs your OK.",
        ),
        "perm.full_control" => tr(lang, "完全控制（以后不再问）", "Full Control (don't ask again)"),

        // —— Right panel / session ——
        "panel.details" => tr(lang, "详情", "Details"),
        "panel.session" => tr(lang, "会话", "Session"),
        "panel.cwd" => tr(lang, "工作目录", "Working directory"),
        "panel.model" => tr(lang, "模型", "Model"),
        "panel.token" => "Token",
        "panel.open_usage" => tr(lang, "打开使用统计", "Open usage"),
        "panel.back_chat" => tr(lang, "回到聊天", "Back to chat"),

        // —— Plugins page ——
        "plugins.title" => tr(lang, "插件", "Plugins"),
        "plugins.blurb" => tr(
            lang,
            "配置本地扩展。对话输入框点「+」即可挂上或拿掉。",
            "Configure local extensions. Use + in chat to attach or remove them.",
        ),
        "plugins.tab_plugins" => tr(lang, "插件", "Plugins"),
        "plugins.tab_skills" => tr(lang, "Skills", "Skills"),
        "plugins.skills_title" => tr(lang, "Skills", "Skills"),
        "plugins.skills_blurb" => tr(
            lang,
            "把能力说明（SKILL.md）挂到对话里，让代理按规范工作。",
            "Attach SKILL.md playbooks so the agent follows project conventions.",
        ),
        "plugins.search" => tr(lang, "搜索…", "Search…"),
        "plugins.search_skills" => tr(lang, "搜索…", "Search…"),
        "plugins.installed" => tr(lang, "已安装", "Installed"),
        "plugins.featured" => tr(lang, "精选", "Featured"),
        "plugins.cat_gamedev" => tr(lang, "游戏开发", "Game Dev"),
        "plugins.cat_video" => tr(lang, "视频", "Video"),
        "plugins.install" => tr(lang, "安装", "Install"),
        "plugins.configure" => tr(lang, "配置", "Configure"),
        "plugins.refresh" => tr(lang, "刷新", "Refresh"),
        "plugins.no_results" => tr(lang, "没有匹配的结果", "No matching results"),
        "plugins.none_installed" => tr(lang, "尚未安装", "Nothing installed yet"),
        "plugins.back_store" => tr(lang, "← 返回商店", "← Back to store"),
        "plugins.skill_om_title" => tr(lang, "OpenMontage Skill", "OpenMontage Skill"),
        "plugins.skill_om_blurb" => tr(
            lang,
            "视频流水线导演说明，写入对话可用的 SKILL.md",
            "Stage-director playbook for video pipelines via SKILL.md",
        ),
        "plugins.skill_bevy_title" => tr(lang, "Bevy Skill", "Bevy Skill"),
        "plugins.skill_bevy_blurb" => tr(
            lang,
            "Bevy ECS 编码规范与项目路径，供对话自动引用",
            "Bevy ECS conventions and project path for chat",
        ),
        "plugins.skill_path" => tr(lang, "SKILL.md", "SKILL.md"),
        "chip.pick_project" => tr(lang, "选择项目", "Choose project"),
        "chip.pick_project_tip" => tr(
            lang,
            "新建对话尚未绑定项目，点击选择",
            "This chat has no project yet — click to choose one",
        ),
        "chip.switch_project_tip" => tr(lang, "点击切换项目", "Click to switch project"),

        // —— VCS panel (jj-inspired, Git backend) ——
        "vcs.working_copy" => tr(lang, "工作副本", "Working copy"),
        "vcs.branch" => tr(lang, "分支", "Branch"),
        "vcs.session_branch" => tr(lang, "会话分支", "Session branch"),
        "vcs.refresh" => tr(lang, "刷新", "Refresh"),
        "vcs.commit" => tr(lang, "描述并提交", "Describe & commit"),
        "vcs.commit_title" => tr(lang, "描述变更", "Describe change"),
        "vcs.commit_hint" => tr(
            lang,
            "简要说明这次改了什么…",
            "Briefly describe this change…",
        ),
        "vcs.commit_need_message" => tr(
            lang,
            "先填写上方的提交说明",
            "Enter a commit message above first",
        ),
        "vcs.restore" => tr(lang, "还原", "Restore"),
        "vcs.restore_confirm_title" => tr(lang, "确认还原", "Confirm restore"),
        "vcs.restore_confirm_body" => tr(
            lang,
            "将丢弃该文件的本地修改（含暂存区）：",
            "Discard local edits to this file (including the index):",
        ),
        "vcs.empty_changes" => tr(lang, "工作副本干净，没有未提交变更。", "Working copy is clean."),
        "vcs.no_repo" => tr(
            lang,
            "当前工作目录不是 Git 仓库，版本面板不可用。",
            "This working directory is not a Git repository.",
        ),
        "vcs.empty_diff" => tr(lang, "暂无 diff", "No diff"),
        "vcs.history" => tr(lang, "最近历史", "Recent history"),
        "vcs.empty_history" => tr(lang, "还没有提交记录。", "No commits yet."),
        "vcs.commit_detail" => tr(lang, "提交详情", "Commit detail"),
        "vcs.changes_tab" => tr(lang, "变更文件", "Changes"),
        "vcs.pick_file" => tr(lang, "选择左侧文件查看 diff", "Select a file to view its diff"),
        "vcs.commit_open_tip" => tr(lang, "点击查看本次提交的变更文件", "Click to view changed files"),
        "vcs.close_detail" => tr(lang, "关闭", "Close"),
        "vcs.resize_tip" => tr(
            lang,
            "拖拽左侧边缘可加宽面板",
            "Drag the left edge to widen this panel",
        ),
        "plugins.unity_title" => tr(lang, "Unity 控制", "Unity control"),
        "plugins.unity_blurb" => tr(
            lang,
            "本地 CLI 驱动编辑器：Pipeline、探测、场景操作",
            "Local CLI for the editor: Pipeline, probe, scene actions",
        ),
        "plugins.openmontage_title" => tr(lang, "OpenMontage", "OpenMontage"),
        "plugins.openmontage_blurb" => tr(
            lang,
            "把 12 条视频流水线接入当前对话，不切换工作区",
            "Attach 12 video pipelines to this chat without switching workspace",
        ),
        "plugins.openmontage_path" => tr(lang, "安装目录", "Install path"),
        "plugins.openmontage_change" => tr(lang, "更改", "Change"),
        "plugins.openmontage_install" => tr(lang, "安装 OpenMontage", "Install OpenMontage"),
        "plugins.openmontage_reinstall_deps" => tr(lang, "重新安装依赖", "Reinstall deps"),
        "plugins.openmontage_retry" => tr(lang, "重试", "Retry"),
        "plugins.openmontage_backlot" => tr(lang, "打开 Backlot", "Open Backlot"),
        "plugins.openmontage_prereq" => tr(
            lang,
            "需要本机已安装 Git、Python 3、Node.js",
            "Requires Git, Python 3, and Node.js on this machine",
        ),
        "plugins.openmontage_installing_hint" => tr(
            lang,
            "安装中请勿关闭应用",
            "Do not close the app while installing",
        ),
        "plugins.openmontage_enabled_hint" => tr(
            lang,
            "对话里提到做视频时，会自动走 OpenMontage 流水线。",
            "Video requests in chat use OpenMontage pipelines automatically.",
        ),
        "plugins.openmontage_status_ready" => tr(
            lang,
            "依赖就绪",
            "Deps ready",
        ),
        "plugins.openmontage_ready_hint" => tr(
            lang,
            "依赖已装好。视频任务由本机 CLI 跑——应用会注入预检结果，勿用 python -c。",
            "Deps installed. Video runs via local CLI — app injects preflight; never python -c.",
        ),
        "plugins.openmontage_status_missing" => tr(lang, "未安装", "Not installed"),
        "plugins.openmontage_status_installing" => tr(lang, "安装中…", "Installing…"),
        "plugins.openmontage_status_failed" => tr(lang, "安装失败", "Install failed"),
        "plugins.openmontage_status_deps" => tr(lang, "缺依赖", "Missing deps"),
        "plugins.enable" => tr(lang, "启用", "Enable"),
        "plugins.status" => tr(lang, "状态", "Status"),
        "plugins.chat_hint" => tr(
            lang,
            "可在聊天输入旁开启 Unity 模式",
            "Enable Unity mode next to chat input",
        ),
        "plugins.open_settings" => tr(lang, "设置", "Settings"),
        "plugins.docs" => tr(lang, "说明", "Docs"),
        "plugins.docs_tip" => tr(
            lang,
            "查看 /unity 指令与常用说法",
            "View /unity commands and phrases",
        ),
        "plugins.use_in_chat" => tr(lang, "在聊天中使用", "Use in chat"),
        "plugins.enabled_hint" => tr(
            lang,
            "启用后可在聊天里用本地 CLI 控制 Unity。",
            "When enabled, chat can drive Unity via local CLI.",
        ),
        "plugins.project" => tr(lang, "项目", "Project"),
        "plugins.pick_existing" => tr(lang, "选择已有", "Pick existing"),
        "plugins.bevy_title" => tr(lang, "Bevy 游戏引擎", "Bevy game engine"),
        "plugins.bevy_blurb" => tr(
            lang,
            "直接写改 ECS 代码，cargo run 看效果（无可视化编辑器）",
            "Edit ECS code directly; cargo run to preview (no visual editor)",
        ),
        "plugins.bevy_running" => tr(lang, "游戏窗口运行中", "Game window running"),
        "plugins.bevy_no_rust" => tr(
            lang,
            "本机未检测到 Rust 工具链（cargo / rustc）",
            "Rust toolchain not found (cargo / rustc)",
        ),
        "plugins.bevy_install_rust" => tr(lang, "一键安装 Rust", "Install Rust"),
        "plugins.bevy_install_rust_tip" => tr(
            lang,
            "应用内运行安装命令并自动重新检测",
            "Runs the install command in-app and re-detects",
        ),
        "plugins.bevy_copy" => tr(lang, "复制", "Copy"),
        "plugins.bevy_copied" => tr(lang, "安装命令已复制", "Install command copied"),
        "plugins.bevy_no_project" => tr(
            lang,
            "还没有 Bevy 项目：新建（依赖 phuhao000/bevy fork），或选择已有。",
            "No Bevy project yet — create one (phuhao000/bevy fork) or pick an existing folder.",
        ),
        "plugins.bevy_project_name" => tr(lang, "项目名", "Project name"),
        "plugins.bevy_create" => tr(lang, "创建新项目", "Create project"),
        "plugins.bevy_check_tip" => tr(
            lang,
            "快速语法/类型检查，不生成可执行文件",
            "Quick typecheck without producing a binary",
        ),
        "plugins.bevy_run" => tr(lang, "运行", "Run"),
        "plugins.bevy_run_tip" => tr(
            lang,
            "cargo run（首次编译可能需要几分钟）",
            "cargo run (first build may take a few minutes)",
        ),
        "plugins.bevy_stop" => tr(lang, "停止", "Stop"),
        "plugins.bevy_detecting" => tr(lang, "检测中…", "Detecting…"),
        "plugins.bevy_enabled_hint" => tr(
            lang,
            "对话里写/改 Bevy 时会自动带上这个项目的规范。",
            "Chat about Bevy will include this project's conventions.",
        ),
        "plugins.more" => tr(lang, "更多插件", "More plugins"),
        "plugins.more_blurb" => tr(
            lang,
            "后续会在这里扩展更多本地能力。",
            "More local capabilities will appear here.",
        ),
        "plugins.back" => tr(lang, "← 插件", "← Plugins"),

        // —— Common ——
        "common.cancel" => tr(lang, "取消", "Cancel"),
        "common.close" => tr(lang, "关闭", "Close"),
        "common.delete" => tr(lang, "删除记录", "Delete record"),
        "common.demo_mode" => tr(lang, "· 演示模式", "· Demo mode"),
        "task.rename_hint" => tr(lang, "任务名称", "Task name"),
        "task.delete_note" => tr(
            lang,
            "不会自动删除 worktree 或其中未提交的修改。",
            "Does not delete the worktree or uncommitted changes.",
        ),
        "task.delete_blocked" => tr(
            lang,
            "请先切换到其他任务，并等待当前运行或审批结束。",
            "Switch tasks first and wait for the current run or approval to finish.",
        ),
        "model.none" => tr(
            lang,
            "暂无可用模型。可在 config.toml 的 [models] 里配置。",
            "No models available. Configure [models] in config.toml.",
        ),
        "model.edit_config" => tr(lang, "编辑 config.toml", "Edit config.toml"),

        // —— About ——
        "about.title" => tr(lang, "关于 Bony Build", "About Bony Build"),
        "about.tagline" => tr(
            lang,
            "原生桌面 AI 编程助手 · 含 Unity CLI 本地控制",
            "Native desktop AI coding assistant · local Unity CLI",
        ),
        "about.version" => tr(lang, "版本", "Version"),
        "about.body" => tr(
            lang,
            "通过 ACP 驱动本地 grok agent，在仓库工作区里对话、改代码、跑工具。",
            "Drives a local grok agent over ACP to chat, edit, and run tools in your repo.",
        ),
        "about.unity" => tr(lang, "Unity 控制（近期）", "Unity control (recent)"),
        "about.other" => tr(lang, "其它能力", "Other capabilities"),
        "about.footer" => tr(
            lang,
            "底层复用 SpaceXAI Grok agent；桌面壳为本产品。",
            "Agent runtime aligns with xAI Grok; the desktop shell is Bony Build.",
        ),
        "about.u1" => tr(
            lang,
            "• 侧栏「插件 → Unity 控制」：安装 CLI / Pipeline、绑定工程、按钮操作",
            "• Sidebar Plugins → Unity: install CLI/Pipeline, bind project, actions",
        ),
        "about.u2" => tr(
            lang,
            "• 聊天旁 Unity 芯片或 /unity：探测编辑器、Play、跑循环等",
            "• Unity chip or /unity in chat: probe editor, Play, loops, etc.",
        ),
        "about.u3" => tr(
            lang,
            "• 走本机 Unity CLI，不经 Agent，避免 pipeline 安装卡死",
            "• Uses local Unity CLI (not Agent) to avoid hung pipeline installs",
        ),
        "about.o1" => tr(
            lang,
            "• 多供应商 BYOK（Qwen / Kimi / 智谱 / OpenAI 兼容等）",
            "• Multi-provider BYOK (Qwen / Kimi / Zhipu / OpenAI-compatible)",
        ),
        "about.o2" => tr(
            lang,
            "• 项目与任务：可选隔离 Git worktree，按任务控制权限",
            "• Projects & tasks: optional Git worktrees, per-task permissions",
        ),
        "about.o3" => tr(
            lang,
            "• 工具卡片、模型切换、使用量统计与多语言界面",
            "• Tool cards, model switching, usage stats, and multilingual UI",
        ),

        other => other,
    }
}
