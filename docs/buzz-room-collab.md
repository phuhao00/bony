# Buzz Room Collaboration (Grok lead)

Grok is the **room lead**. Buzz is the **shared office**. Specialists: ZeroClaw, Unity, OpenMontage.

Buzz source lives **in this monorepo tree** at `third_party/buzz` (ordinary directory — not a git submodule).
All Buzz crates and `buzz-desktop` are members of the **root** `Cargo.toml` workspace:
one `cargo build`, one `Cargo.lock`, one `target/` at the repo root.

## Layout

```
bony-build/
  Cargo.toml                     # single workspace (Grok CLI + Buzz + Desktop)
  target/                        # single Cargo target dir
  third_party/buzz/              # Buzz sources (in-tree)
  integrations/buzz/patches/     # historical Grok deltas (if still used)
  scripts/buzz-room/             # setup / infra / agents
  crates/.../bony-room-tools-mcp
```

## Quick start (from bony-build root only)

```powershell
# Sidecar / MCP tools once
powershell -File .\scripts\buzz-room\build-tools.ps1

# Compose + migrate + relay + Grok (skip rebuild if already built)
powershell -File .\scripts\buzz-room\start-room-stack.ps1 -SkipBuild

# Stop harnesses (Docker left running)
powershell -File .\scripts\buzz-room\stop-room-stack.ps1
```

## Buzz Desktop

| When | Command |
|------|---------|
| **Build** (from monorepo root workspace) | `powershell -File .\scripts\buzz-room\build-desktop.ps1` |
| **Daily launch (fast)** | `powershell -File .\scripts\buzz-room\start-desktop.ps1` |
| Same with cargo, no scripts | `cargo build -p buzz-desktop` then start Vite + exe |

Notes:

- **One** cache: repo-root `target/` (not `third_party/buzz/target` or `desktop/src-tauri/target`).
- Default Desktop features include **local-stt** (sherpa-onnx via **shared DLLs** — no static-lib LNK2019).
- TTS uses `ort` with `load-dynamic` (onnxruntime DLL at runtime).
- Pass `-NoLocalStt` only if you intentionally want a STT-free binary.
- CMake + VS 2022 C++ still required for Opus (and other native pieces).

## Clean target bloat

| Action | Command |
|--------|---------|
| Delete PDB + incremental (keep executables / most rlib) | `powershell -File .\scripts\buzz-room\clean-target-bloat.ps1` |
| Wipe all caches (next build full recompile) | `...\clean-target-bloat.ps1 -Nuclear` |

## 让 Desktop 能「看到」Grok / ZeroClaw / Unity / OpenMontage

它们不是编译进侧边栏的固定入口，而是 **外部 `buzz-acp` + 本机密钥**。Agents 页读的是本地 `managed-agents.json`，目录还查 relay 的 **kind:10100**。进程在线但未注册时，界面列表为空。

```powershell
# Relay 已启动、密钥已 mint 后：
powershell -File .\scripts\buzz-room\register-room-agents.ps1

# 再开 Desktop（需已加入 community: ws://localhost:3000）
powershell -File .\scripts\buzz-room\start-desktop.ps1
```

脚本会：发布 kind:0 / kind:10100 显示名、创建/复用 open 频道 `Local Room`、把四套身份写入 Desktop 的 managed-agents（`%AppData%\xyz.block.buzz.app\agents`）。之后重开 Agents 页即可看到卡片。频道成员栏要看到 agent 徽章，需加入 `Local Room`。

## Agent 约束

编译/启动 Buzz、Desktop 时遵循：`.cursor/skills/buzz-room-build-gate/SKILL.md`

只允许 `scripts/buzz-room/*` 白名单入口或根目录 `cargo -p …`；禁止再建第二套 `target`、禁止无确认 `cargo clean`。

## Policy

- Grok: `subscribe=all`
- Specialists: `subscribe=mentions`
- Permission: `accept-edits`
