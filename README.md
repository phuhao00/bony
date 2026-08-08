<div align="center">

# Bony

**本地优先的桌面 AI 编程与多 Agent 协作平台** — 在一个客户端里打开本地工程、进入 Coding Workspace、运行 Coding Agent，并在共享房间中协调多个专员协作。

当前通过 [ACP](https://agentclientprotocol.com/) 接入 Grok，并为 Codex、Claude Code 等 Coding Agent 提供统一扩展入口。Bony 使用 Rust、Tauri 与 SQLite 构建，面向本地工程与本机运行环境。

**语言:** **中文** · [English](README.en.md)

[快速开始](#快速开始) ·
[功能](#功能) ·
[本地多 Agent 协作](#本地多-agent-协作) ·
[Web 监控](#web-监控) ·
[模型与供应商](#模型与供应商) ·
[架构](#架构) ·
[贡献者](#贡献者) ·
[与上游关系](#与上游关系) ·
[开发](#开发)

![Bony 本地多 Agent 协作界面](docs/buzz-room-local-room.png)

</div>

---

## Bony 是什么

Bony 把桌面编程工作区与多 Agent 房间统一在一个客户端中，核心工作流是：

1. 选择本地工程目录，在频道内打开 Coding Workspace。
2. 通过 ACP 启动 Grok 会话，使用文件、终端和搜索能力完成编码任务。
3. 切回共享房间，由 Grok 通过 `@` 把检索、引擎、剪辑和文档任务交给专员。
4. 通过统一会话入口继续接入 Codex、Claude Code 等 Coding Agent。

**项目名称与产品名称都是 Bony。** `third_party/buzz` 存放 Bony 内嵌并改造的 Block/Buzz 客户端与房间基础代码；Agent / TUI 运行时对齐 [`xai-org/grok-build`](https://github.com/xai-org/grok-build)。仓库：[`phuhao00/bony`](https://github.com/phuhao00/bony)。

---

## 功能

| 能力 | 说明 |
|------|------|
| Coding Workspace | 在频道内打开类似 Codex 的编码界面，集成项目、会话、消息、终端与搜索 |
| 本地工程 | 原生目录选择器、最近项目、移除记录；ACP 会话使用工程真实路径 |
| 会话隔离 | 切换工程时释放旧会话并创建新会话，避免工作目录串线 |
| 多 Agent 演进 | Grok 当前可用；界面和会话层为 Codex、Claude Code 等保留统一扩展点 |
| Bony 桌面交互 | Coding Workspace 与频道丝滑切换，共享主题、标题栏和窗口行为 |
| 房间协作 | Grok 协调 ZeroClaw / Unity / OpenMontage / DocSmith，支持线程和状态反馈 |
| 本地后端 | Rust + SQLite + 进程内 pubsub，单 workspace、单根 `target/` |
| Web 监控 | 架构分层、「怎么工作」、功能影响矩阵与提交影响时间线 |

---

## 本地多 Agent 协作

Bony 的房间能力基于仓库内嵌并改造的 [Block/Buzz](third_party/buzz) 代码：**Grok** 担任房间协调员，**ZeroClaw**（检索）/ **Unity**（游戏引擎）/ **OpenMontage**（剪辑）/ **DocSmith**（文档产出）等专员在共享频道里按 `@` 串行交接协作——工具调用中途状态可见，消息可加表情反馈，还能按需开子线程深入追问。

本地后端使用 **SQLite** 持久化（WAL 模式 + 30s busy timeout），以进程内组件处理发布订阅、限流和在线状态，并用嵌入式 **[LanceDB](https://github.com/lancedb/lancedb)** 提供语义检索；需要附件存储时可配置 S3 兼容对象存储。

![Bony 房间：Grok 交接 ZeroClaw 查询深圳天气，右侧子线程展示完整天气播报](docs/buzz-room-local-room.png)

一键启动（仅限白名单脚本，编译一律走 `cargo build -p <crate>`）：

```powershell
powershell -File .\scripts\buzz-room\start-room-stack.ps1 -SkipBuild   # relay + 进程内 pubsub + SQLite
powershell -File .\scripts\buzz-room\start-desktop.ps1                 # Bony 桌面客户端
powershell -File .\scripts\buzz-room\stop-room-stack.ps1               # 停止
```

策略与架构细节见 [`docs/buzz-room-collab.md`](docs/buzz-room-collab.md)、[`third_party/buzz/README.md`](third_party/buzz/README.md)、[`scripts/buzz-room`](scripts/buzz-room)。

---

## Web 监控

本地仪表盘，查看 **整体架构**、端到端「怎么工作」，以及 **每一次改动带来的影响**：

```powershell
cargo run -p bony-monitor -- --bind 127.0.0.1:8787
# 浏览器打开 http://127.0.0.1:8787
```

能力：

- **功能影响矩阵**：对话、模型切换、登录认证、多供应商、工具执行、权限、会话 ACP、工作区、TUI、监控、文档等
- **多维度评估**：用户体验 / 功能能力 / 安全 / 稳定性 / 兼容性 / 性能 / 开发体验 / 文档
- **怎么工作**：分层与 turn 流程说明（配合架构图）
- 每次提交的**用户影响说明** + **建议验证清单**
- 支持在 commit message 写 `Impact:` / `改进:` / `Risk:` / `风险:`

实现：`crates/codegen/bony-monitor`（Axum）。

---

## 快速开始

### 依赖

1. **Rust**（见 [`rust-toolchain.toml`](rust-toolchain.toml)）
2. **`grok` CLI**（agent 子进程）  
   ```powershell
   npm i -g @xai-official/grok
   grok --version
   ```
3. **凭证**（任选其一）  
   - 在 `%USERPROFILE%\.grok\config.toml` 配置 BYOK 模型 + 对应环境变量（推荐）  
   - 或 `grok login` / `XAI_API_KEY`

### 启动桌面端

```powershell
# 编译与运行都从仓库根目录走 Cargo，共用根 target/
cargo build -p buzz-desktop
powershell -File .\scripts\buzz-room\start-desktop.ps1
```

启动后进入频道，点击标题栏右侧的代码图标打开 Coding Workspace，再选择本地工程目录。

### 也可使用终端 TUI

本仓库包含完整 `grok` TUI / agent 源码：

```powershell
$env:CARGO_TARGET_DIR = "$PWD\target"
cargo run -p xai-grok-pager-bin
```

官方预编译安装：

```powershell
irm https://x.ai/cli/install.ps1 | iex
```

---

## 模型与供应商

模型目录与默认值由 `%USERPROFILE%\.grok\config.toml` 决定。桌面端启动后可点 **模型名** 切换；选择结果会同步写入 `[models] default`。

也可在弹窗中 **编辑 config.toml**。示例（通义 Qwen / DashScope）：

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

已验证可配：

| 供应商 | 典型 `base_url` | 环境变量 |
|--------|-----------------|----------|
| 通义 Qwen | `https://dashscope.aliyuncs.com/compatible-mode/v1` | `DASHSCOPE_API_KEY` |
| Kimi / Moonshot | `https://api.moonshot.cn/v1` | `MOONSHOT_API_KEY` |
| 智谱 GLM | `https://open.bigmodel.cn/api/paas/v4` | `ZHIPUAI_API_KEY` |
| OpenAI 兼容 | 任意 `/v1` 端点 | 自定义 `env_key` |

更多协议见：  
[`crates/codegen/xai-grok-pager/docs/user-guide/11-custom-models.md`](crates/codegen/xai-grok-pager/docs/user-guide/11-custom-models.md)

配置或环境变量变更后请**重启桌面端**。可用 `grok models` 核对当前目录。

---

## 架构

概览（GitHub 可渲染）：

```mermaid
flowchart TB
  UI["Bony 桌面端<br/>频道 + Coding Workspace"]
  ACP["buzz-acp<br/>会话池与队列"]
  Agent["grok agent stdio<br/>MvpAgent / SessionActor"]
  Sample["采样 · 多 backend"]
  Tools["工具 · 终端 / 文件 / 搜索"]
  WS["Workspace / MCP / 子 agent"]
  Room["本地多 Agent 房间<br/>SQLite + 进程内 pubsub"]

  UI --> ACP --> Agent
  Agent --> Sample
  Agent --> Tools
  Agent --> WS
  UI --> Room
```

分层与一次 turn 流程：

![架构分层](docs/architecture-layers.png)

![Turn 流程](docs/architecture-turn-flow.png)

- 桌面应用：[`third_party/buzz/desktop`](third_party/buzz/desktop)
- ACP 会话层：[`third_party/buzz/crates/buzz-acp`](third_party/buzz/crates/buzz-acp)
- 文字说明：[`ARCHITECTURE.md`](ARCHITECTURE.md)

Bony 桌面端通过 ACP 驱动本机 Coding Agent 子进程；工程目录、会话和队列由 Rust 层管理。Rust crate 与目录使用 `buzz-*` 技术前缀。

---

## 贡献者

| 名称 | 角色 |
|------|------|
| [phuhao（@phuhao00）](https://github.com/phuhao00) | Bony 创建者、产品负责人和核心维护者 |
| [OpenAI Codex](https://github.com/apps/openai-codex) | Agentic 编程协作：设计、实现、测试与文档 |
| [Cursor Agent](https://github.com/cursoragent) | AI 编程协作：代码探索、重构与交互迭代 |

GitHub 的 Contributors 侧栏由提交关联账号自动生成；Bony 对应使用 `phuhao00`、`openai-codex[bot]` 与 `cursoragent`，页面显示可能受 GitHub 统计缓存影响。

---

## 与上游关系

| 层级 | 来源 |
|------|------|
| Agent / TUI / 工具栈 | 定期对齐 [`xai-org/grok-build`](https://github.com/xai-org/grok-build)（`Synced from monorepo`） |
| 产品层 | 本仓库自有：Bony 桌面集成、`bony-monitor`、品牌与协作文档 |
| 源码钉 | 根目录 [`SOURCE_REV`](SOURCE_REV) 记录上游 monorepo 同步点 |

### 同步上游

```powershell
git remote add upstream https://github.com/xai-org/grok-build.git   # 仅首次
git fetch upstream
git rebase upstream/main
# 历史改写后推送：
git push --force-with-lease origin main
```

## 仓库布局（摘要）

| 路径 | 说明 |
|------|------|
| `third_party/buzz/desktop` | Bony 桌面客户端实现，包含频道与 Coding Workspace；技术包名为 `buzz-desktop` |
| `third_party/buzz/crates/buzz-acp` | Coding Agent ACP 会话池与队列 |
| `crates/codegen/bony-monitor` | 架构与改动影响 Web 监控 |
| `crates/codegen/xai-grok-shell` | Agent 运行时、stdio / headless |
| `crates/codegen/xai-grok-pager*` | 官方 TUI（`grok`） |
| `crates/codegen/xai-grok-agent` / `*-tools` / `*-workspace` | Agent、工具、工作区 |
| `crates/codegen/xai-acp-lib` | ACP stdio 辅助库（桌面桥接使用） |
| `docs/` | Bony 文档、截图与架构图 |
| `scripts/buzz-room/` | Bony 本地协作栈的启动入口：relay / Desktop / 外部 agent |
| `third_party/buzz` | Bony 内嵌并改造的 Block/Buzz 底层源码（in-tree 工作区成员） |
| `SOURCE_REV` | 上游 monorepo 同步修订 |

完整上游说明见各 crate 文档与 [user guide](crates/codegen/xai-grok-pager/docs/user-guide/)。

---

## 开发

**项目强制规范**（终止目标 · 只用 Rust · 除启动外禁止脚本 · 性能 / 协作默认）：[`docs/PROJECT_STANDARDS.md`](docs/PROJECT_STANDARDS.md) · [`AGENTS.md`](AGENTS.md)

```powershell
$env:CARGO_TARGET_DIR = "$PWD\target"
$env:PROTOC = "$PWD\.tools\protoc\bin\protoc.exe"   # 若已放置 protoc
cargo check -p buzz-desktop -p bony-monitor
cargo test -p buzz-acp
cargo build -p buzz-desktop
```

建议忽略本地产物：`target/`、`.tools/`、`.local-dist/`、各类 `*.log`。

---

## 文档与许可

- 项目规范：[`docs/PROJECT_STANDARDS.md`](docs/PROJECT_STANDARDS.md)
- Coding Workspace：[`third_party/buzz/desktop/src/features/channels/ui/CodingWorkspaceScreen.tsx`](third_party/buzz/desktop/src/features/channels/ui/CodingWorkspaceScreen.tsx)
- 用户指南：[`crates/codegen/xai-grok-pager/docs/user-guide/`](crates/codegen/xai-grok-pager/docs/user-guide/)
- 认证：[`02-authentication.md`](crates/codegen/xai-grok-pager/docs/user-guide/02-authentication.md)
- 自定义模型：[`11-custom-models.md`](crates/codegen/xai-grok-pager/docs/user-guide/11-custom-models.md)
- 上游开源仓：[`xai-org/grok-build`](https://github.com/xai-org/grok-build)
- 许可证：[`LICENSE`](LICENSE) 及各 crate 声明

Agent 运行时与 `grok` CLI 能力来源于 [SpaceXAI / Grok Build](https://x.ai/cli) 与 [`xai-org/grok-build`](https://github.com/xai-org/grok-build)；房间与客户端基础代码来源于 [Block/Buzz](third_party/buzz)。
