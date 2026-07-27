//! OpenMontage video-production skill integration.
//!
//! Keeps the agent session cwd unchanged. When enabled, patches
//! `~/.grok/config.toml` `[skills].paths` to a generated SKILL.md that tells
//! the model to run OpenMontage tools under `OPENMONTAGE_ROOT`.

use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::thread;
use std::time::Duration;

use crate::config_io::grok_config_path;
use crate::process;
use crate::usage::{PluginPrefs, usage_dir};

pub const GITHUB_URL: &str = "https://github.com/calesthio/OpenMontage.git";
const LOG_TAIL_MAX: usize = 40;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OpenMontageStatus {
    Unknown,
    NotInstalled,
    Installing,
    InstallFailed(String),
    MissingDeps(Vec<&'static str>),
    Ready,
}

impl OpenMontageStatus {
    pub fn is_ready(&self) -> bool {
        matches!(self, Self::Ready)
    }
}

enum InstallMsg {
    Progress { step: String, line: Option<String> },
    Done { root: PathBuf },
    Failed { reason: String },
}

pub struct OpenMontageState {
    pub status: OpenMontageStatus,
    pub root: PathBuf,
    pub busy: bool,
    pub log_tail: Vec<String>,
    pub last_step: String,
    pub toast: Option<String>,
    /// Cached provider_menu_summary JSON (or error text) for prompt injection.
    pub preflight_summary: Option<String>,
    checked: bool,
    pending_rx: Option<mpsc::Receiver<InstallMsg>>,
    preflight_rx: Option<mpsc::Receiver<Result<String, String>>>,
}

impl Default for OpenMontageState {
    fn default() -> Self {
        Self {
            status: OpenMontageStatus::Unknown,
            root: default_install_dir(),
            busy: false,
            log_tail: Vec::new(),
            last_step: String::new(),
            toast: None,
            preflight_summary: None,
            checked: false,
            pending_rx: None,
            preflight_rx: None,
        }
    }
}

impl OpenMontageState {
    pub fn from_prefs(prefs: &PluginPrefs) -> Self {
        let mut state = Self::default();
        if let Some(root) = &prefs.openmontage_root {
            state.root = root.clone();
        }
        state.refresh_status();
        state
    }

    pub fn refresh_status(&mut self) {
        if self.busy {
            return;
        }
        self.status = check_status(&self.root);
        self.checked = true;
        if self.status.is_ready() {
            self.start_preflight_refresh();
        } else {
            self.preflight_summary = None;
        }
    }

    pub fn ensure_checked(&mut self) {
        if !self.checked && !self.busy {
            self.refresh_status();
        }
    }

    pub fn take_toast(&mut self) -> Option<String> {
        self.toast.take()
    }

    /// Background provider-menu preflight (safe argv — no nested python -c quotes).
    pub fn start_preflight_refresh(&mut self) {
        if !self.status.is_ready() || self.preflight_rx.is_some() {
            return;
        }
        let root = self.root.clone();
        let (tx, rx) = mpsc::channel();
        self.preflight_rx = Some(rx);
        thread::spawn(move || {
            let _ = tx.send(run_preflight_summary(&root));
        });
    }

    pub fn poll(&mut self) -> bool {
        let mut changed = false;
        if let Some(rx) = self.preflight_rx.as_ref() {
            if let Ok(result) = rx.try_recv() {
                self.preflight_rx = None;
                self.preflight_summary = Some(match result {
                    Ok(summary) => summary,
                    Err(err) => format!("preflight failed: {err}"),
                });
                changed = true;
            }
        }

        let Some(rx) = self.pending_rx.as_ref() else {
            return changed;
        };
        let mut msgs = Vec::new();
        while let Ok(msg) = rx.try_recv() {
            msgs.push(msg);
        }
        if msgs.is_empty() {
            return changed;
        }
        for msg in msgs {
            match msg {
                InstallMsg::Progress { step, line } => {
                    self.last_step = step;
                    if let Some(line) = line {
                        push_log(&mut self.log_tail, line);
                    }
                }
                InstallMsg::Done { root } => {
                    self.busy = false;
                    self.root = root;
                    self.status = check_status(&self.root);
                    self.pending_rx = None;
                    self.toast = Some("OpenMontage 安装完成".into());
                    if self.status.is_ready() {
                        self.start_preflight_refresh();
                    }
                }
                InstallMsg::Failed { reason } => {
                    self.busy = false;
                    self.status = OpenMontageStatus::InstallFailed(reason.clone());
                    push_log(&mut self.log_tail, reason);
                    self.pending_rx = None;
                    self.toast = Some("OpenMontage 安装失败".into());
                }
            }
        }
        true
    }

    /// Full install: clone + deps. `deps_only` skips clone when repo already exists.
    pub fn start_install(&mut self, deps_only: bool) {
        if self.busy {
            return;
        }
        let target = self.root.clone();
        if let Err(missing) = check_prerequisites() {
            self.status = OpenMontageStatus::InstallFailed(missing);
            return;
        }
        if deps_only {
            if !target.join("AGENT_GUIDE.md").is_file() {
                self.status = OpenMontageStatus::InstallFailed(
                    "目录里没有 AGENT_GUIDE.md，无法只装依赖。请先完整安装。".into(),
                );
                return;
            }
        } else if target.exists() {
            let empty = std::fs::read_dir(&target)
                .map(|mut d| d.next().is_none())
                .unwrap_or(false);
            if !empty && !target.join("AGENT_GUIDE.md").is_file() {
                self.status = OpenMontageStatus::InstallFailed(format!(
                    "目标目录非空且不是 OpenMontage：{}",
                    target.display()
                ));
                return;
            }
        }

        self.busy = true;
        self.status = OpenMontageStatus::Installing;
        self.log_tail.clear();
        self.last_step = if deps_only {
            "准备安装依赖…".into()
        } else {
            "准备安装…".into()
        };
        let (tx, rx) = mpsc::channel();
        self.pending_rx = Some(rx);
        thread::spawn(move || {
            let result = run_install(target, deps_only, &tx);
            match result {
                Ok(root) => {
                    let _ = tx.send(InstallMsg::Done { root });
                }
                Err(reason) => {
                    let _ = tx.send(InstallMsg::Failed { reason });
                }
            }
        });
    }

    pub fn open_backlot(&mut self) {
        if !self.status.is_ready() {
            self.toast = Some("OpenMontage 尚未就绪".into());
            return;
        }
        let root = self.root.clone();
        let py = venv_python(&root);
        thread::spawn(move || {
            let mut cmd = process::command(&py);
            cmd.arg("-m")
                .arg("backlot")
                .arg("open")
                .current_dir(&root)
                .stdout(Stdio::null())
                .stderr(Stdio::null());
            let _ = cmd.spawn();
        });
        self.toast = Some("正在打开 Backlot…".into());
    }
}

pub fn default_install_dir() -> PathBuf {
    dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("OpenMontage")
}

pub fn check_status(root: &Path) -> OpenMontageStatus {
    if !root.is_dir() || !root.join("AGENT_GUIDE.md").is_file() {
        return OpenMontageStatus::NotInstalled;
    }
    let mut missing = Vec::new();
    if !venv_python(root).is_file() {
        missing.push(".venv");
    }
    if !root.join(".env").is_file() {
        missing.push(".env");
    }
    if !root.join("remotion-composer").join("node_modules").is_dir() {
        missing.push("node_modules");
    }
    if missing.is_empty() {
        OpenMontageStatus::Ready
    } else {
        OpenMontageStatus::MissingDeps(missing)
    }
}

fn venv_python(root: &Path) -> PathBuf {
    if cfg!(windows) {
        root.join(".venv").join("Scripts").join("python.exe")
    } else {
        root.join(".venv").join("bin").join("python")
    }
}

fn push_log(tail: &mut Vec<String>, line: String) {
    let line = line.trim_end().to_string();
    if line.is_empty() {
        return;
    }
    tail.push(line);
    if tail.len() > LOG_TAIL_MAX {
        let drain = tail.len() - LOG_TAIL_MAX;
        tail.drain(0..drain);
    }
}

fn check_prerequisites() -> Result<(), String> {
    let mut missing = Vec::new();
    if !tool_ok("git", &["--version"]) {
        missing.push("Git");
    }
    if !python_launcher().is_some_and(|(bin, args)| {
        let mut full = args;
        full.push("--version".into());
        let refs: Vec<&str> = full.iter().map(|s| s.as_str()).collect();
        tool_ok(&bin, &refs)
    }) {
        missing.push("Python 3");
    }
    if !tool_ok("node", &["--version"]) {
        missing.push("Node.js");
    }
    if !tool_ok("npm", &["--version"]) {
        missing.push("npm");
    }
    if missing.is_empty() {
        Ok(())
    } else {
        Err(format!(
            "缺少：{}。请先安装后再试（Git / Python 3 / Node.js）。",
            missing.join("、")
        ))
    }
}

fn tool_ok(program: &str, args: &[&str]) -> bool {
    process::command(program)
        .args(args)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

fn python_launcher() -> Option<(String, Vec<String>)> {
    if cfg!(windows) && tool_ok("py", &["-3", "--version"]) {
        return Some(("py".into(), vec!["-3".into()]));
    }
    if tool_ok("python3", &["--version"]) {
        return Some(("python3".into(), Vec::new()));
    }
    if tool_ok("python", &["--version"]) {
        return Some(("python".into(), Vec::new()));
    }
    None
}

fn run_install(
    target: PathBuf,
    deps_only: bool,
    tx: &mpsc::Sender<InstallMsg>,
) -> Result<PathBuf, String> {
    let progress = |step: &str, line: Option<String>| {
        let _ = tx.send(InstallMsg::Progress {
            step: step.to_string(),
            line,
        });
    };

    if !deps_only {
        if target.join("AGENT_GUIDE.md").is_file() {
            progress("仓库已存在，跳过 clone", None);
        } else {
            progress("git clone OpenMontage…", None);
            if let Some(parent) = target.parent() {
                std::fs::create_dir_all(parent)
                    .map_err(|e| format!("无法创建父目录：{e}"))?;
            }
            run_streaming(
                process::command("git")
                    .args(["clone", "--depth", "1", GITHUB_URL])
                    .arg(&target),
                "git clone",
                tx,
                "git clone OpenMontage…",
            )?;
        }
    }

    let py = venv_python(&target);
    if !py.is_file() {
        progress("创建 Python 虚拟环境…", None);
        let (bin, mut args) = python_launcher().ok_or_else(|| "找不到 Python".to_string())?;
        args.push("-m".into());
        args.push("venv".into());
        args.push(".venv".into());
        let mut cmd = process::command(&bin);
        cmd.args(&args).current_dir(&target);
        run_streaming(&mut cmd, "python -m venv", tx, "创建 Python 虚拟环境…")?;
    } else {
        progress("虚拟环境已存在", None);
    }

    let py = venv_python(&target);
    if !py.is_file() {
        return Err("venv 创建后仍找不到 python".into());
    }

    progress("pip install requirements.txt…", None);
    run_streaming(
        process::command(&py)
            .args(["-m", "pip", "install", "-r", "requirements.txt"])
            .current_dir(&target),
        "pip install",
        tx,
        "pip install requirements.txt…",
    )?;

    progress("pip install piper-tts…", None);
    let _ = run_streaming(
        process::command(&py)
            .args(["-m", "pip", "install", "piper-tts"])
            .current_dir(&target),
        "pip install piper-tts",
        tx,
        "pip install piper-tts…",
    );

    let remotion = target.join("remotion-composer");
    if remotion.is_dir() {
        progress("npm install (remotion-composer)…", None);
        let npm_result = run_streaming(
            process::command("npm")
                .arg("install")
                .current_dir(&remotion),
            "npm install",
            tx,
            "npm install (remotion-composer)…",
        );
        if npm_result.is_err() {
            progress("npm install 失败，尝试 npx npm install…", None);
            run_streaming(
                process::command("npx")
                    .args(["--yes", "npm", "install"])
                    .current_dir(&remotion),
                "npx npm install",
                tx,
                "npx npm install…",
            )?;
        }
    }

    let env_path = target.join(".env");
    let example = target.join(".env.example");
    if !env_path.is_file() && example.is_file() {
        progress("复制 .env.example → .env", None);
        std::fs::copy(&example, &env_path).map_err(|e| format!("复制 .env 失败：{e}"))?;
    }

    progress("完成", None);
    Ok(target)
}

fn run_streaming(
    cmd: &mut Command,
    label: &str,
    tx: &mpsc::Sender<InstallMsg>,
    step: &str,
) -> Result<(), String> {
    let mut child = cmd
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("无法启动 {label}：{e}"))?;

    let stdout = child.stdout.take();
    let stderr = child.stderr.take();
    let tx_out = tx.clone();
    let step_out = step.to_string();
    let out_handle = thread::spawn(move || {
        if let Some(out) = stdout {
            for line in BufReader::new(out).lines().flatten() {
                let _ = tx_out.send(InstallMsg::Progress {
                    step: step_out.clone(),
                    line: Some(line),
                });
            }
        }
    });
    let tx_err = tx.clone();
    let step_err = step.to_string();
    let err_handle = thread::spawn(move || {
        let mut last = String::new();
        if let Some(err) = stderr {
            for line in BufReader::new(err).lines().flatten() {
                last = line.clone();
                let _ = tx_err.send(InstallMsg::Progress {
                    step: step_err.clone(),
                    line: Some(line),
                });
            }
        }
        last
    });

    let status = loop {
        match child.try_wait() {
            Ok(Some(s)) => break s,
            Ok(None) => thread::sleep(Duration::from_millis(50)),
            Err(e) => return Err(format!("{label} wait 失败：{e}")),
        }
    };
    let _ = out_handle.join();
    let err_tail = err_handle.join().unwrap_or_default();
    if status.success() {
        Ok(())
    } else {
        Err(format!(
            "{label} 失败（exit {}）{}",
            status.code().unwrap_or(-1),
            if err_tail.is_empty() {
                String::new()
            } else {
                format!("：{err_tail}")
            }
        ))
    }
}

// ── Skill file + config.toml patch ──────────────────────────────────

pub fn skill_path() -> PathBuf {
    usage_dir().join("skills").join("open-montage").join("SKILL.md")
}

pub fn bony_preflight_script(root: &Path) -> PathBuf {
    root.join("scripts").join("bony_preflight.py")
}

const BONY_PREFLIGHT_PY: &str = r#"# Auto-generated by Bony Build — safe preflight without python -c quoting.
import json
import os
import sys

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
os.chdir(ROOT)
if ROOT not in sys.path:
    sys.path.insert(0, ROOT)

from tools.tool_registry import registry  # noqa: E402

registry.discover()
print(json.dumps(registry.provider_menu_summary(), indent=2))
"#;

pub fn ensure_helper_scripts(root: &Path) -> Result<PathBuf, String> {
    let script = bony_preflight_script(root);
    if let Some(parent) = script.parent() {
        std::fs::create_dir_all(parent).map_err(|e| format!("创建 scripts 失败：{e}"))?;
    }
    std::fs::write(&script, BONY_PREFLIGHT_PY)
        .map_err(|e| format!("写入 bony_preflight.py 失败：{e}"))?;
    Ok(script)
}

/// Run provider_menu_summary via the helper script (cwd = root). Truncates output.
pub fn run_preflight_summary(root: &Path) -> Result<String, String> {
    ensure_helper_scripts(root)?;
    let py = venv_python(root);
    if !py.is_file() {
        return Err(format!("venv python missing: {}", py.display()));
    }
    let script = bony_preflight_script(root);
    let output = Command::new(&py)
        .arg(&script)
        .current_dir(root)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .map_err(|e| format!("spawn preflight failed: {e}"))?;
    if !output.status.success() {
        let err = String::from_utf8_lossy(&output.stderr);
        return Err(format!(
            "preflight exit {}: {}",
            output.status.code().unwrap_or(-1),
            err.chars().take(800).collect::<String>()
        ));
    }
    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    Ok(truncate_om_chars(&stdout, 6_000))
}

fn truncate_om_chars(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        return s.to_string();
    }
    let mut out: String = s.chars().take(max).collect();
    out.push_str("\n…(truncated)");
    out
}

pub fn skill_md(root: &Path) -> String {
    let root_native = root.display().to_string();
    let root_disp = root_native.replace('\\', "/");
    let py = venv_python(root).display().to_string();
    let script = bony_preflight_script(root).display().to_string();
    format!(
        r#"---
name: open-montage
description: REQUIRED for any video, short clip, trailer, animation, montage, 短视频, or film request when this skill is enabled. Drive OpenMontage via local shell/Python — never MCP open-montage__*, never image_gen/image_to_video, never python -c with nested quotes on Windows. Do NOT use for unrelated coding tasks.
---

# OpenMontage (via Bony Build)

OpenMontage engine lives at: `{root_disp}`

## Critical — this is NOT an MCP server

OpenMontage does **not** register MCP tools. There is **no** `open-montage__create_project` or `open-montage__*`.
If those names are missing, that is expected. Do **not** tell the user MCP is misconfigured.

## Critical — Windows shell quoting

Never run `python -c "..."` / nested PowerShell `-Command` with mixed quotes. That causes:
`文件名、目录名或卷标语法不正确。(os error 123)`.

If Bony Build already injected a **PREFLIGHT RESULT**, **skip shell preflight** and continue with pipeline selection.

Otherwise run preflight with **exact argv** (no extra quotes, no `cd &&`):

- program: `{py}`
- args: `["{script}"]`
- working_directory: `{root_native}`

## Hard rules

1. Video / clip / trailer / animation / 短视频 → OpenMontage pipelines only. Never `image_gen` / `image_to_video` / `video_gen`.
2. Never invent placeholder URLs (e.g. `https://example.com/...`).
3. Keep the agent session cwd as the user's project. Only set **working_directory** of tool calls to the OpenMontage root.
4. Create projects with filesystem mkdir: `projects/<kebab-slug>/artifacts`, `assets/...`, `renders`.

## Production workflow (Rule Zero)

1. Read `{root_disp}/AGENT_GUIDE.md` with a file-read tool (not shell-cat).
2. Use PREFLIGHT RESULT if present; else run the exact argv above.
3. Pick a pipeline under `{root_disp}/pipeline_defs/` (e.g. `animation.yaml`, `character-animation.yaml`, `cinematic.yaml`).
4. For EACH stage, read `skills/pipelines/<pipeline>/<stage>-director.md` BEFORE work.
5. Call tools via Python registry APIs from the OpenMontage root (prefer small `.py` helper scripts over `python -c`).
6. Tell the user each major stage (plan → assets → assemble → render).

When finished, copy `{root_disp}/projects/<slug>/renders/` into the user's project at `./outputs/video/<slug>/`.
"#
    )
}

/// Prefixed into the agent prompt (not shown in the chat bubble) when OM is on.
pub fn active_prompt_directive(root: &Path) -> String {
    let root_native = root.display().to_string();
    let py = venv_python(root).display().to_string();
    let script = bony_preflight_script(root).display().to_string();
    format!(
        "[Bony Build · OpenMontage ACTIVE]\n\
         Root: {root_native}\n\
         \n\
         CRITICAL:\n\
         - Local CLI/pipeline repo — NOT MCP. No open-montage__* tools exist.\n\
         - Never python -c with nested quotes on Windows (causes os error 123).\n\
         - Forbidden: image_gen, image_to_video, video_gen, example.com placeholders.\n\
         \n\
         If PREFLIGHT RESULT is included below, do NOT re-run preflight in the shell.\n\
         Otherwise run EXACTLY:\n\
           program = {py}\n\
           args = [{script}]\n\
           working_directory = {root_native}\n\
         \n\
         Then: read AGENT_GUIDE.md → pick pipeline_defs/*.yaml → mkdir projects/<slug>/ → stage directors."
    )
}

pub fn write_skill_file(root: &Path) -> Result<PathBuf, String> {
    let _ = ensure_helper_scripts(root);
    let path = skill_path();
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| format!("创建 skill 目录失败：{e}"))?;
    }
    let body = skill_md(root);
    std::fs::write(&path, body).map_err(|e| format!("写入 SKILL.md 失败：{e}"))?;
    Ok(path)
}

/// Add or remove the OpenMontage skill path in `~/.grok/config.toml` `[skills].paths`.
pub fn sync_config_skill_path(enable: bool, skill: &Path) -> Result<(), String> {
    let config_path = grok_config_path();
    if let Some(parent) = config_path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| format!("创建 .grok 目录失败：{e}"))?;
    }
    let text = if config_path.is_file() {
        std::fs::read_to_string(&config_path).map_err(|e| format!("读取 config.toml 失败：{e}"))?
    } else {
        String::new()
    };

    let mut doc = text
        .parse::<toml_edit::DocumentMut>()
        .map_err(|e| format!("解析 config.toml 失败：{e}"))?;

    let skill_str = skill.display().to_string();
    let skill_alt = skill_str.replace('\\', "/");

    let skills = doc
        .entry("skills")
        .or_insert(toml_edit::Item::Table(toml_edit::Table::new()));
    let table = skills
        .as_table_mut()
        .ok_or_else(|| "[skills] 不是表".to_string())?;

    let paths_item = table
        .entry("paths")
        .or_insert(toml_edit::Item::Value(toml_edit::Value::Array(
            toml_edit::Array::new(),
        )));
    let arr = paths_item
        .as_array_mut()
        .ok_or_else(|| "[skills].paths 不是数组".to_string())?;

    // Remove existing entries that point at this skill (any slash style).
    let mut keep = toml_edit::Array::new();
    for item in arr.iter() {
        let Some(s) = item.as_str() else {
            keep.push(item.clone());
            continue;
        };
        let norm = s.replace('\\', "/");
        let is_ours = norm == skill_alt
            || Path::new(s) == skill
            || norm.ends_with("/open-montage/SKILL.md")
            || norm.ends_with("/open-montage");
        if !is_ours {
            keep.push(s);
        }
    }
    *arr = keep;

    if enable {
        arr.push(skill_str);
    }

    std::fs::write(&config_path, doc.to_string())
        .map_err(|e| format!("写入 config.toml 失败：{e}"))?;
    Ok(())
}

pub fn enable_skill(prefs: &mut PluginPrefs, root: &Path) -> Result<(), String> {
    let skill = write_skill_file(root)?;
    sync_config_skill_path(true, &skill)?;
    prefs.openmontage_enabled = true;
    prefs.openmontage_root = Some(root.to_path_buf());
    Ok(())
}

pub fn disable_skill(prefs: &mut PluginPrefs) -> Result<(), String> {
    sync_config_skill_path(false, &skill_path())?;
    prefs.openmontage_enabled = false;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn check_status_not_installed() {
        let tmp = tempdir().unwrap();
        assert_eq!(
            check_status(tmp.path()),
            OpenMontageStatus::NotInstalled
        );
    }

    #[test]
    fn check_status_missing_deps() {
        let tmp = tempdir().unwrap();
        std::fs::write(tmp.path().join("AGENT_GUIDE.md"), "x").unwrap();
        match check_status(tmp.path()) {
            OpenMontageStatus::MissingDeps(m) => {
                assert!(m.contains(&".venv"));
            }
            other => panic!("unexpected {other:?}"),
        }
    }

    #[test]
    fn skill_md_contains_root() {
        let md = skill_md(Path::new("C:/Users/x/OpenMontage"));
        assert!(md.contains("open-montage"));
        assert!(md.contains("C:/Users/x/OpenMontage"));
        assert!(md.contains("AGENT_GUIDE.md"));
        assert!(md.contains("image_to_video"));
        assert!(md.contains("NOT an MCP"));
        assert!(md.contains("open-montage__create_project"));
        assert!(md.contains("bony_preflight.py"));
        assert!(md.contains("os error 123"));
    }

    #[test]
    fn active_prompt_directive_forbids_builtin_media() {
        let d = active_prompt_directive(Path::new("C:/om"));
        assert!(d.contains("OpenMontage ACTIVE"));
        assert!(d.contains("image_to_video"));
        assert!(d.contains("example.com") || d.contains("NOT MCP"));
        assert!(d.contains("open-montage__"));
        assert!(d.contains("bony_preflight.py"));
    }

    #[test]
    fn bony_preflight_script_path() {
        let p = bony_preflight_script(Path::new("C:/om"));
        assert_eq!(
            p.file_name().and_then(|s| s.to_str()),
            Some("bony_preflight.py")
        );
    }

    #[test]
    fn sync_config_roundtrip() {
        let tmp = tempdir().unwrap();
        let skill = tmp.path().join("skills").join("open-montage").join("SKILL.md");
        std::fs::create_dir_all(skill.parent().unwrap()).unwrap();
        std::fs::write(&skill, "---\nname: open-montage\n---\n").unwrap();

        // Point grok config at a temp file by writing via absolute path helper —
        // we test the document mutation directly instead.
        let mut doc = toml_edit::DocumentMut::new();
        doc["models"] = toml_edit::Item::Table(toml_edit::Table::new());
        doc["models"]["default"] = toml_edit::value("qwen-max");
        let config = tmp.path().join("config.toml");
        std::fs::write(&config, doc.to_string()).unwrap();

        let text = std::fs::read_to_string(&config).unwrap();
        let mut doc = text.parse::<toml_edit::DocumentMut>().unwrap();
        let skills = doc
            .entry("skills")
            .or_insert(toml_edit::Item::Table(toml_edit::Table::new()));
        let table = skills.as_table_mut().unwrap();
        let paths = table
            .entry("paths")
            .or_insert(toml_edit::Item::Value(toml_edit::Value::Array(
                toml_edit::Array::new(),
            )));
        paths.as_array_mut().unwrap().push(skill.display().to_string());
        std::fs::write(&config, doc.to_string()).unwrap();

        let after = std::fs::read_to_string(&config).unwrap();
        assert!(after.contains("skills"));
        assert!(after.contains("open-montage"));
        assert!(after.contains("qwen-max"));
    }
}
