---
name: upstream-feature-sync
description: >-
  Safely rebases xai-org/grok-build into this repository while preserving the
  Buzz Desktop product shell and local multi-agent room (SQLite / in-process pubsub /
  LanceDB), and ZeroClaw / OpenMontage overlays. Use when the user asks to
  rebase/sync/merge upstream grok-build, 监测上游, periodically adopt monorepo
  updates, or keep product + Buzz features when rewriting against upstream/main.
---

# Upstream feature sync (grok-build rebase)

Bony sits on **xai-org/grok-build** plus product-only layers this fork owns.
Every rebase must **take upstream agent/TUI runtime** without erasing **Bony + Buzz**.

Pin: root [`SOURCE_REV`](../../../SOURCE_REV) = `git rev-parse upstream/main` **after a successful rebase**.

Remote (once):

```powershell
git remote add upstream https://github.com/xai-org/grok-build.git
```

---

## Hard rule: what must never die

### A) Product shell (prefer **ours** / commit being replayed)

| Path | Why |
|------|-----|
| `third_party/buzz/desktop/**` | sole Tauri desktop shell, Coding Workspace, local integration |
| `third_party/buzz/crates/buzz-acp/**` | shared ACP pool and coding-agent sessions |
| `crates/codegen/bony-monitor/**` | local impact / architecture web |
| `crates/codegen/bony-room-tools-mcp/**` | room helpers MCP |
| `crates/codegen/bony-docs-tools-mcp/**` | DocSmith tool MCP |
| `scripts/buzz-room/start-desktop.ps1` | product launch only |
| Root `README.md` / `README.en.md` / `ARCHITECTURE.md` | brand + Buzz + BYOK docs |
| `docs/PROJECT_STANDARDS.md`, `docs/AGENT_COLLABORATION.md`, `docs/buzz-room-*.md`, buzz screenshots | product norms |
| `.cursor/rules/**`, `.cursor/skills/**`, `AGENTS.md` | agent norms |

### B) Buzz local multi-agent room (prefer **ours** product commits)

Current architecture (do **not** reintroduce Docker/Postgres/Redis as required):

| Surface | Keep |
|---------|------|
| Tree | `third_party/buzz/**` as **in-tree workspace members** (not submodule) |
| Workspace | Root `Cargo.toml` members: all `third_party/buzz/crates/buzz-*` listed today |
| Launch only | `scripts/buzz-room/start-room-stack.ps1`, `start-desktop.ps1`, `stop-room-stack.ps1` |
| Persistence | SQLite via `buzz-db` (`sqlite_connect_options`: WAL + busy_timeout) |
| Messaging | In-process `buzz-pubsub` (no Redis for single-instance default) |
| Search | FTS5 + optional LanceDB (`buzz-search`) |
| Collab docs | `docs/buzz-room-collab.md`, orchestration plan, room prompts under `scripts/buzz-room/prompts/` |

If upstream ever adds files that name “Postgres-only relay” into **our** Buzz crates path: **reject** / re-apply SQLite cutover. Buzz is **not** upstream grok-build code; conflicts there almost always mean keep bony history.

### C) Partner overlays (prefer **ours**)

- ZeroClaw / OpenMontage: room prompts, managed install discovery, and `bony-room-tools-mcp`
- Coding agents: Buzz Desktop Coding Workspace, room seed contracts, and `buzz-acp`

### D) Prefer **upstream** only for pure runtime

`crates/codegen/xai-grok-*`, `xai-acp-lib`, other `xai-*` shared crates **without** bony-only patches documented in recent commits.

---

## Conflict decision tree (during `git rebase`)

| Path pattern | Pick |
|--------------|------|
| `Cargo.lock` only | **Regenerate**: `cargo generate-lockfile` then `git add Cargo.lock` (never long hand-merge) |
| `Cargo.toml` members / workspace deps | **Union**: keep all `bony-*` + all `third_party/buzz/crates/*` members + any **new** upstream members |
| `SOURCE_REV` | After rebase finishes: write `git rev-parse upstream/main`; mid-conflict temporary either side is OK |
| `bony-*`, `docs/`, root README, `.cursor/`, `scripts/buzz-room/`, `third_party/buzz/` | **`--theirs` during rebase** (replayed bony commit) if pure product; never discard Buzz SQLite/in-process stack |
| Pure `xai-grok-*` without our documented patch | **`--ours` during rebase** (= already-rebased upstream base) |
| Unsure | Abort with `git rebase --abort` and re-list paths; do not invent merges |

Note Git’s rebase vocabulary: **ours** = upstream branch tip being replayed onto; **theirs** = commit currently being applied (usually our product history).

---

## Pre-flight (must pass)

```powershell
git status   # clean only (commit or stash product WIP first)
git fetch upstream main
git rev-list --left-right --count HEAD...upstream/main
git log --oneline HEAD..upstream/main | Select-Object -First 20
git merge-base HEAD upstream/main
```

Create a rollback tag **before** touching history:

```powershell
$tag = "backup/pre-upstream-rebase-$(Get-Date -Format 'yyyyMMdd-HHmmss')"
git tag $tag HEAD
```

Do **not** start if dirty tree or user only asked to 监测 without apply.

---

## Rebase procedure

```powershell
$env:GIT_EDITOR = "true"
git rebase upstream/main
# On conflict: apply decision tree, then:
git add <resolved>
git rebase --continue   # until done (no interactive -i)
```

Abort anytime:

```powershell
git rebase --abort
# or reset to backup tag if already finished wrongly
# git reset --hard backup/pre-upstream-rebase-...
```

### After rebase succeeds

1. `SOURCE_REV` ← `git rev-parse upstream/main` (UTF-8, single line + newline).
2. If `Cargo.lock` touched mid-way at a half-workspace state: final clean `cargo generate-lockfile` on full tree.
3. Preserve check (paths must exist):

```text
crates/codegen/bony-monitor/src/main.rs
third_party/buzz/desktop/src-tauri/src/commands/coding_workspace.rs
third_party/buzz/desktop/src/features/channels/ui/CodingWorkspaceScreen.tsx
third_party/buzz/crates/buzz-acp/src/pool.rs
third_party/buzz/crates/buzz-relay/src/main.rs
third_party/buzz/crates/buzz-db/src/lib.rs
scripts/buzz-room/start-room-stack.ps1
docs/buzz-room-collab.md
```

4. Compile gate (minimum):

```powershell
$env:CARGO_TARGET_DIR = "$PWD\target"
cargo check -p buzz-desktop -p bony-monitor -p buzz-relay -p buzz-db
```

5. Optional smoke: room stack start only if user wants; do not invent extra scripts.
6. Push only when user explicitly asks (history rewrite):

```powershell
git push --force-with-lease origin main
# Prefer SSH origin if HTTPS token is another account
```

Never `git push --force` without lease unless user insists after understanding rollback tag.

---

## Monitor-only (read-only)

When user says 监测 / check updates **without** “帮我 rebase/同步”:

```powershell
git fetch upstream main
git rev-list --left-right --count HEAD...upstream/main
git log --oneline HEAD..upstream/main | Select-Object -First 15
```

Also optional ZeroClaw / OpenMontage (managed clones — not monorepo rebase):

```powershell
# ZeroClaw managed tree
git -C "$env:USERPROFILE\.bony-build\zeroclaw" fetch origin
git -C "$env:USERPROFILE\.bony-build\zeroclaw" log --oneline HEAD..origin/master | Select-Object -First 10
# OpenMontage
gh api repos/calesthio/OpenMontage/commits/main --jq ".sha,.commit.message"
```

Report template:

```markdown
## 上游监测
| 源 | 钉 / HEAD | 落后 N | 建议 |
|----|-----------|--------|------|
| xai-org/grok-build | SOURCE_REV vs upstream/main | … | rebase / wait |
| zeroclaw | managed clone | … | pull+overlay |
| OpenMontage | install root | … | pull skill tree |

## 保留项自检清单（若 rebase）
- Buzz Desktop / bony-monitor / buzz room SQLite 栈
- 启动白名单 scripts/buzz-room/start-*
- 不恢复 Docker/Postgres/Redis 为默认依赖
```

---

## Partner tracks (not the monorepo rebase)

### ZeroClaw managed clone

Keep room contracts and managed-install discovery after any ZeroClaw pull; rebuild the managed `zeroclaw` release binary when stale.

### OpenMontage

`git pull` only under the install root; preserve the room prompt and `bony-room-tools-mcp` integration.

---

## Safety / Windows

- Clean tree only; no `rebase -i`.
- PowerShell: `;` not `&&`; write source files with Write/StrReplace tools (UTF-8), not `Set-Content` for Chinese prose.
- Do not commit `.env`, `.local-dist/`, `target/`, keys under `scripts/buzz-room/keys/`.
- Single `target/` at repo root; never create a second workspace under `third_party/buzz`.
- If lockfile generation fails mid-rebase because a later commit’s crate path is not yet applied, resolve only after the missing path exists in a later commit; or take one side of lockfile and regenerate at the **end**.

## Cadence

| When | Action |
|------|--------|
| User: 监测 | Read-only table; no rewrite |
| User: rebase / 同步 grok-build | Full procedure + backup tag + checks |
| After success | `SOURCE_REV` + `cargo check` minimum set above |
| Push | Only on explicit user request + `--force-with-lease` |

## Rollback

```powershell
git rebase --abort   # if still in progress
git reset --hard backup/pre-upstream-rebase-<timestamp>
```

## Related

- Project norms: [`docs/PROJECT_STANDARDS.md`](../../../docs/PROJECT_STANDARDS.md), root `AGENTS.md`
- Room collab: [`docs/buzz-room-collab.md`](../../../docs/buzz-room-collab.md)
- Printable: [`checklist.md`](checklist.md), [`examples.md`](examples.md)
