<div align="center">

# Bony Build

**Native desktop AI coding assistant** — chat to edit code, isolate tasks in Git worktrees, and drive Unity via local CLI.

**Language:** [中文](README.md) · **English**

[Quick start](#quick-start) ·
[Features](#features) ·
[Unity control](#unity-control) ·
[Web monitor](#web-monitor) ·
[Models & providers](#models--providers) ·
[Architecture](#architecture) ·
[Development](#development)

![Bony Build desktop](docs/bony-build-desktop-2026-07-25.png)

</div>

---

## What is this

**Bony Build** is a native desktop client (Rust / egui). It drives a local `grok agent stdio` process over [ACP](https://agentclientprotocol.com/) and does **conversational coding** in the workspace you choose—explore code, edit files, run the terminal and search tools—not just a chat window.

Good fit if you want to:

- Use **multi-provider BYOK** (Qwen / Kimi / Zhipu / OpenAI-compatible, etc.) for day-to-day edits on your machine
- Keep work **isolated per task with Git worktrees** so the main checkout stays clean
- Work on Unity projects with a **local CLI visual loop** (probe editor, Play, Pipeline) without routing install flows through the Agent
- Inspect architecture layers and per-commit feature impact with a local **Web monitor**

Typical uses: explain repo structure, dig into recent changes, add tests, summarize auth / architecture. The Agent calls terminal, file-edit, and search tools. Per-task permissions support read-only / ask / allow edits / full control; you can also require manual approval globally with `--ask-permissions`.

The runtime reuses the SpaceXAI Grok agent stack; **the product brand and desktop shell are Bony Build**. Repo: [`phuhao00/bony-build`](https://github.com/phuhao00/bony-build). The same runtime can also run as the official `grok` TUI (see [Terminal TUI](#terminal-tui-optional) below).

---

## Features

| Capability | Description |
|------------|-------------|
| Chat workspace | Codex-style sidebar + timeline; Markdown, user bubbles / assistant cards, inline tool results |
| Projects & tasks | Recent projects, create / switch tasks; optional isolated worktrees and branches with reviewable state |
| Permission modes | Per task: read-only / ask / allow edits / full control; CLI also supports `--ask-permissions` |
| Quick starts | One-click common tasks (explain structure, find bugs, add tests, summarize auth, …) |
| Model switching | Click the model name in the header / composer; choice is written to `~/.grok/config.toml` as default |
| Multi-provider | Kimi / Qwen / Zhipu / OpenAI-compatible / Anthropic Messages, etc. (BYOK) |
| Unity control | Sidebar setup + in-chat `Unity` chip / `/unity`; uses **local CLI**, not the Agent |
| Usage stats | Turn and token usage panel (line / bar charts) |
| CJK UI | System Chinese fonts (e.g. Microsoft YaHei) to avoid tofu glyphs |
| Shortcuts | **Enter** to send, **Shift+Enter** for newline |
| Web monitor | Architecture layers, “how it works” flow, feature-impact matrix, commit impact timeline |

Some sidebar entries (plugins, sites, PRs, schedules, …) are still placeholders and will open up in later iterations.

---

## Unity control

Two entry points:

1. **Sidebar → Plugins → Unity** — install CLI / Pipeline, bind a project, button actions  
2. **Composer `+` → Unity control** — attach a dismissible Unity chip for this chat; you can also type “probe editor”, “enter Play”, or `/unity`

In-chat Unity actions use the **local Unity CLI**, not the grok Agent, so `unity pipeline install` does not hang inside a worktree.

Recommended flow: install CLI → re-detect → confirm a project root that contains `Assets` → install Pipeline → open the editor and probe → run the loop. Default Windows CLI: `%LOCALAPPDATA%\Unity\bin\unity.exe`.

```powershell
$env:UNITY_CLI_CHANNEL='beta'; irm https://public-cdn.cloud.unity3d.com/hub/prod/cli/install.ps1 | iex
```

More detail: [`crates/codegen/bony-build/README.md`](crates/codegen/bony-build/README.md).

---

## Web monitor

A local dashboard for **overall architecture**, end-to-end “how it works”, and **impact of each change**:

```powershell
powershell -ExecutionPolicy Bypass -File .\scripts\run-monitor.ps1
# Open http://127.0.0.1:8787
```

Capabilities:

- **Feature impact matrix**: chat, model switch, auth, multi-provider, tools, permissions, ACP sessions, workspace, TUI, monitor, docs, …
- **Multi-axis scoring**: UX / capability / security / stability / compatibility / performance / DX / docs
- **How it works**: layering and turn-flow notes (with architecture diagrams)
- Per-commit **user impact notes** + **suggested verification checklist**
- Commit messages can include `Impact:` / `改进:` / `Risk:` / `风险:`
- Directory hot-reload; sync the capability catalog with `scripts/sync-monitor-catalog.ps1`

Implementation: `crates/codegen/bony-monitor` (Axum).

---

## Quick start

### Dependencies

1. **Rust** (see [`rust-toolchain.toml`](rust-toolchain.toml))
2. **`grok` CLI** (agent subprocess)  
   ```powershell
   npm i -g @xai-official/grok
   grok --version
   ```
3. **Credentials** (pick one)  
   - Configure BYOK models in `%USERPROFILE%\.grok\config.toml` plus matching env vars (recommended)  
   - Or `grok login` / `XAI_API_KEY`

### Launch the desktop app

```powershell
# Recommended
powershell -ExecutionPolicy Bypass -File .\scripts\run-desktop.ps1

# Or
$env:CARGO_TARGET_DIR = "$PWD\target"
cargo run -p bony-build
```

Common flags:

```text
--cwd <path>        Session working directory (default: current directory)
--grok-bin <path>   Path to the grok binary
--ask-permissions   Require manual tool approval (default: auto-approve)
```

On Windows, if you hit **os error 4551** (Smart App Control), build from a trusted terminal or turn SAC off and retry.

### Terminal TUI (optional)

This repo still includes the full `grok` TUI / agent sources:

```powershell
powershell -ExecutionPolicy Bypass -File .\scripts\run-dev.ps1
# Or
cargo run -p xai-grok-pager-bin
```

Official prebuilt install:

```powershell
irm https://x.ai/cli/install.ps1 | iex
```

---

## Models & providers

The model catalog and defaults live in `%USERPROFILE%\.grok\config.toml`. After the desktop app starts, click the **model name** to switch; the choice is synced to `[models] default`.

You can also open **Edit config.toml** in the picker. Example (Qwen / DashScope):

```toml
[models]
default = "qwen-max"
stream_tool_calls = false

[model.qwen-max]
model = "qwen-max"
base_url = "https://dashscope.aliyuncs.com/compatible-mode/v1"
name = "Qwen Max"
env_key = "DASHSCOPE_API_KEY"
api_backend = "chat_completions"
context_window = 32768
```

Verified setups:

| Provider | Typical `base_url` | Env var |
|----------|--------------------|---------|
| Qwen (DashScope) | `https://dashscope.aliyuncs.com/compatible-mode/v1` | `DASHSCOPE_API_KEY` |
| Kimi / Moonshot | `https://api.moonshot.cn/v1` | `MOONSHOT_API_KEY` |
| Zhipu GLM | `https://open.bigmodel.cn/api/paas/v4` | `ZHIPUAI_API_KEY` |
| OpenAI-compatible | Any `/v1` endpoint | Custom `env_key` |

More protocols (Anthropic `messages`, Responses API, Ollama, …):  
[`crates/codegen/xai-grok-pager/docs/user-guide/11-custom-models.md`](crates/codegen/xai-grok-pager/docs/user-guide/11-custom-models.md)

Restart the desktop app after changing config or env vars. Use `grok models` to verify the active catalog.

---

## Architecture

```text
Bony Build (egui desktop shell)
        │  ACP JSON-RPC over stdio
        ▼
grok agent stdio  →  MvpAgent / SessionActor
        │
        ├─ Sampling (multi-backend)
        ├─ Tools (terminal / files / search …)
        └─ Workspace / MCP / sub-agents

Side path: Unity CLI (local process, not via ACP / Agent)
```

- Desktop crate: [`crates/codegen/bony-build`](crates/codegen/bony-build)
- Layers & turn flow: [`ARCHITECTURE.md`](ARCHITECTURE.md)
- Diagrams: [`docs/architecture-layers.png`](docs/architecture-layers.png), [`docs/architecture-turn-flow.png`](docs/architecture-turn-flow.png)

The desktop app does **not** embed the full agent runtime; it drives an installed `grok` subprocess. Unity control calls the local CLI directly.

---

## Repo layout (summary)

| Path | Description |
|------|-------------|
| `crates/codegen/bony-build` | Bony Build desktop client |
| `crates/codegen/bony-monitor` | Architecture & change-impact Web monitor |
| `crates/codegen/xai-grok-shell` | Agent runtime, stdio / headless |
| `crates/codegen/xai-grok-pager*` | Official TUI (`grok`) |
| `crates/codegen/xai-grok-agent` / `*-tools` / `*-workspace` | Agent, tools, workspace |
| `scripts/run-desktop.ps1` | One-shot build & run for desktop |
| `scripts/run-monitor.ps1` | Start Web monitor (default :8787) |
| `scripts/sync-monitor-catalog.ps1` | Sync monitor capability catalog |
| `scripts/run-dev.ps1` | TUI dev launch |
| `docs/` | Screenshots and architecture diagrams |

Full upstream docs remain in each crate and the [user guide](crates/codegen/xai-grok-pager/docs/user-guide/).

---

## Development

```powershell
$env:CARGO_TARGET_DIR = "$PWD\target"
$env:PROTOC = "$PWD\.tools\protoc\bin\protoc.exe"   # if you have protoc placed here
cargo build -p bony-build
cargo run -p bony-build -- --cwd $PWD
```

Ignore local artifacts: `target/`, `.tools/`, and `*.log` files.

---

## Docs & license

- User guide: [`crates/codegen/xai-grok-pager/docs/user-guide/`](crates/codegen/xai-grok-pager/docs/user-guide/)
- Auth: [`02-authentication.md`](crates/codegen/xai-grok-pager/docs/user-guide/02-authentication.md)
- Custom models: [`11-custom-models.md`](crates/codegen/xai-grok-pager/docs/user-guide/11-custom-models.md)

This repo includes agent / TUI sources synced from the SpaceXAI monorepo; the desktop product layer is Bony Build. See root [`LICENSE`](LICENSE) (if present) and per-crate declarations.

---

## Acknowledgments

Agent runtime and `grok` CLI capabilities come from the [SpaceXAI / Grok Build](https://x.ai/cli) ecosystem. Bony Build adds a multi-provider desktop experience, task / worktree workflows, local Unity control, and change observability on top.
