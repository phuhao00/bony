//! Minimal MCP (stdio JSON-RPC) surface for Buzz room specialist agents.
//!
//! Tools:
//! - `unity_cli` — run Unity CLI subcommands (probe / eval / play / pipeline …)
//! - `openmontage_status` — check managed OpenMontage install readiness
//! - `openmontage_preflight` — run provider menu summary against the venv
//! - `openmontage_run` — run a Python helper under the OpenMontage root
//!
//! Designed to be launched as `BUZZ_ACP_MCP_COMMAND` for a `buzz-agent`
//! persona (Unity specialist or OpenMontage specialist).

use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::Duration;

use anyhow::{Context, Result};
use serde_json::{json, Value};

fn main() {
    if let Err(e) = run() {
        eprintln!("bony-room-tools-mcp fatal: {e:#}");
        std::process::exit(1);
    }
}

fn run() -> Result<()> {
    let stdin = std::io::stdin();
    let mut stdout = std::io::stdout();
    let reader = BufReader::new(stdin.lock());

    for line in reader.lines() {
        let line = line.context("read stdin")?;
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let req: Value = match serde_json::from_str(line) {
            Ok(v) => v,
            Err(e) => {
                eprintln!("skip bad json: {e}");
                continue;
            }
        };
        if let Some(resp) = handle_message(&req) {
            let out = serde_json::to_string(&resp)?;
            writeln!(stdout, "{out}")?;
            stdout.flush()?;
        }
    }
    Ok(())
}

fn handle_message(req: &Value) -> Option<Value> {
    let method = req.get("method")?.as_str()?;
    let id = req.get("id").cloned();
    // Notifications have no id — ignore quietly.
    if id.is_none() && method.starts_with("notifications/") {
        return None;
    }

    let result = match method {
        "initialize" => Ok(json!({
            "protocolVersion": req
                .pointer("/params/protocolVersion")
                .cloned()
                .unwrap_or_else(|| json!("2024-11-05")),
            "capabilities": { "tools": {} },
            "serverInfo": {
                "name": "bony-room-tools-mcp",
                "version": env!("CARGO_PKG_VERSION")
            }
        })),
        "ping" => Ok(json!({})),
        "tools/list" => Ok(json!({ "tools": tool_defs() })),
        "tools/call" => {
            let name = req
                .pointer("/params/name")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            let args = req
                .pointer("/params/arguments")
                .cloned()
                .unwrap_or_else(|| json!({}));
            Ok(call_tool(name, &args))
        }
        other => Err(rpc_error(-32601, format!("Method not found: {other}"))),
    };

    match (id, result) {
        (Some(id), Ok(result)) => Some(json!({
            "jsonrpc": "2.0",
            "id": id,
            "result": result
        })),
        (Some(id), Err(err)) => Some(json!({
            "jsonrpc": "2.0",
            "id": id,
            "error": err
        })),
        _ => None,
    }
}

fn rpc_error(code: i64, message: String) -> Value {
    json!({ "code": code, "message": message })
}

fn tool_defs() -> Vec<Value> {
    vec![
        json!({
            "name": "unity_cli",
            "description": "Run a Unity CLI command (standalone `unity` binary). Prefer: status, project, eval, play, pause, pipeline list/install, test, build. Pass `args` as a JSON array of CLI tokens after the binary (example: [\"eval\", \"--code\", \"return true;\"]). Working directory may be set via `cwd`.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "args": {
                        "type": "array",
                        "items": { "type": "string" },
                        "description": "Arguments after the unity binary"
                    },
                    "cwd": {
                        "type": "string",
                        "description": "Optional working directory (Unity project root preferred)"
                    },
                    "timeout_secs": {
                        "type": "integer",
                        "description": "Timeout seconds (default 120, max 1800)"
                    }
                },
                "required": ["args"]
            }
        }),
        json!({
            "name": "openmontage_status",
            "description": "Check whether the managed OpenMontage install is ready (clone + venv + deps).",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "root": {
                        "type": "string",
                        "description": "OpenMontage root (default: %USERPROFILE%/.bony-build/openmontage)"
                    }
                }
            }
        }),
        json!({
            "name": "openmontage_preflight",
            "description": "Run provider-menu preflight summary using OpenMontage venv Python.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "root": { "type": "string" }
                }
            }
        }),
        json!({
            "name": "openmontage_run",
            "description": "Run a Python script file under the OpenMontage root with the managed venv. Path is relative to root unless absolute. Prefer small helper scripts over python -c.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "root": { "type": "string" },
                    "script": {
                        "type": "string",
                        "description": "Script path relative to OpenMontage root or absolute"
                    },
                    "args": {
                        "type": "array",
                        "items": { "type": "string" }
                    },
                    "timeout_secs": { "type": "integer" }
                },
                "required": ["script"]
            }
        }),
    ]
}

fn call_tool(name: &str, args: &Value) -> Value {
    let result = match name {
        "unity_cli" => tool_unity_cli(args),
        "openmontage_status" => tool_openmontage_status(args),
        "openmontage_preflight" => tool_openmontage_preflight(args),
        "openmontage_run" => tool_openmontage_run(args),
        other => Err(format!("unknown tool: {other}")),
    };
    match result {
        Ok(text) => json!({
            "content": [{ "type": "text", "text": text }],
            "isError": false
        }),
        Err(err) => json!({
            "content": [{ "type": "text", "text": format!("Error: {err}") }],
            "isError": true
        }),
    }
}

fn tool_unity_cli(args: &Value) -> std::result::Result<String, String> {
    let tokens = args
        .get("args")
        .and_then(|v| v.as_array())
        .ok_or_else(|| "args must be a string array".to_string())?
        .iter()
        .map(|v| {
            v.as_str()
                .map(str::to_string)
                .ok_or_else(|| "args items must be strings".to_string())
        })
        .collect::<std::result::Result<Vec<_>, _>>()?;
    if tokens.is_empty() {
        return Err("args must not be empty (e.g. [\"--help\"])".into());
    }
    let cwd = args.get("cwd").and_then(|v| v.as_str()).map(PathBuf::from);
    let timeout = timeout_from(args, 120, 1800);
    let bin = resolve_unity_bin().ok_or_else(|| {
        "unity CLI not found on PATH or default install locations".to_string()
    })?;
    run_capture(&bin, &tokens, cwd.as_deref(), timeout)
}

fn tool_openmontage_status(args: &Value) -> std::result::Result<String, String> {
    let root = openmontage_root(args);
    let status = openmontage_check(&root);
    Ok(format!("root={}\nstatus={status}", root.display()))
}

fn tool_openmontage_preflight(args: &Value) -> std::result::Result<String, String> {
    let root = openmontage_root(args);
    let py = venv_python(&root);
    if !py.is_file() {
        return Err(format!("venv python missing: {}", py.display()));
    }
    // Prefer a small helper if present; else dump a short diagnostic.
    let helper = root.join("scripts").join("provider_menu_summary.py");
    if helper.is_file() {
        return run_capture(
            &py,
            &[helper.to_string_lossy().into_owned()],
            Some(&root),
            Duration::from_secs(120),
        );
    }
    // Fallback: import check only.
    run_capture(
        &py,
        &[
            "-c".into(),
            "import sys; print('python', sys.version); print('root_ok', True)".into(),
        ],
        Some(&root),
        Duration::from_secs(30),
    )
}

fn tool_openmontage_run(args: &Value) -> std::result::Result<String, String> {
    let root = openmontage_root(args);
    let script = args
        .get("script")
        .and_then(|v| v.as_str())
        .ok_or_else(|| "script is required".to_string())?;
    let mut script_path = PathBuf::from(script);
    if script_path.is_relative() {
        script_path = root.join(script_path);
    }
    if !script_path.is_file() {
        return Err(format!("script not found: {}", script_path.display()));
    }
    let py = venv_python(&root);
    if !py.is_file() {
        return Err(format!("venv python missing: {}", py.display()));
    }
    let extra: Vec<String> = args
        .get("args")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str().map(str::to_string))
                .collect()
        })
        .unwrap_or_default();
    let mut tokens = vec![script_path.to_string_lossy().into_owned()];
    tokens.extend(extra);
    let timeout = timeout_from(args, 300, 1800);
    run_capture(&py, &tokens, Some(&root), timeout)
}

fn openmontage_root(args: &Value) -> PathBuf {
    if let Some(p) = args.get("root").and_then(|v| v.as_str()) {
        return PathBuf::from(p);
    }
    if let Ok(p) = std::env::var("OPENMONTAGE_ROOT") {
        if !p.is_empty() {
            return PathBuf::from(p);
        }
    }
    dirs_openmontage_default()
}

fn dirs_openmontage_default() -> PathBuf {
    if let Some(home) = std::env::var_os("USERPROFILE").or_else(|| std::env::var_os("HOME")) {
        return PathBuf::from(home)
            .join(".bony-build")
            .join("openmontage");
    }
    PathBuf::from(".bony-build").join("openmontage")
}

fn venv_python(root: &Path) -> PathBuf {
    if cfg!(windows) {
        root.join(".venv").join("Scripts").join("python.exe")
    } else {
        root.join(".venv").join("bin").join("python")
    }
}

fn openmontage_check(root: &Path) -> &'static str {
    if !root.is_dir() {
        return "not_installed";
    }
    if !venv_python(root).is_file() {
        return "missing_venv";
    }
    // Lightweight marker of a real clone.
    if root.join("pyproject.toml").is_file() || root.join("README.md").is_file() {
        return "ready";
    }
    "incomplete"
}

fn resolve_unity_bin() -> Option<PathBuf> {
    if let Ok(p) = std::env::var("UNITY_CLI") {
        let pb = PathBuf::from(p);
        if pb.is_file() {
            return Some(pb);
        }
    }
    if let Some(p) = which("unity") {
        return Some(p);
    }
    if let Some(p) = which("unity.exe") {
        return Some(p);
    }
    // Windows Unity Hub CLI default.
    if let Some(local) = std::env::var_os("LOCALAPPDATA") {
        let candidate = PathBuf::from(local).join("Unity").join("bin").join("unity.exe");
        if candidate.is_file() {
            return Some(candidate);
        }
    }
    None
}

fn which(name: &str) -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    for dir in std::env::split_paths(&path) {
        let candidate = dir.join(name);
        if candidate.is_file() {
            return Some(candidate);
        }
        #[cfg(windows)]
        {
            let with_exe = dir.join(format!("{name}.exe"));
            if with_exe.is_file() {
                return Some(with_exe);
            }
            let with_cmd = dir.join(format!("{name}.cmd"));
            if with_cmd.is_file() {
                return Some(with_cmd);
            }
        }
    }
    None
}

fn timeout_from(args: &Value, default_secs: u64, max_secs: u64) -> Duration {
    let secs = args
        .get("timeout_secs")
        .and_then(|v| v.as_u64())
        .unwrap_or(default_secs)
        .clamp(1, max_secs);
    Duration::from_secs(secs)
}

fn run_capture(
    bin: &Path,
    args: &[String],
    cwd: Option<&Path>,
    timeout: Duration,
) -> std::result::Result<String, String> {
    let mut cmd = Command::new(bin);
    cmd.args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    if let Some(cwd) = cwd {
        cmd.current_dir(cwd);
    }
    configure_no_window(&mut cmd);
    let mut child = cmd.spawn().map_err(|e| format!("spawn {}: {e}", bin.display()))?;
    let started = std::time::Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(status)) => {
                let mut stdout = String::new();
                let mut stderr = String::new();
                if let Some(mut out) = child.stdout.take() {
                    use std::io::Read;
                    let _ = out.read_to_string(&mut stdout);
                }
                if let Some(mut err) = child.stderr.take() {
                    use std::io::Read;
                    let _ = err.read_to_string(&mut stderr);
                }
                let mut text = String::new();
                if !stdout.is_empty() {
                    text.push_str(&stdout);
                }
                if !stderr.is_empty() {
                    if !text.is_empty() {
                        text.push_str("\n--- stderr ---\n");
                    }
                    text.push_str(&stderr);
                }
                if text.is_empty() {
                    text = format!("(no output) exit={}", status);
                } else if !status.success() {
                    text.push_str(&format!("\n(exit {status})"));
                }
                // Cap tool result size for LLM context.
                const MAX: usize = 32_000;
                if text.len() > MAX {
                    let skip = text.len() - MAX;
                    text = format!("…[truncated {skip} bytes]…\n{}", &text[skip..]);
                }
                return Ok(text);
            }
            Ok(None) => {
                if started.elapsed() > timeout {
                    let _ = child.kill();
                    let _ = child.wait();
                    return Err(format!(
                        "timeout after {}s: {} {:?}",
                        timeout.as_secs(),
                        bin.display(),
                        args
                    ));
                }
                std::thread::sleep(Duration::from_millis(50));
            }
            Err(e) => return Err(format!("wait failed: {e}")),
        }
    }
}

fn configure_no_window(cmd: &mut Command) {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
    #[cfg(not(windows))]
    {
        let _ = cmd;
    }
}
