<div align="center">

# Bony Build

**Native desktop AI coding assistant + local multi-agent collab room** — switch between channels and a Coding Workspace inside one [Buzz Desktop](third_party/buzz/desktop), open local projects, and drive Grok over ACP, with a clear path for future Codex and Claude Code collaboration.

**Language:** [中文](README.md) · **English**

[Quick start](#quick-start) ·
[Features](#features) ·
[Buzz local collab room](#buzz-local-collab-room) ·
[Web monitor](#web-monitor) ·
[Models & providers](#models--providers) ·
[Architecture](#architecture) ·
[Upstream relationship](#upstream-relationship) ·
[Development](#development)

![Buzz Desktop local multi-agent room](docs/buzz-room-local-room.png)

</div>

---

## What is this

This repo unifies the desktop coding workspace and multi-agent room inside one Buzz Desktop:

1. **Coding Workspace**: click the code icon in a channel header, open a local project, and drive `grok agent stdio` over [ACP](https://agentclientprotocol.com/) for code exploration, edits, terminal work, and search.
2. **Local multi-agent room**: Grok coordinates ZeroClaw / Unity / OpenMontage / DocSmith specialists through serial `@` handoffs. The backend uses Rust + SQLite with no Docker / Postgres / Redis requirement.

Good fit if you want to:

- Use **multi-provider BYOK** (Qwen / Kimi / Zhipu / OpenAI-compatible, etc.) for day-to-day edits on your machine
- Manage local projects in Coding Workspace and bind each ACP session to the real project directory
- Switch smoothly between channel collaboration and coding tasks while retaining the Buzz theme and window behavior
- Grow from Grok today to a shared session model for Codex, Claude Code, and other coding agents
- Inspect architecture layers and per-commit feature impact with a local **Web monitor**
- Split work across specialized agents via `@` handoffs in the **Buzz room** instead of one agent doing everything

Typical uses: explain repo structure, dig into recent changes, add tests, summarize auth / architecture; or in the Buzz room, have ZeroClaw search, Unity drive the engine, and DocSmith produce docs.

**Bony Build is the repository and product name; Buzz Desktop is the sole desktop shell.** Agent / TUI runtime tracks open-source [`xai-org/grok-build`](https://github.com/xai-org/grok-build) (see [Upstream relationship](#upstream-relationship)). Repo: [`phuhao00/bony`](https://github.com/phuhao00/bony).

---

## Features

| Capability | Description |
|------------|-------------|
| Coding Workspace | Open a Codex-like coding surface inside a channel without launching a second app |
| Local projects | Native directory picker, recent projects, and removal; ACP sessions use the real project path |
| Session isolation | Switching projects releases the old session and starts a new one to prevent cwd leakage |
| Multi-agent path | Grok works today; UI and session layers expose one extension point for Codex and Claude Code |
| Buzz interaction | Smooth workspace/channel transitions with Buzz theme, title bar, and window behavior |
| Room collaboration | Grok coordinates ZeroClaw / Unity / OpenMontage / DocSmith with threads and progress feedback |
| Local backend | Rust + SQLite + in-process pubsub, one workspace and one root `target/` |
| Web monitor | Architecture layers, “how it works”, feature-impact matrix, commit impact timeline |

---

## Buzz local collab room

This monorepo also ships [Block/Buzz](third_party/buzz)'s local multi-agent collab room: **Grok** is the room coordinator, with **ZeroClaw** (search), **Unity** (game engine), **OpenMontage** (editing), and **DocSmith** (docs) as specialists collaborating in a shared channel via serial `@` handoffs — mid-turn tool status is visible, messages can carry emoji reactions, and threads open on demand for follow-ups.

The backend has been fully refactored for **single-instance, Docker-free** deployment: persistence is **SQLite** (WAL mode + a 30s busy timeout, so concurrent multi-agent writes wait instead of failing); pub/sub, rate limiting, and presence run **in-process** instead of Redis; semantic search is wired up to embedded **[LanceDB](https://github.com/lancedb/lancedb)**; object storage still goes through an optional S3-compatible bucket — it runs fine locally even unconfigured.

![Buzz room: Grok hands the Shenzhen weather query to ZeroClaw; the thread panel shows the full weather report](docs/buzz-room-local-room.png)

One-shot startup (whitelisted scripts only; always build with `cargo build -p <crate>`):

```powershell
powershell -File .\scripts\buzz-room\start-room-stack.ps1 -SkipBuild   # relay + in-process pubsub + SQLite
powershell -File .\scripts\buzz-room\start-desktop.ps1                 # Buzz Desktop
powershell -File .\scripts\buzz-room\stop-room-stack.ps1               # stop everything
```

Policy and architecture detail: [`docs/buzz-room-collab.md`](docs/buzz-room-collab.md), [`third_party/buzz/README.md`](third_party/buzz/README.md), [`scripts/buzz-room`](scripts/buzz-room).

---

## Web monitor

Local dashboard for **architecture**, end-to-end “how it works”, and **per-change impact**:

```powershell
cargo run -p bony-monitor -- --bind 127.0.0.1:8787
# open http://127.0.0.1:8787
```

Capabilities:

- **Feature-impact matrix**: chat, model switch, auth, multi-provider, tools, permissions, ACP session, workspace, TUI, monitor, docs, …
- **Multi-axis scoring**: UX / capability / security / stability / compatibility / performance / DX / docs
- **How it works**: layer and turn-flow notes (with architecture diagrams)
- Per-commit **user impact** + **suggested verification checklist**
- Commit messages may include `Impact:` / `改进:` / `Risk:` / `风险:`

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
3. **Credentials** (either)  
   - Configure BYOK models + env vars in `%USERPROFILE%\.grok\config.toml` (recommended)  
   - Or `grok login` / `XAI_API_KEY`

### Launch the desktop app

```powershell
# Build and run from the repository root, sharing the root target/
cargo build -p buzz-desktop
powershell -File .\scripts\buzz-room\start-desktop.ps1
```

After startup, enter a channel, click the code icon in the header to open Coding Workspace, then choose a local project directory.

On Windows, **os error 4551** (Smart App Control) usually means build from a trusted terminal or disable SAC.

### Terminal TUI

This repo still ships the full `grok` TUI / agent sources:

```powershell
$env:CARGO_TARGET_DIR = "$PWD\target"
cargo run -p xai-grok-pager-bin
```

Official prebuilt install:

```powershell
irm https://x.ai/cli/install.ps1 | iex
```

---

## Models & providers

Model catalog and defaults come from `%USERPROFILE%\.grok\config.toml`. After launch, click the **model name** to switch; the choice is written to `[models] default`.

You can also **edit config.toml** in the picker. Example (Qwen / DashScope):

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

Verified providers:

| Provider | Typical `base_url` | Env var |
|----------|--------------------|---------|
| Qwen | `https://dashscope.aliyuncs.com/compatible-mode/v1` | `DASHSCOPE_API_KEY` |
| Kimi / Moonshot | `https://api.moonshot.cn/v1` | `MOONSHOT_API_KEY` |
| Zhipu GLM | `https://open.bigmodel.cn/api/paas/v4` | `ZHIPUAI_API_KEY` |
| OpenAI-compatible | Any `/v1` endpoint | Custom `env_key` |

More protocols:  
[`crates/codegen/xai-grok-pager/docs/user-guide/11-custom-models.md`](crates/codegen/xai-grok-pager/docs/user-guide/11-custom-models.md)

Restart the desktop app after config or env changes. Use `grok models` to verify the catalog.

---

## Architecture

Overview (renders on GitHub):

```mermaid
flowchart TB
  UI["Buzz Desktop<br/>channels + Coding Workspace"]
  ACP["buzz-acp<br/>session pool and queue"]
  Agent["grok agent stdio<br/>MvpAgent / SessionActor"]
  Sample["Sampling · multi-backend"]
  Tools["Tools · terminal / files / search"]
  WS["Workspace / MCP / sub-agents"]
  Room["Local multi-agent room<br/>SQLite + in-process pubsub"]

  UI --> ACP --> Agent
  Agent --> Sample
  Agent --> Tools
  Agent --> WS
  UI --> Room
```

Layered view and a single turn:

![Architecture layers](docs/architecture-layers.png)

![Turn flow](docs/architecture-turn-flow.png)

- Desktop app: [`third_party/buzz/desktop`](third_party/buzz/desktop)
- ACP session layer: [`third_party/buzz/crates/buzz-acp`](third_party/buzz/crates/buzz-acp)
- Write-up: [`ARCHITECTURE.md`](ARCHITECTURE.md)

Buzz Desktop drives local coding-agent subprocesses over ACP; Rust owns project paths, sessions, and queueing.

---

## Upstream relationship

| Layer | Source |
|-------|--------|
| Agent / TUI / tool stack | Periodically aligned with [`xai-org/grok-build`](https://github.com/xai-org/grok-build) (`Synced from monorepo`) |
| Product shell | Fork-owned: Buzz Desktop integration, `bony-monitor`, branded and collaboration docs |
| Pin | Root [`SOURCE_REV`](SOURCE_REV) records the upstream monorepo sync point |

### Sync upstream

```powershell
git remote add upstream https://github.com/xai-org/grok-build.git   # once
git fetch upstream
git rebase upstream/main
# After history rewrite:
git push --force-with-lease origin main
```

Rollback tag (if still present locally): `backup/pre-upstream-sync`.

---

## Repo layout (summary)

| Path | Description |
|------|-------------|
| `third_party/buzz/desktop` | Sole desktop client, including channels and Coding Workspace |
| `third_party/buzz/crates/buzz-acp` | Coding-agent ACP session pool and queue |
| `crates/codegen/bony-monitor` | Architecture & change-impact Web monitor |
| `crates/codegen/xai-grok-shell` | Agent runtime, stdio / headless |
| `crates/codegen/xai-grok-pager*` | Official TUI (`grok`) |
| `crates/codegen/xai-grok-agent` / `*-tools` / `*-workspace` | Agent, tools, workspace |
| `crates/codegen/xai-acp-lib` | ACP stdio helpers (used by the desktop bridge) |
| `docs/` | Screenshots and architecture diagrams (incl. Buzz room) |
| `scripts/buzz-room/` | Local Buzz room: relay / Desktop / external agents |
| `third_party/buzz` | Buzz sources (in-tree workspace members) |
| `SOURCE_REV` | Upstream monorepo sync revision |

Full upstream docs remain in each crate and the [user guide](crates/codegen/xai-grok-pager/docs/user-guide/).

---

## Development

```powershell
$env:CARGO_TARGET_DIR = "$PWD\target"
$env:PROTOC = "$PWD\.tools\protoc\bin\protoc.exe"   # if you have protoc placed here
cargo check -p buzz-desktop -p bony-monitor
cargo test -p buzz-acp
cargo build -p buzz-desktop
```

Ignore local artifacts: `target/`, `.tools/`, `.local-dist/`, `*.log`.

## Docs & license

- Coding Workspace: [`third_party/buzz/desktop/src/features/channels/ui/CodingWorkspaceScreen.tsx`](third_party/buzz/desktop/src/features/channels/ui/CodingWorkspaceScreen.tsx)
- User guide: [`crates/codegen/xai-grok-pager/docs/user-guide/`](crates/codegen/xai-grok-pager/docs/user-guide/)
- Auth: [`02-authentication.md`](crates/codegen/xai-grok-pager/docs/user-guide/02-authentication.md)
- Custom models: [`11-custom-models.md`](crates/codegen/xai-grok-pager/docs/user-guide/11-custom-models.md)
- Upstream open-source repo: [`xai-org/grok-build`](https://github.com/xai-org/grok-build)

This repo includes agent / TUI sources synced from the SpaceXAI monorepo / `xai-org/grok-build`; Buzz Desktop is the sole desktop product layer. See root [`LICENSE`](LICENSE) and per-crate declarations.

---

## Acknowledgments

Agent runtime and `grok` CLI capabilities come from [SpaceXAI / Grok Build](https://x.ai/cli) and [`xai-org/grok-build`](https://github.com/xai-org/grok-build). Bony Build adds a local-project Coding Workspace, multi-agent room, and change observability inside Buzz Desktop.
