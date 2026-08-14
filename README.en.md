<div align="center">

# Bony

**A local-first desktop platform for AI coding and multi-agent collaboration** — open a local project, enter Coding Workspace, run coding agents, and collaborate in a shared room from one client.

Coding agents attach over [ACP](https://agentclientprotocol.com/) (Codex, Claude Code, or a custom runtime). The default room seat is **ZeroClaw** (research). Built with Rust, Tauri, and SQLite.

**Language:** [中文](README.md) · **English**

[Quick start](#quick-start) ·
[Features](#features) ·
[Technology stack](#technology-stack) ·
[Local multi-agent collaboration](#local-multi-agent-collaboration) ·
[Models & providers](#models--providers) ·
[Architecture](#architecture-and-runtime-flow) ·
[Development](#development)

[▶ Play the Bony desktop workspace demo](https://cdn.jsdelivr.net/gh/phuhao00/bony@main/docs/bony-desktop-demo.mp4)

</div>

---

## What is Bony

1. Pick a local project and open Coding Workspace in a channel.
2. Run an authorized ACP coding agent against the real project path.
3. Hand off in the room with `@Agent` (at most one `@` per message). Default research is `@ZeroClaw`.
4. The same catalog can add Codex, Claude Code, or a custom ACP runtime.

**The product and repository are named Bony.** Sources live at the repo root: `desktop/` (Tauri) and `crates/` (relay, ACP, data).

---

## Features

| Capability | Description |
|------------|-------------|
| Coding Workspace | In-channel coding surface: project, session, messages, terminal, search |
| Local projects | Native picker and recents; ACP `cwd` is the real directory |
| Session isolation | Switching projects switches ACP session/`cwd` |
| Agents | Default room seat ZeroClaw; Coding Workspace uses the ACP catalog |
| Room | Serial `@` handoff, threads, progress; tool denylist over prompt-only rules |
| Local backend | Rust + SQLite + in-process pubsub; one workspace, one root `target/` |

---

## Technology stack

Tauri 2 / Rust / Tokio for the shell; React / Vite for UI only; ACP via `buzz-acp`; Axum + WebSocket + Nostr for the room; SQLite WAL + in-process pubsub; FTS5 + LanceDB for search. `buzz-*` is a crate prefix; the product name is Bony.

---

## Local multi-agent collaboration

On launch, Desktop seeds **Local Room** and **ZeroClaw** (`seed_room_agents`). Seats that are no longer in the spec are archived on reconcile.

```powershell
powershell -File .\scripts\buzz-room\start-room-stack.ps1 -SkipBuild
powershell -File .\scripts\buzz-room\start-desktop.ps1
powershell -File .\scripts\buzz-room\stop-room-stack.ps1
```

See [`docs/buzz-room-collab.md`](docs/buzz-room-collab.md) and [`docs/PROJECT_STANDARDS.md`](docs/PROJECT_STANDARDS.md).

---

## Quick start

From the **repository root**:

```powershell
cargo build -p buzz-relay -p buzz-desktop
powershell -File .\scripts\buzz-room\start-room-stack.ps1 -SkipBuild
powershell -File .\scripts\buzz-room\start-desktop.ps1
```

ZeroClaw defaults to `~/.bony-build/zeroclaw/target/release/zeroclaw.exe` or `zeroclaw` on PATH.

---

## Models & providers

Model keys and catalogs belong to whichever ACP runtime you selected. Restart the desktop after changing env vars or that runtime's config.

---

## Architecture and runtime flow

Desktop (Tauri) talks to `buzz-relay` (SQLite + pubsub). `buzz-acp` turns room events into ACP sessions bound to the project path in the `coding-workspace-v1` marker. Hard stop: `BUZZ_ACP_DENY_TOOLS` → `reject_once`. Details: [`ARCHITECTURE.md`](ARCHITECTURE.md).

Room data defaults to `buzz.db` at the repo root (gitignored).

---

## Repo layout

| Path | Description |
|------|-------------|
| `desktop/` | Desktop app (`buzz-desktop`) |
| `crates/` | relay, acp, db, pubsub, search, economy |
| `desktop/src-tauri/prompts/` | Room seat copy |
| `docs/` | Standards and room collab |
| `scripts/buzz-room/` | Three launch/stop scripts only |
| `migrations/` | DB migrations |

---

## Development

[`docs/PROJECT_STANDARDS.md`](docs/PROJECT_STANDARDS.md) · [`AGENTS.md`](AGENTS.md)

```powershell
cargo check -p buzz-desktop
cargo test -p buzz-acp
cargo build -p buzz-desktop
```

---

## Docs & license

- [`docs/PROJECT_STANDARDS.md`](docs/PROJECT_STANDARDS.md)
- [`docs/buzz-room-collab.md`](docs/buzz-room-collab.md)
- [`ARCHITECTURE.md`](ARCHITECTURE.md)
- [`LICENSE`](LICENSE)
