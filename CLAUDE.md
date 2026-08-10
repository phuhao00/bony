# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## 项目定位

Bony：本地优先的桌面 AI 编程 + 多 Agent 协作平台。一个 Tauri 2 桌面客户端（频道 / Coding Workspace），一个本地房间栈（`buzz-relay` + SQLite + 进程内 pubsub + LanceDB），以及经 ACP 接入的 Coding Agent 运行面（Grok，对齐上游 `xai-org/grok-build`；为 Codex / Claude Code 保留统一扩展点）。

权威规范（本文件只是摘要与导航，冲突时以下列为准）：

1. 用户当次明确指令
2. `docs/PROJECT_STANDARDS.md` + `.cursor/rules/*.mdc`（强制规则，`alwaysApply`）
3. `AGENTS.md`、`docs/AGENT_COLLABORATION.md`、`docs/buzz-room-*.md`、`.cursor/skills/*/SKILL.md`

## 硬性约束（违反即返工）

- **新实现只写 Rust**（根 workspace crate）。禁止新增 Python / Node / Go / bash 业务程序；桌面端既有 TS 只承载界面，不扩业务/编排（编排、seed、密钥、agent 生命周期进 Tauri Rust / `buzz-*` crate）。
- **除启动脚本外禁止脚本**。白名单只有三个入口（仓库根执行）：`scripts/buzz-room/start-room-stack.ps1`、`start-desktop.ps1`、`stop-room-stack.ps1`。编译、测试、seed、注册、清理一律 `cargo -p` 或 Rust 代码；禁止新增或调用 `build-*.ps1` / `mint-*` / `register-*` / `clean-*` / `setup-*` 等脚本。
- **单一 Cargo workspace / 单一根 `target/` / 单一 `Cargo.lock`**。`third_party/buzz` 是 in-tree 普通目录（非 submodule），其 crates 与 `buzz-desktop` 都是根 workspace 成员。禁止第二套 target / 裸 `tauri dev` 另起缓存；**禁止无确认 `cargo clean`**；有可用 exe 不盲目重编。
- **复用优先**：写前搜同位逻辑；一次真相（策略/常量/校验单点定义）；入口薄（command / bin / UI）→ 领域 crate 厚；≥2 处重复才抽象，禁止为一处调用铺空框架。
- **UTF-8**：文件内容一律用编辑工具写入，**禁止** PowerShell `Set-Content` / `Out-File` / 重定向写正文（真实事故：GBK 写坏 `buzz-relay/src/event.rs`）。
- **clippy 禁用方法**（见根 `clippy.toml`）：`std::fs::canonicalize` / `Path::canonicalize` / `tokio::fs::canonicalize` → 用 `dunce::canonicalize`（Windows verbatim `\\?\` 路径问题）；`std/tokio::process::Command::spawn` → 用 `xai_tty_utils::ProcessScope::enroll` 登记子进程。
- 不主动 commit / push；一个文件 / 权威实现点同时只允许一个写入者；同一根 `target/` 不并行跑多个 Cargo 任务。
- 与用户沟通用**中文**；代码与公开 API 标识符保持英文。

## 常用命令

全部从**仓库根**执行（toolchain 由 `rust-toolchain.toml` 钉在 stable，勿随意升级）：

```powershell
# 编译 / 测试 / lint（-p 指定 crate，单测加 -- 过滤）
cargo build -p buzz-desktop
cargo test -p buzz-acp
cargo test -p buzz-acp -- some_test_name      # 单个测试
cargo clippy -p <crate>
cargo check -p <crate>                         # 快速验证优先用 check

# 跑 grok TUI
cargo run -p xai-grok-pager-bin

# 启动 / 停止（仅白名单脚本；有产物时必带 -SkipBuild）
powershell -File .\scripts\buzz-room\start-room-stack.ps1 -SkipBuild   # relay + pubsub + SQLite
powershell -File .\scripts\buzz-room\start-desktop.ps1                 # 桌面客户端
powershell -File .\scripts\buzz-room\stop-room-stack.ps1
```

注意事项：

- 根 `Cargo.toml` 是**自动生成**的（"Auto-generated workspace root"）——改依赖/成员请改各 crate 的 `Cargo.toml`。
- `.cargo/config.toml` 里 `[profile.dev]` 关掉了 debuginfo（控制 Windows PDB 体积），调试时不要预期有符号；`build.jobs = 16`。
- 需要 protoc 时：`$env:PROTOC = "$PWD\.tools\protoc\bin\protoc.exe"`（仓库带 `bin/protoc` dotslash 清单）。
- Windows 构建依赖 VS 2022 C++ + CMake（Opus 等原生件）。
- 桌面 TS 前端（仅改 UI 时）：`third_party/buzz/desktop`，pnpm@11.4.0；`pnpm typecheck`、`pnpm lint` / `pnpm check`（Biome）、`pnpm test`（node --test，`src/**/*.test.mjs`）、Playwright e2e（`pnpm test:e2e`）。日常启动不走 vite dev，走白名单 start 脚本。

## 架构总览

三个平面，详细版见 `ARCHITECTURE.md`（Agent/Session/工具细节）与 `README.md`（组件图与时序图）：

1. **Bony Desktop**（`third_party/buzz/desktop`，crate 名 `buzz-desktop`，Rust 层在 `src-tauri/`）：Tauri 2 壳 + React 渲染。频道与 Coding Workspace 的 UI；可信路径、原生对话框、Git、Keyring、managed process 生命周期都在 Tauri Rust 层（如 `room_seed`、coding-workspace 命令）。
2. **本地协作核心**：`buzz-relay`（Axum + WebSocket + Nostr 签名事件，SQLite/SQLx WAL + 30s busy timeout）、`buzz-pubsub`（进程内 broadcast / presence / 限流）、`buzz-search`（FTS5 + LanceDB）。数据默认落 `buzz.db`。
3. **Coding Agent 运行面**：`buzz-acp`（房间事件 → ACP 请求；队列、会话池、managed runtime 启停）→ ACP runtime（Grok / Codex / Claude Code 共用 managed-session 宿主协议）。Grok 路径 = `xai-grok-shell::SessionActor` 跑 采样 → 工具 → 再采样 循环；`xai-grok-agent::AgentBuilder` 装配提示词与工具；`ToolBridge` 落工具调用；`xai-grok-workspace` 管权限 / 沙箱 / checkpoint。

关键链路：Coding Workspace 请求里工程路径经 `coding-workspace-v1` 事件标记传给 `buzz-acp`，绑定为 ACP session 的受信 `cwd`；切换工程 = 切换会话边界，防止工作目录串线。房间协作防回归靠**硬拦**而非 prompt：`buzz-acp` 的 `BUZZ_ACP_DENY_TOOLS`（`acp.rs` 里对 `session/request_permission` 命中即 `reject_once`）。

### 代码落点

| 需求 | 落点 |
|------|------|
| 桌面 UI / 交互 | `third_party/buzz/desktop/src`（TS，仅界面） |
| 桌面本机能力、seed、agent 生命周期 | `third_party/buzz/desktop/src-tauri`（Rust） |
| Agent / 工具 / 会话 / TUI | `crates/codegen/xai-grok-*`（`-shell` 运行时、`-agent` 装配、`-tools` 工具、`-pager*` TUI） |
| 房间协议 / ACP 池 / pubsub / db | `third_party/buzz/crates/*` |
| Bony 自有 MCP 工具 | `crates/codegen/bony-room-tools-mcp`、`bony-docs-tools-mcp` |
| Agent 文案（非业务） | `scripts/buzz-room/prompts/*.md` |

### 与上游的关系

- `crates/codegen/xai-grok-*`、`crates/common/*`、`prod/mc/*` 定期与 [`xai-org/grok-build`](https://github.com/xai-org/grok-build) 对齐，同步点记录在根 `SOURCE_REV`。上游对齐代码保持小而清晰的 diff；Bony 产品层（桌面集成、`buzz-*` 房间改造、文档）独立演进。
- `third_party/buzz` 源自 Block/Buzz，已按本项目改造（本地 SQLite 栈替代 Postgres/Redis 等）；改动时注意其产品层归属。
- 房间运行时路由与角色契约：`docs/buzz-room-collab.md`、`docs/buzz-room-agent-orchestration-plan.md`、`.cursor/skills/buzz-agent-contracts/SKILL.md`。

## 数据与状态位置

| 数据 | 位置 |
|------|------|
| 房间数据 | SQLite `buzz.db`（WAL） |
| 最近项目 | Tauri app data 下 `coding-workspaces.json`（≤12 项） |
| Grok 配置 / 模型目录 / 记忆 | `%USERPROFILE%\.grok\`（BYOK 在 `config.toml`） |
| 密钥 | OS Keyring / 环境变量 |
| 实时状态 | `buzz-pubsub` 进程内（不落盘） |

## 交付自检（Done 判定）

可验证（给出用户可复现命令 / UI 路径）· 无脚本与多语言旁路 · 职责在正确 crate · 无重复实现 · `cargo check/test -p` 已过 · 不顺手扩 scope、不顺手大清理。
