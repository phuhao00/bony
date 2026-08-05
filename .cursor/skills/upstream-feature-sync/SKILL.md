---
name: upstream-feature-sync
description: >-
  Monitors ZeroClaw and OpenMontage upstream updates, rebases xai-org/grok-build
  into bony-build while preserving product-layer features, and reapplies managed
  overlays (e.g. Open-Meteo weather). Use when the user asks to 监测/sync/rebase
  ZeroClaw, OpenMontage, grok-build upstream, periodically merge external
  features, or keep bony-build customizations while adopting new upstream code.
---

# Upstream / partner feature sync

Bony Build sits on three external tracks. This skill defines **how to monitor
them**, **how to rebase**, and **what must never be lost**.

## Source map

| Track | Remote / URL | Role | Integration in this repo |
|-------|--------------|------|--------------------------|
| **Grok monorepo** | `upstream` → `https://github.com/xai-org/grok-build.git` | Agent/TUI/runtime | Rest of workspace (`xai-grok-*`) |
| **ZeroClaw** | `https://github.com/zeroclaw-labs/zeroclaw.git` (managed clone) | General ACP backend | `crates/codegen/bony-build/src/zeroclaw*.rs`, weather overlay |
| **OpenMontage** | `https://github.com/calesthio/OpenMontage.git` | Video skill + CLI | `crates/codegen/bony-build/src/openmontage.rs` |

Product shell (**keep ours**): `crates/codegen/bony-build/**`, `crates/codegen/bony-monitor/**`, branded README/ARCHITECTURE, desktop release workflow, route cards / intent / ACP bridges.

Pin file: root [`SOURCE_REV`](../../../SOURCE_REV) = last known `upstream/main` SHA after a successful grok-build rebase.

Managed install paths (machine-local, not in git):

- ZeroClaw: `~/.bony-build/zeroclaw` (`SOURCE_GIT_URL` in `zeroclaw.rs`)
- OpenMontage: user-chosen / default under usage dir (see `openmontage.rs`)

## Bony-owned customizations (do not drop)

When resolving conflicts or refreshing clones, **preserve or re-apply**:

### Always (desktop product)

- Intent router, route cards, ZC ACP bridge (`zeroclaw.rs`, `zeroclaw_bridge.rs`, timeline `Route` / `MessageSource::Zeroclaw`)
- Tool detail humanization (real newlines, not JSON-escaped `\n`)
- Session `session/load` timeout → new session (Grok ACP)
- OpenMontage / Bevy / Unity plugin UX strings and install flows in `app.rs` / i18n
- Any `Full Control` / permission UX for tools

### ZeroClaw overlays

- Agent config bootstrap: `custom.bonybuild` provider + `runtime_profile` + `agentic` + `native_tools`
- Vendored weather tool: `crates/codegen/bony-build/assets/zeroclaw_weather_tool.rs`  
  Written into managed tree before `cargo build` via `apply_managed_source_overrides` (Open-Meteo; Chinese cities e.g. 深圳 → China, not wttr.in → Hong Kong).
- After ZC source upgrade: if `weather_patch_is_stale`, rebuild managed binary (`cargo +stable build --release --bin zeroclaw`).

### OpenMontage overlays

- Skill injection / `OPENMONTAGE_ROOT` wiring and skill text in `openmontage.rs`
- Install stream UX (clone depth 1, deps, status polled into UI)
- Do **not** replace OpenMontage with MCP-style tool names; stay CLI/pipeline skill style

---

## Periodic monitor checklist

Run when user asks to 监测 / check updates / weekly sync. Report in 中文 unless they want English.

### A) Grok-build (`upstream/main`)

```powershell
git remote add upstream https://github.com/xai-org/grok-build.git   # once
git fetch upstream main
git log --oneline -5 upstream/main
git rev-list --left-right --count HEAD...upstream/main
git log --oneline HEAD..upstream/main | Select-Object -First 15
```

### B) ZeroClaw (fork + optional parent)

```powershell
# Managed tree if present
cd $env:USERPROFILE\.bony-build\zeroclaw
git remote set-url origin https://github.com/zeroclaw-labs/zeroclaw.git   # if needed
git fetch origin
git log --oneline -5 HEAD
git log --oneline HEAD..origin/master   # default branch is master
```

Also note URL used by the app: `zeroclaw::SOURCE_GIT_URL` → `zeroclaw-labs/zeroclaw`.

### C) OpenMontage

```powershell
# If already installed (typical root from OpenMontageState)
git -C <OPENMONTAGE_ROOT> fetch origin
git -C <OPENMONTAGE_ROOT> log --oneline -5 HEAD
git -C <OPENMONTAGE_ROOT> log --oneline HEAD..origin/main
# Or shallow check without local clone:
gh api repos/calesthio/OpenMontage/commits/main --jq ".sha,.commit.message"
```

**Report template:**

```markdown
## 上游监测
| 源 | 本地/钉 | 远端 HEAD | 落后 N | 是否建议 rebase |
|----|---------|-----------|--------|-----------------|
| xai-org/grok-build | SOURCE_REV / main | ... | ... | ... |
| zeroclaw-labs/zeroclaw | ~/.bony-build/zeroclaw | ... | ... | ... |
| calesthio/OpenMontage | install root / n/a | ... | ... | ... |

## 风险点
- (list files likely to conflict: Cargo.lock, app.rs bridges, weather overlay)

## 建议顺序
1. ...
```

Do **not** rebase until the user (or an explicit “帮我同步”) confirms — monitoring alone is read-only.

---

## Rebase workflows

### 1) Monorepo: rebase bony-build onto `xai-org/grok-build:main`

Preconditions: clean working tree.

```powershell
git status   # must be clean
git fetch upstream main
git rebase upstream/main
```

**Conflicts:**

| File | Strategy |
|------|----------|
| `Cargo.lock` | After `Cargo.toml` is resolved: `cargo generate-lockfile` (or `cargo check -p bony-build`), then `git add Cargo.lock` |
| `Cargo.toml` members | Keep **both** `bony-build` / `bony-monitor` and any new upstream members |
| `crates/codegen/bony-build/**` | Prefer **ours** product changes; re-apply upstream useful hunks manually when needed |
| Pure `xai-grok-*` | Prefer **upstream** unless we have intentional patches documented in commit messages |

Continue:

```powershell
$env:GIT_EDITOR = "true"
git rebase --continue   # repeat until done
cargo check -p bony-build
```

**After success:**

1. Update root `SOURCE_REV` to `git rev-parse upstream/main`
2. Verify ZeroClaw / OpenMontage integration still present (`zeroclaw.rs`, `openmontage.rs`, weather asset)
3. History rewrite → push only with user OK:

```powershell
git push --force-with-lease origin main
# This repo often needs SSH as phuhao00 if gh HTTPS is phuhao000 (read-only):
# git push --force-with-lease git@github.com:phuhao00/bony-build.git HEAD:main
```

Never force-push `main` without user request. Prefer `--force-with-lease` over `--force`.

### 2) ZeroClaw: refresh managed clone + rebuild

Goal: take fork `main` updates, keep bony overlays.

```powershell
$dir = "$env:USERPROFILE\.bony-build\zeroclaw"
git -C $dir fetch origin
git -C $dir pull --ff-only origin master   # default branch is master
# Overlay is reapplied by bony-build on ensure_started/build; or:
# copy assets/zeroclaw_weather_tool.rs → crates/zeroclaw-tools/src/weather_tool.rs
$env:RUSTUP_TOOLCHAIN = "stable"
cargo +stable build --release --bin zeroclaw --manifest-path "$dir/Cargo.toml"
```

If comparing against an older local clone that still points at a personal fork:  
`git remote set-url origin https://github.com/zeroclaw-labs/zeroclaw.git` first.

App config: ensure `~/.zeroclaw/config.toml` still has `runtime_profile`, `agentic=true`, `native_tools=true` for `bonybuild`.

### 3) OpenMontage: pull feature updates, keep skill wiring

```powershell
git -C <OPENMONTAGE_ROOT> fetch origin
git -C <OPENMONTAGE_ROOT> pull --ff-only origin main
# Re-run deps if package manifests changed (same install path as UI "安装")
```

Then verify in-app:

- Skill still enabled / path unchanged
- Agent skill prompt text still documents CLI usage under `OPENMONTAGE_ROOT`
- No regression in bony-build `openmontage.rs` install/status UI

Do **not** wholesale replace `openmontage.rs` from upstream—there is no OpenMontage “product shell” in their repo; only the pipeline git tree is upstream.

---

## “Keep our features” conflict decision tree

1. **File only under `bony-build` / `bony-monitor`** → keep ours; cherry-pick upstream ideas if valuable.
2. **Shared runtime `xai-grok-*`** → prefer upstream; re-test ACP/desktop after.
3. **Lockfile only** → regenerate after merge, never hand-edit for long.
4. **Managed ZC weather tool** → always re-apply embed after any ZC `git pull` that touches `weather_tool.rs`.
5. **New upstream capability useful to general chat** → wire through ZeroClaw intent/ACP or Grok tools with route cards; do not hide behind silent degrade.

---

## Cadence recommendation

| Cadence | Action |
|---------|--------|
| Weekly (or when user says 监测) | Read-only monitor (sections A–C), report table |
| When report shows useful upstream deltas | User-confirmed rebase/pull per workflow 1–3 |
| After any monorepo rebase | `SOURCE_REV` update + `cargo check -p bony-build` + smoke OpenMontage/ZC if time |
| After ZC rebuild | One 深圳天气 turn: route=ZeroClaw, place=Shenzhen China |

---

## Safety

- No rebase/force-push on dirty tree.
- No `git push --force` to main without explicit ask; use `--force-with-lease`.
- PowerShell: use `;` not `&&`; no interactive `git rebase -i`.
- Do not commit `.env`, API keys, or `~/.zeroclaw` config contents.
- Skip lockfile hand-merge thrash: regenerate with cargo.

## Related paths

- Desktop: `crates/codegen/bony-build/src/{zeroclaw,zeroclaw_bridge,openmontage,agent_bridge,app}.rs`
- Weather overlay: `crates/codegen/bony-build/assets/zeroclaw_weather_tool.rs`
- Docs: root `README.md` / `README.en.md` “Upstream relationship”
