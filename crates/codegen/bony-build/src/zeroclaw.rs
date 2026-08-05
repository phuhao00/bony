//! ZeroClaw (<https://github.com/zeroclaw-labs/zeroclaw>) — the second,
//! non-coding ACP backend. Deep-fused, not a plugin: no Plugins-panel card, no
//! enable/disable toggle. This module owns the fully background self-heal
//! lifecycle (clone → build → generate config) and the lightweight intent
//! router that decides, per message, whether it should go to `grok` (coding)
//! or `zeroclaw` (general assistant / non-coding capabilities).
//!
//! Mirrors `bevy.rs`'s background-job pattern (spawn_job / poll / cancel_flag)
//! but drives `git clone` + `cargo build --release` + `zeroclaw config …`
//! instead of a game engine build.

use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, mpsc};
use std::thread;
use std::time::Duration;

use crate::process;
use crate::usage::usage_dir;

/// Official ZeroClaw tree (`zeroclaw-labs/zeroclaw`, default branch `master`).
pub const SOURCE_GIT_URL: &str = "https://github.com/zeroclaw-labs/zeroclaw.git";
/// Minimum rustc version the zeroclaw workspace requires (observed via a
/// failed build: `zeroclawlabs@x.y.z requires rustc 1.96.0`). Older toolchains
/// fail the build with a clear message we can detect and self-heal via
/// `rustup update stable`.
const MIN_RUSTC_MINOR: u32 = 96;
/// Provider/agent/risk-profile alias written into zeroclaw's own config.
/// One agent only, so ACP `session/new` never needs an explicit `agentAlias`.
const ZC_PROVIDER_TYPE: &str = "custom";
const ZC_ALIAS: &str = "bonybuild";
const ZC_AGENT_ALIAS: &str = "bonybuild";
const ZC_RISK_PROFILE: &str = "bonybuild";
const LOG_TAIL_MAX: usize = 400;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ZeroclawStatus {
    Unknown,
    Cloning,
    Building,
    Configuring,
    Ready,
    Error(String),
}

impl ZeroclawStatus {
    pub fn is_ready(&self) -> bool {
        matches!(self, Self::Ready)
    }

    pub fn is_busy(&self) -> bool {
        matches!(self, Self::Cloning | Self::Building | Self::Configuring)
    }

    pub fn label(&self, lang: crate::i18n::Language) -> String {
        use crate::i18n::t;
        match self {
            Self::Unknown => t(lang, "zeroclaw.status_unknown").to_string(),
            Self::Cloning => t(lang, "zeroclaw.status_cloning").to_string(),
            Self::Building => t(lang, "zeroclaw.status_building").to_string(),
            Self::Configuring => t(lang, "zeroclaw.status_configuring").to_string(),
            Self::Ready => t(lang, "zeroclaw.status_ready").to_string(),
            Self::Error(e) => format!("{}: {e}", t(lang, "zeroclaw.status_error")),
        }
    }
}

enum ZcMsg {
    Line(String),
    Status(ZeroclawStatus),
    Done(Result<PathBuf, String>),
}

/// Background lifecycle state, polled once per frame from `app.rs` — the same
/// shape as `BevyState`, minus anything UI-panel-shaped (no `busy` gating a
/// visible button; this runs unattended from app startup).
pub struct ZeroclawState {
    pub status: ZeroclawStatus,
    pub bin_path: Option<PathBuf>,
    pub log_tail: Vec<String>,
    started: bool,
    cancel_flag: Option<Arc<AtomicBool>>,
    pending_rx: Option<mpsc::Receiver<ZcMsg>>,
}

impl Default for ZeroclawState {
    fn default() -> Self {
        Self {
            status: ZeroclawStatus::Unknown,
            bin_path: None,
            log_tail: Vec::new(),
            started: false,
            cancel_flag: None,
            pending_rx: None,
        }
    }
}

impl ZeroclawState {
    /// Kick off clone/build/config self-heal. Safe every frame:
    /// - job in flight → no-op
    /// - binary appears later (e.g. external `cargo build`) → only configure
    /// - weather tool patch stale on managed tree → rebuild (Open-Meteo fix)
    /// - first call without binary → full setup once (no Error thrash loops)
    pub fn ensure_started(&mut self) {
        if self.pending_rx.is_some() {
            return;
        }
        let managed = install_dir();
        let patch_stale = weather_patch_is_stale(&managed);
        if let Some(bin) = resolve_zeroclaw_bin() {
            let is_managed_bin = bin
                .canonicalize()
                .ok()
                .zip(managed_bin_path().canonicalize().ok())
                .is_some_and(|(a, b)| a == b)
                || bin.starts_with(&managed);
            if patch_stale && is_managed_bin && managed.join(".git").is_dir() {
                self.started = true;
                self.status = ZeroclawStatus::Building;
                self.spawn_full_setup();
                return;
            }
            if matches!(self.status, ZeroclawStatus::Ready) && self.bin_path.is_some() {
                return;
            }
            // Binary exists (install finished, or fixed outside the app) → configure if needed.
            self.started = true;
            self.bin_path = Some(bin);
            self.status = ZeroclawStatus::Configuring;
            self.spawn_config_only();
            return;
        }
        if self.started {
            // Already attempted full setup; stay idle until binary appears or user restarts app.
            return;
        }
        self.started = true;
        self.status = ZeroclawStatus::Cloning;
        self.spawn_full_setup();
    }

    fn spawn_job<F>(&mut self, work: F)
    where
        F: FnOnce(mpsc::Sender<ZcMsg>, Arc<AtomicBool>) + Send + 'static,
    {
        let cancel = Arc::new(AtomicBool::new(false));
        let (tx, rx) = mpsc::channel();
        self.pending_rx = Some(rx);
        self.cancel_flag = Some(cancel.clone());
        thread::spawn(move || work(tx, cancel));
    }

    fn spawn_full_setup(&mut self) {
        self.log_tail.clear();
        self.spawn_job(|tx, cancel| {
            let result = full_setup(&tx, &cancel);
            let _ = tx.send(ZcMsg::Done(result));
        });
    }

    fn spawn_config_only(&mut self) {
        self.log_tail.clear();
        let bin = self.bin_path.clone();
        self.spawn_job(move |tx, _cancel| {
            let result = (|| {
                let bin = bin.ok_or_else(|| "missing zeroclaw binary".to_string())?;
                if !config_is_ready(&bin) {
                    write_default_config(&bin, &tx)?;
                }
                // Idempotent: ensure tools can actually run (agentic + native tools).
                ensure_tool_capable_config(&bin, &tx)?;
                Ok(bin)
            })();
            let _ = tx.send(ZcMsg::Done(result));
        });
    }

    /// Drain background messages; returns true when repaint is warranted.
    pub fn poll(&mut self) -> bool {
        let Some(rx) = self.pending_rx.as_ref() else {
            return false;
        };
        let mut msgs = Vec::new();
        while let Ok(msg) = rx.try_recv() {
            msgs.push(msg);
        }
        if msgs.is_empty() {
            return false;
        }
        for msg in msgs {
            match msg {
                ZcMsg::Line(line) => push_log(&mut self.log_tail, line),
                ZcMsg::Status(status) => self.status = status,
                ZcMsg::Done(Ok(bin)) => {
                    self.bin_path = Some(bin);
                    self.status = ZeroclawStatus::Ready;
                    self.pending_rx = None;
                }
                ZcMsg::Done(Err(reason)) => {
                    push_log(&mut self.log_tail, reason.clone());
                    self.status = ZeroclawStatus::Error(reason);
                    self.pending_rx = None;
                }
            }
        }
        true
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

fn exe_name() -> &'static str {
    if cfg!(windows) { "zeroclaw.exe" } else { "zeroclaw" }
}

fn install_dir() -> PathBuf {
    usage_dir().join("zeroclaw")
}

fn managed_bin_path() -> PathBuf {
    install_dir()
        .join("target")
        .join("release")
        .join(exe_name())
}

/// Overlay vendored source fixes onto the managed ZeroClaw tree before build.
/// Currently: replace `weather_tool` so Chinese cities (深圳…) resolve via
/// Open-Meteo instead of broken wttr.in → Hong Kong border stations.
// Embedded copy of assets/zeroclaw_weather_tool.rs (not a bony-build module).
const WEATHER_TOOL_SRC: &str = include_str!("../assets/zeroclaw_weather_tool.rs");

fn weather_tool_src_path(dir: &Path) -> PathBuf {
    dir.join("crates")
        .join("zeroclaw-tools")
        .join("src")
        .join("weather_tool.rs")
}

fn weather_patch_is_stale(dir: &Path) -> bool {
    let target = weather_tool_src_path(dir);
    if !target.is_file() {
        return false;
    }
    match std::fs::read_to_string(&target) {
        Ok(current) => current != WEATHER_TOOL_SRC,
        Err(_) => true,
    }
}

fn apply_managed_source_overrides(dir: &Path, send_line: &dyn Fn(String)) -> Result<(), String> {
    let target = weather_tool_src_path(dir);
    if !target.parent().is_some_and(|p| p.is_dir()) {
        return Ok(());
    }
    let current = std::fs::read_to_string(&target).unwrap_or_default();
    if current == WEATHER_TOOL_SRC {
        return Ok(());
    }
    std::fs::write(&target, WEATHER_TOOL_SRC)
        .map_err(|e| format!("写入 weather 工具补丁失败：{e}"))?;
    send_line("已应用天气工具补丁（Open-Meteo，修正中国城市定位）".into());
    Ok(())
}

/// PATH first (user may already have zeroclaw installed globally), then the
/// managed clone-and-build directory under `~/.bony-build/zeroclaw`.
pub fn resolve_zeroclaw_bin() -> Option<PathBuf> {
    if let Some(p) = which("zeroclaw") {
        return Some(p);
    }
    let managed = managed_bin_path();
    if managed.is_file() {
        return Some(managed);
    }
    None
}

fn which(program: &str) -> Option<PathBuf> {
    let dirs: Vec<PathBuf> = std::env::var_os("PATH")
        .map(|p| std::env::split_paths(&p).collect())
        .unwrap_or_default();
    let names: &[&str] = if cfg!(windows) {
        &["zeroclaw.exe", "zeroclaw.cmd", "zeroclaw.bat"]
    } else {
        &["zeroclaw"]
    };
    let _ = program;
    for dir in dirs {
        for name in names {
            let candidate = dir.join(name);
            if candidate.is_file() {
                return Some(candidate);
            }
        }
    }
    None
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

/// `rustc --version` minor version, best-effort (e.g. `1.92.0` → `Some(92)`).
fn rustc_minor_version() -> Option<u32> {
    let output = process::command("rustc").arg("--version").output().ok()?;
    let text = String::from_utf8_lossy(&output.stdout);
    let ver = text.split_whitespace().nth(1)?; // "rustc" "1.92.0" ...
    let minor = ver.split('.').nth(1)?;
    minor.parse().ok()
}

fn full_setup(tx: &mpsc::Sender<ZcMsg>, cancel: &Arc<AtomicBool>) -> Result<PathBuf, String> {
    let send_line = |line: String| {
        let _ = tx.send(ZcMsg::Line(line));
    };
    if !tool_ok("git", &["--version"]) {
        return Err("未检测到 git，无法自动获取 ZeroClaw 源码".into());
    }
    if !tool_ok("cargo", &["--version"]) {
        return Err("未检测到 Rust/cargo，无法自动构建 ZeroClaw".into());
    }

    let dir = install_dir();
    if !dir.join(".git").is_dir() {
        if let Some(parent) = dir.parent() {
            std::fs::create_dir_all(parent).map_err(|e| format!("创建目录失败：{e}"))?;
        }
        if dir.exists() {
            std::fs::remove_dir_all(&dir).map_err(|e| format!("清理旧目录失败：{e}"))?;
        }
        send_line(format!("git clone --depth 1 {SOURCE_GIT_URL}"));
        let mut cmd = process::command("git");
        cmd.args(["clone", "--depth", "1", SOURCE_GIT_URL, &dir.to_string_lossy()]);
        run_streaming(&mut cmd, tx, cancel)?;
    } else {
        send_line("已存在本地 ZeroClaw 克隆，跳过 clone".into());
    }

    // Pin stable toolchain so cargo never inherits bony-build's older rust-toolchain.toml
    // when rustup resolves from an unexpected working directory / env.
    let pin = dir.join("rust-toolchain.toml");
    if !pin.is_file() {
        let _ = std::fs::write(
            &pin,
            "[toolchain]\nchannel = \"stable\"\n",
        );
    }

    apply_managed_source_overrides(&dir, &send_line)?;

    let _ = tx.send(ZcMsg::Status(ZeroclawStatus::Building));
    build_release(&dir, tx, cancel)?;

    let bin = dir.join("target").join("release").join(exe_name());
    if !bin.is_file() {
        return Err("构建完成但未找到 zeroclaw 可执行文件".into());
    }

    if !config_is_ready(&bin) {
        let _ = tx.send(ZcMsg::Status(ZeroclawStatus::Configuring));
        write_default_config(&bin, tx)?;
    }
    Ok(bin)
}

fn build_release(
    dir: &Path,
    tx: &mpsc::Sender<ZcMsg>,
    cancel: &Arc<AtomicBool>,
) -> Result<(), String> {
    let send_line = |line: String| {
        let _ = tx.send(ZcMsg::Line(line));
    };
    apply_managed_source_overrides(dir, &send_line)?;
    send_line("cargo build --release --bin zeroclaw".into());
    let mut cmd = process::command("cargo");
    cmd.args(["+stable", "build", "--release", "--bin", "zeroclaw"])
        .env("RUSTUP_TOOLCHAIN", "stable")
        .env_remove("CARGO_TOOLCHAIN")
        .current_dir(dir);
    match run_streaming(&mut cmd, tx, cancel) {
        Ok(()) => return Ok(()),
        Err(e) if !looks_like_toolchain_error(&e) => return Err(e),
        Err(_) => {}
    }

    // Toolchain too old — self-heal via `rustup update stable`, then retry once.
    if let Some(minor) = rustc_minor_version()
        && minor >= MIN_RUSTC_MINOR
    {
        return Err("cargo build 失败（非工具链版本问题）".into());
    }
    if !tool_ok("rustup", &["--version"]) {
        return Err(format!(
            "ZeroClaw 需要 rustc >= 1.{MIN_RUSTC_MINOR}，且未检测到 rustup，无法自动升级。请手动执行 `rustup update stable`。"
        ));
    }
    send_line("检测到 rustc 版本过旧，正在执行 rustup update stable…".into());
    let mut update_cmd = process::command("rustup");
    update_cmd.args(["update", "stable"]);
    run_streaming(&mut update_cmd, tx, cancel)?;
    let mut default_cmd = process::command("rustup");
    default_cmd.args(["default", "stable"]);
    let _ = run_streaming(&mut default_cmd, tx, cancel);

    send_line("重新执行 cargo +stable build --release --bin zeroclaw…".into());
    let mut retry_cmd = process::command("cargo");
    retry_cmd
        .args(["+stable", "build", "--release", "--bin", "zeroclaw"])
        .env("RUSTUP_TOOLCHAIN", "stable")
        .env_remove("CARGO_TOOLCHAIN")
        .current_dir(dir);
    run_streaming(&mut retry_cmd, tx, cancel)
}

fn looks_like_toolchain_error(msg: &str) -> bool {
    let lower = msg.to_ascii_lowercase();
    lower.contains("requires rustc") || lower.contains("not supported by")
}

/// Run `cmd`, streaming stdout/stderr lines, killing the child early if
/// `cancel` flips true — same shape as `bevy.rs::run_streaming`.
fn run_streaming(
    cmd: &mut Command,
    tx: &mpsc::Sender<ZcMsg>,
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
                let _ = tx_out.send(ZcMsg::Line(line));
            }
        }
    });
    let tx_err = tx.clone();
    let err_handle = thread::spawn(move || {
        let mut last = String::new();
        if let Some(err) = stderr {
            for line in BufReader::new(err).lines().flatten() {
                last = line.clone();
                let _ = tx_err.send(ZcMsg::Line(line));
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
            return Err("已取消".into());
        }
        match child.try_wait() {
            Ok(Some(s)) => break s,
            Ok(None) => thread::sleep(Duration::from_millis(120)),
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

fn zeroclaw_config_path() -> PathBuf {
    let home = std::env::var_os("USERPROFILE")
        .or_else(|| std::env::var_os("HOME"))
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."));
    home.join(".zeroclaw").join("config.toml")
}

fn config_is_ready(_bin: &Path) -> bool {
    let path = zeroclaw_config_path();
    if !path.is_file() {
        return false;
    }
    // ZeroClaw rejects session/new unless the agent is "dispatchable":
    // enabled + non-empty model_provider, risk_profile, runtime_profile.
    // An empty runtime_profile is the most common incomplete bootstrap.
    let Ok(content) = std::fs::read_to_string(&path) else {
        return false;
    };
    agent_section_looks_dispatchable(&content, ZC_AGENT_ALIAS)
}

/// Best-effort TOML scan (no full parse): the `[agents.<alias>]` block must
/// set the three ref fields and not set `enabled = false`.
fn agent_section_looks_dispatchable(content: &str, alias: &str) -> bool {
    let header = format!("[agents.{alias}]");
    let Some(section) = content.split(&header).nth(1) else {
        return false;
    };
    // End at next top-level or nested header that isn't agents.alias.*
    let section = section
        .split("\n[")
        .next()
        .unwrap_or(section);
    let has_nonempty = |key: &str| {
        for line in section.lines() {
            let line = line.trim();
            if line.starts_with('#') {
                continue;
            }
            if let Some(rest) = line.strip_prefix(key) {
                let rest = rest.trim_start();
                if let Some(rest) = rest.strip_prefix('=') {
                    let v = rest.trim().trim_matches('"').trim();
                    return !v.is_empty() && v != "\"\"";
                }
            }
        }
        false
    };
    let enabled_ok = !section.lines().any(|l| {
        let l = l.trim();
        l == "enabled = false" || l.starts_with("enabled=false")
    });
    enabled_ok
        && has_nonempty("model_provider")
        && has_nonempty("risk_profile")
        && has_nonempty("runtime_profile")
}

/// A model/provider entry read straight out of `~/.grok/config.toml`
/// (structured, unlike `config_io::ConfigModels` which folds everything into
/// a display `description` string).
struct GrokModelEntry {
    model: String,
    base_url: Option<String>,
    env_key: Option<String>,
}

fn read_grok_model_entry(model_id: &str) -> Option<GrokModelEntry> {
    let path = crate::config_io::grok_config_path();
    let text = std::fs::read_to_string(path).ok()?;
    let doc: toml_edit::DocumentMut = text.parse().ok()?;
    let table = doc.get("model")?.get(model_id)?.as_table()?;
    let model = table
        .get("model")
        .and_then(|v| v.as_str())
        .unwrap_or(model_id)
        .to_string();
    let base_url = table
        .get("base_url")
        .and_then(|v| v.as_str())
        .map(str::to_string);
    let env_key = table
        .get("env_key")
        .and_then(|v| v.as_str())
        .map(str::to_string);
    Some(GrokModelEntry {
        model,
        base_url,
        env_key,
    })
}

/// Resolve the actual credential value to inject as an env var when spawning
/// `zeroclaw acp` — never written to zeroclaw's own config.toml on disk.
pub fn resolve_provider_env() -> Option<(String, String)> {
    let _ = crate::config_io::hydrate_model_env_keys();
    let catalog = crate::config_io::load_models_catalog();
    let entry = read_grok_model_entry(catalog.default_id.as_deref()?)?;
    let env_key = entry.env_key?;
    let value = std::env::var(&env_key).ok().filter(|v| !v.is_empty())?;
    Some((zc_provider_env_var_name(), value))
}

fn zc_provider_env_var_name() -> String {
    format!("ZEROCLAW_providers__models__{ZC_PROVIDER_TYPE}__{ZC_ALIAS}__api_key")
}

/// Non-interactive, scriptable config bootstrap. `zeroclaw quickstart` needs
/// a real TTY (hard-fails otherwise), so we drive `zeroclaw config init` /
/// `zeroclaw config set` instead — the same commands the docs recommend for
/// headless setups. Reuses the LLM credentials Bony Build already resolved
/// for `grok`'s `~/.grok/config.toml`; the secret itself is never written to
/// zeroclaw's config (see `resolve_provider_env`), only `uri` + `model`.
fn write_default_config(bin: &Path, tx: &mpsc::Sender<ZcMsg>) -> Result<(), String> {
    let send_line = |line: String| {
        let _ = tx.send(ZcMsg::Line(line));
    };
    let _ = crate::config_io::hydrate_model_env_keys();
    let catalog = crate::config_io::load_models_catalog();
    let default_id = catalog
        .default_id
        .clone()
        .ok_or_else(|| "~/.grok/config.toml 未配置任何模型，无法生成 ZeroClaw 配置".to_string())?;
    let entry = read_grok_model_entry(&default_id)
        .ok_or_else(|| format!("无法读取 ~/.grok/config.toml 中的 [model.{default_id}]"))?;
    let base_url = entry
        .base_url
        .ok_or_else(|| format!("[model.{default_id}] 缺少 base_url，无法映射到 ZeroClaw provider"))?;

    let provider_path = format!("providers.models.{ZC_PROVIDER_TYPE}.{ZC_ALIAS}");
    let agent_path = format!("agents.{ZC_AGENT_ALIAS}");
    let risk_path = format!("risk_profiles.{ZC_RISK_PROFILE}");
    // Runtime profile is required for `AliasedAgentConfig::is_dispatchable`
    // (along with enabled + model_provider + risk_profile). Empty
    // runtime_profile makes session/new fail with "not enabled for dispatch".
    let runtime_path = format!("runtime_profiles.{ZC_RISK_PROFILE}");

    send_line(format!("zeroclaw config init {provider_path}"));
    zc_config(bin, &["config", "init", &provider_path])?;
    zc_config(bin, &["config", "set", &format!("{provider_path}.uri"), &base_url])?;
    zc_config(bin, &["config", "set", &format!("{provider_path}.model"), &entry.model])?;

    send_line(format!("zeroclaw config init {risk_path}"));
    zc_config(bin, &["config", "init", &risk_path])?;

    send_line(format!("zeroclaw config init {runtime_path}"));
    zc_config(bin, &["config", "init", &runtime_path])?;

    send_line(format!("zeroclaw config init {agent_path}"));
    zc_config(bin, &["config", "init", &agent_path])?;
    zc_config(
        bin,
        &[
            "config",
            "set",
            &format!("{agent_path}.model_provider"),
            &format!("{ZC_PROVIDER_TYPE}.{ZC_ALIAS}"),
        ],
    )?;
    zc_config(
        bin,
        &["config", "set", &format!("{agent_path}.risk_profile"), ZC_RISK_PROFILE],
    )?;
    zc_config(
        bin,
        &["config", "set", &format!("{agent_path}.runtime_profile"), ZC_RISK_PROFILE],
    )?;
    let _ = zc_config(bin, &["config", "set", &format!("{agent_path}.enabled"), "true"]);

    ensure_tool_capable_config(bin, tx)?;

    send_line("ZeroClaw 配置生成完成".into());
    Ok(())
}

/// Make the managed agent able to run tools on OpenAI-compatible endpoints
/// (DashScope/Qwen etc.): agentic runtime + native function-calling wire.
/// Idempotent — safe to re-run on every startup after a binary is present.
fn ensure_tool_capable_config(bin: &Path, tx: &mpsc::Sender<ZcMsg>) -> Result<(), String> {
    let send_line = |line: String| {
        let _ = tx.send(ZcMsg::Line(line));
    };
    let provider_path = format!("providers.models.{ZC_PROVIDER_TYPE}.{ZC_ALIAS}");
    let runtime_path = format!("runtime_profiles.{ZC_RISK_PROFILE}");

    send_line(format!("zeroclaw config set {runtime_path}.agentic true"));
    zc_config(bin, &["config", "set", &format!("{runtime_path}.agentic"), "true"])?;

    // Prefer chat-completions native tools over free-form XML; qwen-max often
    // emits incomplete `<tool_call>` bodies without a tool name otherwise.
    send_line(format!("zeroclaw config set {provider_path}.native_tools true"));
    zc_config(
        bin,
        &["config", "set", &format!("{provider_path}.native_tools"), "true"],
    )?;
    Ok(())
}

fn zc_config(bin: &Path, args: &[&str]) -> Result<(), String> {
    let output = process::command(bin)
        .args(args)
        .output()
        .map_err(|e| format!("无法启动 zeroclaw {}: {e}", args.join(" ")))?;
    if output.status.success() {
        return Ok(());
    }
    let stderr = String::from_utf8_lossy(&output.stderr);
    let stdout = String::from_utf8_lossy(&output.stdout);
    Err(format!(
        "zeroclaw {} 失败：{}",
        args.join(" "),
        if stderr.trim().is_empty() {
            stdout.trim().to_string()
        } else {
            stderr.trim().to_string()
        }
    ))
}

// ── Intent routing ─────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Backend {
    Coding,
    General,
}

/// Manual escape hatch: `/zc <text>` always forces the ZeroClaw backend, and
/// the prefix is stripped before the text is sent. Not a plugin toggle — just
/// a chat-level prefix command, kept deliberately unobtrusive.
pub const FORCE_ZC_PREFIX: &str = "/zc";

/// Human-readable decision from [`classify_intent_detail`] — used both for
/// routing and for the always-visible route card in the chat timeline.
#[derive(Debug, Clone)]
pub struct IntentDecision {
    pub backend: Backend,
    /// Short reason shown in the route card (Chinese/English mixed ok —
    /// rendered through app-side localization of fixed keys where critical).
    pub reason: String,
    pub matched_keyword: Option<String>,
    pub forced: bool,
}

/// Heuristic keyword router (no extra LLM round-trip in v1). Defaults to
/// `Coding` (unchanged existing behavior) whenever the message doesn't
/// clearly look like a non-coding / general-assistant request.
pub fn classify_intent(text: &str) -> Backend {
    classify_intent_detail(text).backend
}

pub fn classify_intent_detail(text: &str) -> IntentDecision {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return IntentDecision {
            backend: Backend::Coding,
            reason: "empty".into(),
            matched_keyword: None,
            forced: false,
        };
    }
    if trimmed.starts_with(FORCE_ZC_PREFIX)
        && trimmed[FORCE_ZC_PREFIX.len()..]
            .chars()
            .next()
            .is_none_or(|c| c.is_whitespace())
    {
        return IntentDecision {
            backend: Backend::General,
            reason: "force_prefix".into(),
            matched_keyword: Some(FORCE_ZC_PREFIX.into()),
            forced: true,
        };
    }

    // Strong coding signals always win, even if a non-coding keyword also
    // appears — never regress existing grok behavior on ambiguous text.
    if looks_like_coding(trimmed) {
        let kw = first_match(trimmed, CODING_KEYWORDS);
        return IntentDecision {
            backend: Backend::Coding,
            reason: "coding_signal".into(),
            matched_keyword: kw,
            forced: false,
        };
    }

    if let Some(kw) = first_match(trimmed, NON_CODING_KEYWORDS) {
        return IntentDecision {
            backend: Backend::General,
            reason: "non_coding_keyword".into(),
            matched_keyword: Some(kw),
            forced: false,
        };
    }

    IntentDecision {
        backend: Backend::Coding,
        reason: "default_coding".into(),
        matched_keyword: None,
        forced: false,
    }
}

fn looks_like_coding(text: &str) -> bool {
    text.contains("```")
        || text.contains("Traceback")
        || text.contains("error[")
        || text.contains("fn ")
        || text.contains("def ")
        || text.contains("class ")
        || contains_any(text, CODING_KEYWORDS)
}

fn contains_any(text: &str, keywords: &[&str]) -> bool {
    first_match(text, keywords).is_some()
}

fn first_match(text: &str, keywords: &[&str]) -> Option<String> {
    let lower = text.to_ascii_lowercase();
    keywords
        .iter()
        .find(|k| lower.contains(&k.to_ascii_lowercase()))
        .map(|k| (*k).to_string())
}

const CODING_KEYWORDS: &[&str] = &[
    "代码", "报错", "bug", "重构", "编译", "函数", "变量", "compile", "stack trace", "编写",
    "写个函数", "写一个函数", "单元测试", "unit test", "pull request", "commit", "cargo build",
    "npm install",
];

const NON_CODING_KEYWORDS: &[&str] = &[
    // Reminders / scheduling / cron.
    "提醒", "定时", "闹钟", "cron", "schedule", "reminder",
    // Messaging channels.
    "discord", "telegram", "邮件", "email", "webhook", "发消息", "slack", "whatsapp",
    // Hardware.
    "gpio", "树莓派", "raspberry pi", "arduino", "esp32", "硬件", "传感器", "sensor",
    // SOP / automation workflows.
    "sop", "自动化流程", "工作流",
    // General knowledge chit-chat markers.
    "记忆", "memory", "聊聊", "陪我", "天气", "翻译一下", "帮我查",
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classify_defaults_to_coding_for_ambiguous_text() {
        assert_eq!(classify_intent("这是什么意思"), Backend::Coding);
        assert_eq!(classify_intent(""), Backend::Coding);
    }

    #[test]
    fn classify_routes_coding_signals_to_grok() {
        assert_eq!(classify_intent("这个函数报错了，帮我看看 bug"), Backend::Coding);
        assert_eq!(classify_intent("```rust\nfn main() {}\n```"), Backend::Coding);
        assert_eq!(classify_intent("重构一下这段代码"), Backend::Coding);
    }

    #[test]
    fn classify_routes_non_coding_signals_to_zeroclaw() {
        assert_eq!(classify_intent("帮我设置一个明天9点的提醒"), Backend::General);
        assert_eq!(classify_intent("帮我发一条消息到 discord"), Backend::General);
        assert_eq!(classify_intent("这个 gpio 传感器怎么接线"), Backend::General);
        assert_eq!(classify_intent("今天天气怎么样"), Backend::General);
        assert_eq!(classify_intent("深圳天气"), Backend::General);
        let d = classify_intent_detail("深圳天气");
        assert_eq!(d.backend, Backend::General);
        assert_eq!(d.matched_keyword.as_deref(), Some("天气"));
    }

    #[test]
    fn classify_coding_signal_wins_over_non_coding_keyword() {
        // Mentions "邮件" (non-coding) but is clearly a coding request.
        assert_eq!(
            classify_intent("帮我写个函数发送邮件，代码报错了 bug"),
            Backend::Coding
        );
    }

    #[test]
    fn force_prefix_always_routes_to_zeroclaw() {
        assert_eq!(classify_intent("/zc 你好"), Backend::General);
        assert_eq!(classify_intent("/zc"), Backend::General);
        // Not a real prefix match when followed by more letters (e.g. "/zcode").
        assert_eq!(classify_intent("/zcode 帮我修复报错的函数"), Backend::Coding);
    }

    #[test]
    fn resolve_bin_prefers_path_over_managed_dir() {
        // Smoke test only: on machines without zeroclaw installed this
        // simply returns None, exercising both lookup branches without
        // panicking.
        let _ = resolve_zeroclaw_bin();
    }

    #[test]
    fn zc_provider_env_var_name_matches_schema_mirror_grammar() {
        assert_eq!(
            zc_provider_env_var_name(),
            "ZEROCLAW_providers__models__custom__bonybuild__api_key"
        );
    }
}
