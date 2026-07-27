//! Bevy (Rust ECS game engine) integration — code-first, unlike Unity.
//!
//! Bevy games are plain Rust, so there is no external editor/CLI to bridge to:
//! the agent edits ECS source directly inside a user-chosen project and this
//! module just drives `cargo check` / `cargo build` / `cargo run` as the
//! observe → act → verify loop, streaming output the same way `unity.rs`
//! streams CLI output and `openmontage.rs` streams its install script.
//!
//! The engine dependency points at a fork (tracked at `main`, not pinned) so
//! that engine-level patches can land there later:
//! `https://github.com/phuhao000/bevy.git`.

use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc};
use std::thread;
use std::time::Duration;

use crate::config_io::grok_config_path;
use crate::process;
use crate::usage::{usage_dir, PluginPrefs};

pub const ENGINE_GIT_URL: &str = "https://github.com/phuhao000/bevy.git";
pub const ENGINE_GIT_BRANCH: &str = "main";
const LOG_TAIL_MAX: usize = 200;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BevyStatus {
    Unknown,
    NoRust,
    NoProject,
    Ready,
    Error,
}

impl BevyStatus {
    pub fn is_ready(self) -> bool {
        matches!(self, Self::Ready)
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Unknown => "检测中",
            Self::NoRust => "未安装 Rust",
            Self::NoProject => "未选择 Bevy 项目",
            Self::Ready => "已就绪",
            Self::Error => "异常",
        }
    }
}

enum BevyMsg {
    Line(String),
    ScaffoldDone { path: PathBuf },
    ScaffoldFailed(String),
    JobDone { ok: bool, message: String },
    RustInstallDone { ok: bool, message: String },
}

pub struct BevyState {
    pub status: BevyStatus,
    pub project_path: PathBuf,
    pub busy: bool,
    /// True while a `cargo run` child (the game window) is alive.
    pub running: bool,
    pub log_tail: Vec<String>,
    pub last_error: Option<String>,
    pub toast: Option<String>,
    checked: bool,
    job_seq: u64,
    cancel_flag: Option<Arc<AtomicBool>>,
    pending_rx: Option<mpsc::Receiver<(u64, BevyMsg)>>,
}

impl Default for BevyState {
    fn default() -> Self {
        Self {
            status: BevyStatus::Unknown,
            project_path: default_project_dir(),
            busy: false,
            running: false,
            log_tail: Vec::new(),
            last_error: None,
            toast: None,
            checked: false,
            job_seq: 0,
            cancel_flag: None,
            pending_rx: None,
        }
    }
}

impl BevyState {
    pub fn from_prefs(prefs: &PluginPrefs) -> Self {
        let mut state = Self::default();
        if let Some(root) = &prefs.bevy_project_root {
            state.project_path = root.clone();
        }
        state.refresh_status();
        state
    }

    pub fn ensure_checked(&mut self) {
        if !self.checked && !self.busy {
            self.refresh_status();
        }
    }

    pub fn refresh_status(&mut self) {
        if self.busy {
            return;
        }
        self.status = check_status(&self.project_path);
        self.checked = true;
    }

    pub fn set_project_path(&mut self, path: PathBuf) {
        self.project_path = path;
        self.refresh_status();
    }

    pub fn take_toast(&mut self) -> Option<String> {
        self.toast.take()
    }

    pub fn can_stop(&self) -> bool {
        self.busy && self.cancel_flag.is_some()
    }

    pub fn stop(&mut self) {
        if let Some(flag) = &self.cancel_flag {
            flag.store(true, Ordering::SeqCst);
            self.toast = Some("正在停止…".into());
        }
    }

    /// Spawn a background job tagged with a fresh id, mirroring unity.rs's
    /// `spawn_job`: stale replies from a superseded job are dropped, and any
    /// prior in-flight job is asked to cancel.
    fn spawn_job<F>(&mut self, work: F) -> Arc<AtomicBool>
    where
        F: FnOnce(u64, mpsc::Sender<(u64, BevyMsg)>, Arc<AtomicBool>) + Send + 'static,
    {
        if let Some(prev) = self.cancel_flag.take() {
            prev.store(true, Ordering::SeqCst);
        }
        self.job_seq += 1;
        let id = self.job_seq;
        let cancel = Arc::new(AtomicBool::new(false));
        let (tx, rx) = mpsc::channel();
        self.pending_rx = Some(rx);
        self.cancel_flag = Some(cancel.clone());
        let cancel_for_thread = cancel.clone();
        thread::spawn(move || work(id, tx, cancel_for_thread));
        cancel
    }

    pub fn poll(&mut self) -> bool {
        let Some(rx) = self.pending_rx.as_ref() else {
            return false;
        };
        let mut msgs = Vec::new();
        while let Ok((id, msg)) = rx.try_recv() {
            if id == self.job_seq {
                msgs.push(msg);
            }
        }
        if msgs.is_empty() {
            return false;
        }
        for msg in msgs {
            match msg {
                BevyMsg::Line(line) => push_log(&mut self.log_tail, line),
                BevyMsg::ScaffoldDone { path } => {
                    self.busy = false;
                    self.project_path = path;
                    self.status = check_status(&self.project_path);
                    self.toast = Some("Bevy 项目已创建".into());
                }
                BevyMsg::ScaffoldFailed(reason) => {
                    self.busy = false;
                    self.last_error = Some(reason.clone());
                    push_log(&mut self.log_tail, reason);
                    self.toast = Some("创建 Bevy 项目失败".into());
                }
                BevyMsg::JobDone { ok, message } => {
                    self.busy = false;
                    self.running = false;
                    if !message.is_empty() {
                        push_log(&mut self.log_tail, message.clone());
                    }
                    if ok {
                        self.last_error = None;
                        self.toast = Some("完成".into());
                    } else {
                        self.last_error = Some(message);
                        self.toast = Some("执行失败（详见日志）".into());
                    }
                    self.status = check_status(&self.project_path);
                }
                BevyMsg::RustInstallDone { ok, message } => {
                    self.busy = false;
                    push_log(&mut self.log_tail, message.clone());
                    if ok {
                        self.toast = Some("Rust 安装完成，正在重新检测…".into());
                        self.status = BevyStatus::Unknown;
                        self.checked = false;
                        self.ensure_checked();
                    } else {
                        self.status = BevyStatus::NoRust;
                        self.last_error = Some(message);
                        self.toast = Some("Rust 安装失败".into());
                    }
                }
            }
        }
        if !self.busy {
            self.pending_rx = None;
        }
        true
    }

    /// Scaffold a new Bevy project at `parent/name`, wiring the engine fork
    /// as a git dependency and writing a deliberately minimal `main.rs`
    /// (just opens a window) so the very first `cargo run` proves the whole
    /// toolchain → git fetch → compile → run pipeline works end to end.
    pub fn create_project(&mut self, parent: PathBuf, name: String) {
        if self.busy {
            return;
        }
        self.busy = true;
        self.log_tail.clear();
        self.last_error = None;
        self.toast = Some("正在创建 Bevy 项目…".into());
        self.spawn_job(move |id, tx, _cancel| {
            let result = scaffold_project(&parent, &name, id, &tx);
            let msg = match result {
                Ok(path) => BevyMsg::ScaffoldDone { path },
                Err(reason) => BevyMsg::ScaffoldFailed(reason),
            };
            let _ = tx.send((id, msg));
        });
    }

    pub fn check(&mut self) {
        self.run_cargo(&["check"], "cargo check");
    }

    pub fn build(&mut self) {
        self.run_cargo(&["build"], "cargo build");
    }

    /// `cargo run` launches the game window as a long-lived child; Stop
    /// cooperatively kills it via the same cancel flag used elsewhere.
    pub fn run(&mut self) {
        self.running = true;
        self.run_cargo(&["run"], "cargo run");
    }

    fn run_cargo(&mut self, args: &'static [&'static str], label: &'static str) {
        if self.busy || !matches!(self.status, BevyStatus::Ready) {
            return;
        }
        self.busy = true;
        self.toast = Some(format!("正在执行 {label}…"));
        let project = self.project_path.clone();
        self.spawn_job(move |id, tx, cancel| {
            let mut cmd = process::command("cargo");
            cmd.args(args).current_dir(&project);
            let result = run_streaming(&mut cmd, id, &tx, &cancel);
            let msg = match result {
                Ok(()) => BevyMsg::JobDone {
                    ok: true,
                    message: format!("{label} 完成"),
                },
                Err(reason) => BevyMsg::JobDone { ok: false, message: reason },
            };
            let _ = tx.send((id, msg));
        });
    }

    pub fn can_install_rust(&self) -> bool {
        !self.busy && matches!(self.status, BevyStatus::NoRust)
    }

    /// One-click Rust toolchain install (mirrors Unity's `install_cli`):
    /// Windows tries `winget`, falling back to the rustup-init download;
    /// Unix runs the official rustup.rs shell one-liner.
    pub fn install_rust(&mut self) {
        if self.busy {
            return;
        }
        self.busy = true;
        self.log_tail.clear();
        self.toast = Some("正在安装 Rust…".into());
        self.spawn_job(move |id, tx, cancel| {
            let result = install_rust_toolchain(id, &tx, &cancel);
            let msg = match result {
                Ok(()) => BevyMsg::RustInstallDone {
                    ok: true,
                    message: "Rust 安装完成".into(),
                },
                Err(reason) => BevyMsg::RustInstallDone { ok: false, message: reason },
            };
            let _ = tx.send((id, msg));
        });
    }

    pub fn rust_install_hint(&self) -> &'static str {
        if cfg!(windows) {
            "winget install --id Rustlang.Rustup -e"
        } else {
            "curl https://sh.rustup.rs -sSf | sh"
        }
    }
}

fn default_project_dir() -> PathBuf {
    dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("BonyBevyGames")
        .join("my-game")
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

pub fn check_status(project: &Path) -> BevyStatus {
    if !tool_ok("cargo", &["--version"]) || !tool_ok("rustc", &["--version"]) {
        return BevyStatus::NoRust;
    }
    let cargo_toml = project.join("Cargo.toml");
    if !cargo_toml.is_file() {
        return BevyStatus::NoProject;
    }
    match std::fs::read_to_string(&cargo_toml) {
        Ok(text) if text.contains("bevy") => BevyStatus::Ready,
        Ok(_) => BevyStatus::NoProject,
        Err(_) => BevyStatus::Error,
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

fn main_rs_template() -> &'static str {
    r#"use bevy::prelude::*;

// Deliberately minimal: this fork tracks Bevy's `main` branch, whose APIs
// move fast. Confirm current API shapes (see the SKILL.md notes on checking
// the fetched git checkout / `cargo doc`) before building on top of this.
fn main() {
    App::new()
        .add_plugins(DefaultPlugins)
        .add_systems(Startup, setup)
        .run();
}

fn setup(mut commands: Commands) {
    commands.spawn(Camera2d);
}
"#
}

fn cargo_toml_template(name: &str) -> String {
    format!(
        r#"[package]
name = "{name}"
version = "0.1.0"
edition = "2021"

[dependencies]
bevy = {{ git = "{ENGINE_GIT_URL}", branch = "{ENGINE_GIT_BRANCH}" }}

# Bevy's recommended dev-profile tweak: keep our own crate unoptimized for
# fast iterative compiles, but optimize dependencies (this matters a lot for
# a large engine dependency graph).
[profile.dev]
opt-level = 1

[profile.dev.package."*"]
opt-level = 3
"#
    )
}

fn scaffold_project(
    parent: &Path,
    name: &str,
    id: u64,
    tx: &mpsc::Sender<(u64, BevyMsg)>,
) -> Result<PathBuf, String> {
    let send_line = |line: String| {
        let _ = tx.send((id, BevyMsg::Line(line)));
    };
    std::fs::create_dir_all(parent).map_err(|e| format!("无法创建目录：{e}"))?;
    let target = parent.join(name);
    if target.exists() {
        return Err(format!("目标目录已存在：{}", target.display()));
    }
    send_line(format!("cargo new {}", target.display()));
    let status = process::command("cargo")
        .args(["new", "--bin", "--name", name])
        .arg(&target)
        .current_dir(parent)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map_err(|e| format!("无法启动 cargo new：{e}"))?;
    if !status.success() {
        return Err(format!("cargo new 退出码 {}", status.code().unwrap_or(-1)));
    }
    send_line("写入 Cargo.toml（bevy 引擎依赖：fork main 分支）".into());
    std::fs::write(target.join("Cargo.toml"), cargo_toml_template(name))
        .map_err(|e| format!("写入 Cargo.toml 失败：{e}"))?;
    send_line("写入最小可运行 main.rs".into());
    std::fs::write(target.join("src").join("main.rs"), main_rs_template())
        .map_err(|e| format!("写入 main.rs 失败：{e}"))?;
    send_line("完成".into());
    Ok(target)
}

/// Run `cmd`, streaming stdout/stderr lines as `BevyMsg::Line`, and killing
/// the child early if `cancel` flips true (used both for bounded jobs like
/// `cargo build` and unbounded ones like `cargo run`'s game window).
fn run_streaming(
    cmd: &mut Command,
    id: u64,
    tx: &mpsc::Sender<(u64, BevyMsg)>,
    cancel: &Arc<AtomicBool>,
) -> Result<(), String> {
    let mut child = cmd
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("无法启动：{e}"))?;

    let stdout = child.stdout.take();
    let stderr = child.stderr.take();
    let tx_out = tx.clone();
    let out_handle = thread::spawn(move || {
        if let Some(out) = stdout {
            for line in BufReader::new(out).lines().flatten() {
                let _ = tx_out.send((id, BevyMsg::Line(line)));
            }
        }
    });
    let tx_err = tx.clone();
    let err_handle = thread::spawn(move || {
        let mut last = String::new();
        if let Some(err) = stderr {
            for line in BufReader::new(err).lines().flatten() {
                last = line.clone();
                let _ = tx_err.send((id, BevyMsg::Line(line)));
            }
        }
        last
    });

    let status = loop {
        if cancel.load(Ordering::Relaxed) {
            let _ = child.kill();
            let _ = child.wait();
            let _ = out_handle.join();
            let _ = err_handle.join();
            return Err("已停止".into());
        }
        match child.try_wait() {
            Ok(Some(s)) => break s,
            Ok(None) => thread::sleep(Duration::from_millis(80)),
            Err(e) => return Err(format!("等待进程失败：{e}")),
        }
    };
    let _ = out_handle.join();
    let err_tail = err_handle.join().unwrap_or_default();
    if status.success() {
        Ok(())
    } else {
        Err(format!(
            "退出码 {}{}",
            status.code().unwrap_or(-1),
            if err_tail.is_empty() {
                String::new()
            } else {
                format!("：{err_tail}")
            }
        ))
    }
}

fn install_rust_toolchain(
    id: u64,
    tx: &mpsc::Sender<(u64, BevyMsg)>,
    cancel: &Arc<AtomicBool>,
) -> Result<(), String> {
    if cfg!(windows) && tool_ok("winget", &["--version"]) {
        let mut cmd = process::command("winget");
        cmd.args([
            "install",
            "--id",
            "Rustlang.Rustup",
            "-e",
            "--silent",
            "--accept-package-agreements",
            "--accept-source-agreements",
        ]);
        return run_streaming(&mut cmd, id, tx, cancel);
    }
    if !cfg!(windows) {
        let mut cmd = process::command("bash");
        cmd.args(["-c", "curl https://sh.rustup.rs -sSf | sh -s -- -y"]);
        return run_streaming(&mut cmd, id, tx, cancel);
    }
    Err("未检测到 winget，请手动安装 Rust：https://rustup.rs".into())
}

// ── Skill file + config.toml patch (same mechanism as openmontage.rs) ──────

pub fn skill_path() -> PathBuf {
    usage_dir().join("skills").join("bevy-game-dev").join("SKILL.md")
}

pub fn skill_md(project: &Path) -> String {
    let project_disp = project.display().to_string().replace('\\', "/");
    format!(
        r#"---
name: bevy-game-dev
description: Use when the user asks to create, extend, or debug a Bevy (Rust ECS) game — 2D/3D scenes, gameplay systems, physics, input, or anything driven by the `bevy` crate. Do NOT use for Unity or OpenMontage tasks.
---

# Bevy Game Engine (via Bony Build)

Active game project: `{project_disp}`

This project depends on a Bevy engine fork tracked at the `main` branch (not a pinned release): `{ENGINE_GIT_URL}`. It is not pinned so that engine-level patches can land there later — if the user asks you to modify engine internals rather than game code, that means editing the fork itself (clone it separately, switch this project's dependency to a local `path` override while iterating).

Because `main` moves fast, published Bevy docs/tutorials can be stale or wrong for this exact checkout. Before writing non-trivial ECS code:
1. Check the actual fetched source for current API shapes — look under `~/.cargo/git/checkouts/bevy-*/` (Windows: `%USERPROFILE%\.cargo\git\checkouts\bevy-*\`), or run `cargo doc -p bevy --open` from the project root.
2. Prefer `cargo check` after each edit — much faster than `cargo run` and catches API drift immediately.

Workflow:
1. Edit ECS code directly (Components, Systems, Resources, Plugins, Schedules) under `{project_disp}/src/`.
2. Run `cargo check` to validate quickly; run `cargo run` to see the actual game window.
3. The first build after a fresh clone or dependency bump can take several minutes (Bevy's dependency graph is large) — say so up front and avoid re-running `cargo run` while a build is already in flight.
4. To stop a running game window, use Bony Build's Bevy panel Stop button (kills the `cargo run` child) rather than leaving orphaned processes.
"#
    )
}

pub fn write_skill_file(project: &Path) -> Result<PathBuf, String> {
    let path = skill_path();
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| format!("创建 skill 目录失败：{e}"))?;
    }
    std::fs::write(&path, skill_md(project)).map_err(|e| format!("写入 SKILL.md 失败：{e}"))?;
    Ok(path)
}

/// Add or remove the Bevy skill path in `~/.grok/config.toml` `[skills].paths`.
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

    let mut keep = toml_edit::Array::new();
    for item in arr.iter() {
        let Some(s) = item.as_str() else {
            keep.push(item.clone());
            continue;
        };
        let norm = s.replace('\\', "/");
        let is_ours = norm == skill_alt
            || Path::new(s) == skill
            || norm.ends_with("/bevy-game-dev/SKILL.md")
            || norm.ends_with("/bevy-game-dev");
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

pub fn enable_skill(prefs: &mut PluginPrefs, project: &Path) -> Result<(), String> {
    let skill = write_skill_file(project)?;
    sync_config_skill_path(true, &skill)?;
    prefs.bevy_enabled = true;
    prefs.bevy_project_root = Some(project.to_path_buf());
    Ok(())
}

pub fn disable_skill(prefs: &mut PluginPrefs) -> Result<(), String> {
    sync_config_skill_path(false, &skill_path())?;
    prefs.bevy_enabled = false;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn check_status_no_project() {
        let tmp = tempdir().unwrap();
        // Force past the Rust-toolchain gate deterministically isn't possible
        // here without mocking; just assert the no-Cargo.toml path when Rust
        // happens to be present (dev/test machines always have cargo).
        if tool_ok("cargo", &["--version"]) && tool_ok("rustc", &["--version"]) {
            assert_eq!(check_status(tmp.path()), BevyStatus::NoProject);
        }
    }

    #[test]
    fn check_status_ready_when_bevy_dep_present() {
        let tmp = tempdir().unwrap();
        std::fs::write(
            tmp.path().join("Cargo.toml"),
            "[package]\nname = \"x\"\n[dependencies]\nbevy = \"0.14\"\n",
        )
        .unwrap();
        if tool_ok("cargo", &["--version"]) && tool_ok("rustc", &["--version"]) {
            assert_eq!(check_status(tmp.path()), BevyStatus::Ready);
        }
    }

    #[test]
    fn skill_md_contains_project_and_engine_url() {
        let md = skill_md(Path::new("C:/games/my-game"));
        assert!(md.contains("bevy-game-dev"));
        assert!(md.contains("C:/games/my-game"));
        assert!(md.contains(ENGINE_GIT_URL));
        assert!(md.contains("main"));
    }

    #[test]
    fn cargo_toml_template_pins_git_branch_not_rev() {
        let toml = cargo_toml_template("my-game");
        assert!(toml.contains(ENGINE_GIT_URL));
        assert!(toml.contains("branch = \"main\""));
        assert!(!toml.contains("rev ="));
    }
}
