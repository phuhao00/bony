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
        "task.pick_project_title" => tr(lang, "选择要新建任务的项目", "Choose a project for the new task"),
        "task.pick_project_hint" => tr(
            lang,
            "有多个项目时请先选择，不会默认落到当前启动目录。",
            "Pick a project first when you have more than one — launch folder is not the default.",
        ),
        "task.pick_other_project" => tr(lang, "打开其他项目…", "Open another project…"),
        "sidebar.filter_tasks" => tr(lang, "筛选任务…", "Filter tasks…"),
        "sidebar.by_project" => tr(lang, "项目", "Projects"),
        "sidebar.no_chats" => tr(lang, "暂无对话", "No conversations"),
        "sidebar.no_history" => tr(lang, "还没有对话记录", "No conversations yet"),
        "sidebar.no_match" => tr(lang, "没有匹配的对话", "No matching conversations"),
        "sidebar.switch_project" => tr(lang, "切换到此项目", "Switch to project"),
        "sidebar.remove_from_list" => tr(lang, "从列表移除", "Remove from list"),
        "sidebar.search" => tr(lang, "搜索任务", "Search tasks"),

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

        // —— Plus menu ——
        "plus.add_file" => tr(lang, "添加文件", "Add files"),
        "plus.add_file_sub" => tr(lang, "加入当前对话上下文", "Add to conversation context"),
        "plus.section_plugins" => tr(lang, "插件", "Plugins"),
        "plus.unity" => tr(lang, "Unity 控制", "Unity control"),
        "plus.unity_on" => tr(lang, "已启用 · 再点关闭", "On · click to turn off"),
        "plus.unity_off" => tr(lang, "本地 CLI，不经 Agent", "Local CLI · not via Agent"),
        "plus.unity_disabled" => tr(
            lang,
            "未启用 · 去插件页开启",
            "Disabled · enable in Plugins",
        ),
        "plus.manage" => tr(lang, "管理插件", "Manage plugins"),
        "plus.manage_sub" => tr(lang, "安装、启用或关闭", "Install, enable, or disable"),

        // —— User menu / settings ——
        "user.usage" => tr(lang, "使用统计", "Usage"),
        "user.edit_config" => tr(lang, "编辑 config.toml", "Edit config.toml"),
        "user.language" => tr(lang, "语言", "Language"),
        "user.login" => tr(lang, "登录", "Sign in"),
        "user.relogin" => tr(lang, "重新登录", "Sign in again"),
        "user.open_failed" => tr(lang, "无法打开配置", "Could not open config"),
        "user.local_account" => tr(lang, "本机账号", "Local account"),
        "user.signed_out" => tr(lang, "未登录", "Not signed in"),
        "user.settings_section" => tr(lang, "设置", "Settings"),

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
            "在此安装扩展；对话里点输入框「+」即可使用或取消。",
            "Install extensions here. Use + in chat to enable or remove them.",
        ),
        "plugins.unity_title" => tr(lang, "Unity 控制", "Unity control"),
        "plugins.unity_blurb" => tr(
            lang,
            "本地 CLI 驱动编辑器：安装 Pipeline、探测、场景操作",
            "Drive the editor with local CLI: Pipeline, probe, scene actions",
        ),
        "plugins.openmontage_title" => tr(lang, "OpenMontage", "OpenMontage"),
        "plugins.openmontage_blurb" => tr(
            lang,
            "把 OpenMontage 的 12 条视频流水线接入当前对话（不切换工作区）",
            "Add OpenMontage's 12 video pipelines to this chat (no workspace switch)",
        ),
        "plugins.openmontage_path" => tr(lang, "安装目录", "Install path"),
        "plugins.openmontage_change" => tr(lang, "更改…", "Change…"),
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
            "启用后，对话里提到做视频时会自动使用 OpenMontage 流水线。",
            "When enabled, video requests in chat use OpenMontage pipelines.",
        ),
        "plugins.openmontage_status_ready" => tr(lang, "就绪", "Ready"),
        "plugins.openmontage_status_missing" => tr(lang, "未安装", "Not installed"),
        "plugins.openmontage_status_installing" => tr(lang, "安装中…", "Installing…"),
        "plugins.openmontage_status_failed" => tr(lang, "安装失败", "Install failed"),
        "plugins.openmontage_status_deps" => tr(lang, "缺依赖", "Missing deps"),
        "plugins.enable" => tr(lang, "启用", "Enable"),
        "plugins.status" => tr(lang, "状态", "Status"),
        "plugins.chat_hint" => tr(
            lang,
            "· 聊天输入旁可开 Unity 模式",
            "· Enable Unity mode from chat +",
        ),
        "plugins.open_settings" => tr(lang, "打开设置", "Open settings"),
        "plugins.docs" => tr(lang, "说明文档", "Docs"),
        "plugins.docs_tip" => tr(
            lang,
            "查看 /unity 指令与常用说法",
            "View /unity commands and phrases",
        ),
        "plugins.use_in_chat" => tr(lang, "在聊天中使用", "Use in chat"),
        "plugins.enabled_hint" => tr(
            lang,
            "启用后可在聊天里用本地 CLI 控制 Unity，不经 Agent。",
            "When enabled, chat can drive Unity via local CLI (not Agent).",
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
