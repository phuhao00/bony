<div align="center">

# Bony Build

**Native desktop AI coding assistant** — chat to edit code, isolate tasks in Git worktrees, session plugins (Unity CLI), and change observability.

**Language:** [中文](README.md) · **English**

[Prebuilt binaries](#prebuilt-binaries) ·
[Quick start](#quick-start) ·
[Features](#features) ·
[Plugins & Unity](#plugins--unity) ·
[Web monitor](#web-monitor) ·
[Models & providers](#models--providers) ·
[Architecture](#architecture) ·
[Upstream relationship](#upstream-relationship) ·
[Development](#development)

![Bony Build desktop](docs/bony-build-desktop-2026-07-25.png)

</div>

---

## What is this

**Bony Build** is a native desktop client (Rust / egui, currently `v0.1.2`). It drives a local `grok agent stdio` process over [ACP](https://agentclientprotocol.com/) and does **conversational coding** in the workspace you choose—explore code, edit files, run the terminal and search tools—not just a chat window.

Good fit if you want to:

- Use **multi-provider BYOK** (Qwen / Kimi / Zhipu / OpenAI-compatible, etc.) for day-to-day edits on your machine
- Keep work **isolated per task with Git worktrees**, with sidebar conversations grouped **by project**
- Drive Unity with a **local CLI loop** (probe, Play, Pipeline) without hanging installs through the Agent
- Inspect architecture layers and per-commit feature impact with a local **Web monitor**

Typical uses: explain repo structure, dig into recent changes, add tests, summarize auth / architecture. The Agent calls terminal, file-edit, and search tools. Per-task permissions: read-only / ask / allow edits / full control; or require manual approval globally with `--ask-permissions`.

**The product brand and desktop shell are Bony Build.** Agent / TUI runtime tracks open-source [`xai-org/grok-build`](https://github.com/xai-org/grok-build) (see [Upstream relationship](#upstream-relationship)). Repo: [`phuhao00/bony-build`](https://github.com/phuhao00/bony-build).

---

## Prebuilt binaries

GitHub Releases ship desktop zips (you still need a local `grok` CLI):

- [**Bony Build v0.1.2**](https://github.com/phuhao00/bony-build/releases/tag/v0.1.2)
  - `bony-build-v0.1.2-windows-x86_64.zip`
  - `bony-build-v0.1.2-macos-aarch64.zip`
  - `bony-build-v0.1.2-macos-x86_64.zip`

Built by [`.github/workflows/release-desktop.yml`](.github/workflows/release-desktop.yml) on `v*` tags (`release-dist` profile).

---

## Features

| Capability | Description |
|------------|-------------|
| Chat workspace | Codex-style sidebar + timeline; Markdown, user bubbles / assistant cards, inline tool results |
| Grouped by project | Sidebar groups conversations by project; delete / archive; suggested titles |
| Tasks & worktrees | Create / switch tasks; optional isolated worktrees and branches |
| Session plugins | Composer **`+`**: attach files, enable Unity, manage plugins; dismissible chips (session-scoped, not sticky across relaunch) |
| Permission modes | Per task: read-only / ask / allow edits / full control; CLI supports `--ask-permissions` |
| Quick starts | One-click common tasks (explain structure, find bugs, add tests, summarize auth, …) |
| Model switching | Click the model name in the composer; choice is written to `~/.grok/config.toml` as default |
| Multi-provider | Kimi / Qwen / Zhipu / OpenAI-compatible / Anthropic Messages, etc. (BYOK) |
| Unity control | **Plugins** page for install & project binding; in-chat Unity chip + shortcuts / `/unity`; **local CLI, not Agent** |
| Usage stats | Turn and token usage panel (line / bar charts) |
| CJK UI | System Chinese fonts (e.g. Microsoft YaHei) to avoid tofu glyphs |
| Shortcuts | **Enter** to send, **Shift+Enter** for newline; Send stays readable when disabled, with hover reasons |
| Web monitor | Architecture layers, “how it works”, feature-impact matrix, commit impact timeline |

Primary sidebar nav today: **New task** · **Chat** · **Plugins**. Sites / PRs / schedules remain future placeholders.

---

## Plugins & Unity

### Plugin model

1. Sidebar **Plugins**: enable / disable Unity control, open settings or docs  
2. Composer **`+`**: attach files or Unity for this conversation; dismissible context chips appear  
3. With Unity on, the composer shows quiet shortcuts (save scene, refresh assets, Play, …) and Docs

In-chat Unity actions use the **local Unity CLI**, not the grok Agent, so `unity pipeline install` does not hang inside a worktree.

### Recommended setup

Install CLI → re-detect → confirm a project root that contains `Assets` → install Pipeline → open the editor and probe → run the loop. Default Windows CLI: `%LOCALAPPDATA%\Unity\bin\unity.exe`.

```powershell
$env:UNITY_CLI_CHANNEL='beta'; irm https://public-cdn.cloud.unity3d.com/hub/prod/cli/install.ps1 | iex
```

More detail (scaffolds, NPC AI, slash commands): [`crates/codegen/bony-build/README.md`](crates/codegen/bony-build/README.md).

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
# Dev: build and run
powershell -ExecutionPolicy Bypass -File .\scripts\run-desktop.ps1

# Clean relaunch: kill old processes → release build → start
powershell -ExecutionPolicy Bypass -File .\scripts\run-bony-build.ps1

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

You can also **Edit config.toml** in the picker. Example (Qwen / DashScope):

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

## Upstream relationship

| Layer | Source |
|-------|--------|
| Agent / TUI / tool stack | Periodically aligned with [`xai-org/grok-build`](https://github.com/xai-org/grok-build) (`Synced from monorepo`) |
| Product shell | Fork-owned: `bony-build`, `bony-monitor`, branded docs, desktop release workflow |
| Pin | Root [`SOURCE_REV`](SOURCE_REV) records the upstream monorepo sync point |

Integration approach: take upstream `main` as the base, then reapply the Bony product layer (histories do not share a common ancestor, so a plain `git rebase` is not possible). Rollback tag: `backup/pre-upstream-sync`.

---

## Repo layout (summary)

| Path | Description |
|------|-------------|
| `crates/codegen/bony-build` | Bony Build desktop client (Unity / plugin UX) |
| `crates/codegen/bony-monitor` | Architecture & change-impact Web monitor |
| `crates/codegen/xai-grok-shell` | Agent runtime, stdio / headless |
| `crates/codegen/xai-grok-pager*` | Official TUI (`grok`) |
| `crates/codegen/xai-grok-agent` / `*-tools` / `*-workspace` | Agent, tools, workspace |
| `crates/codegen/xai-acp-lib` | ACP stdio helpers (used by the desktop bridge) |
| `scripts/run-desktop.ps1` | Desktop build & run |
| `scripts/run-bony-build.ps1` | Kill old processes + release build + launch |
| `scripts/run-monitor.ps1` | Start Web monitor (default :8787) |
| `scripts/sync-monitor-catalog.ps1` | Sync monitor capability catalog |
| `scripts/run-dev.ps1` | TUI dev launch |
| `.github/workflows/release-desktop.yml` | Multi-platform desktop zip release |
| `docs/` | Screenshots and architecture diagrams |
| `SOURCE_REV` | Upstream monorepo sync revision |

Full upstream docs remain in each crate and the [user guide](crates/codegen/xai-grok-pager/docs/user-guide/).

---

## Development

```powershell
$env:CARGO_TARGET_DIR = "$PWD\target"
$env:PROTOC = "$PWD\.tools\protoc\bin\protoc.exe"   # if you have protoc placed here
cargo check -p bony-build -p bony-monitor
cargo build -p bony-build --profile release-dist
cargo run -p bony-build -- --cwd $PWD
```

Ignore local artifacts: `target/`, `.tools/`, `*.log`, and a local `Bony Build.exe`.

To cut a release: push an annotated tag (e.g. `v0.1.2`) to trigger the desktop workflow, or `workflow_dispatch` with an existing tag.

---

## Docs & license

- User guide: [`crates/codegen/xai-grok-pager/docs/user-guide/`](crates/codegen/xai-grok-pager/docs/user-guide/)
- Auth: [`02-authentication.md`](crates/codegen/xai-grok-pager/docs/user-guide/02-authentication.md)
- Custom models: [`11-custom-models.md`](crates/codegen/xai-grok-pager/docs/user-guide/11-custom-models.md)
- Upstream open-source repo: [`xai-org/grok-build`](https://github.com/xai-org/grok-build)

This repo includes agent / TUI sources synced from the SpaceXAI monorepo / `xai-org/grok-build`; the desktop product layer is Bony Build. See root [`LICENSE`](LICENSE) and per-crate declarations.

---

## Acknowledgments

Agent runtime and `grok` CLI capabilities come from [SpaceXAI / Grok Build](https://x.ai/cli) and [`xai-org/grok-build`](https://github.com/xai-org/grok-build). Bony Build adds a multi-provider desktop experience, task / worktree workflows, session plugins (Unity), and change observability on top.
