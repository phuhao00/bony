<div align="center">

# Bony Build

**原生桌面 AI 编程助手 + 本地多 Agent 协作房间** — 对话改代码、任务隔离 worktree、详情面板版本管理、会话级插件（Unity / Bevy）；内置 [Buzz](third_party/buzz) 房间，Grok 协调 ZeroClaw / Unity / OpenMontage / DocSmith 等专员协作，纯 Rust + SQLite，无需 Docker。

**语言:** **中文** · [English](README.en.md)

[预编译包](#预编译包) ·
[快速开始](#快速开始) ·
[功能](#功能) ·
[Buzz 本地协作房间](#buzz-本地协作房间) ·
[详情与版本管理](#详情与版本管理) ·
[插件与 Unity](#插件与-unity) ·
[Web 监控](#web-监控) ·
[模型与供应商](#模型与供应商) ·
[架构](#架构) ·
[与上游关系](#与上游关系) ·
[开发](#开发)

![Bony Build 桌面端](docs/bony-build-desktop-2026-07-25.png)

</div>

---

## 这是什么

本仓库是两块拼在一起的东西，共用一个 Rust workspace：

1. **Bony Build**：原生桌面客户端（Rust / egui，当前 `v0.1.4`）。通过 [ACP](https://agentclientprotocol.com/) 驱动本地 `grok agent stdio`，在选定仓库里做**对话式编程**——探索代码、改文件、跑终端与搜索工具——而不是只做一个聊天窗口。
2. **Buzz 本地协作房间**：内置的多 Agent 群聊后端（详见 [Buzz 本地协作房间](#buzz-本地协作房间)）。Grok 当协调员，ZeroClaw / Unity / OpenMontage / DocSmith 等专员按 `@` 交接分工；后端已单机化重构成纯 Rust + SQLite，不依赖 Docker / Postgres / Redis。

适合：

- 本机 **多供应商 BYOK**（Qwen / Kimi / 智谱 / OpenAI 兼容等）日常改码
- **按任务隔离 Git worktree**，侧栏「按项目」管理对话，避免弄脏主工作区
- 右侧 **详情面板**查看工作副本改动、按文件看 diff、浏览提交历史（Fork 风格）
- Unity / Bevy 等本地扩展：Unity 走 **本机 CLI 闭环**（探测、Play、Pipeline），不经 Agent 挂死安装
- 用本地 **Web 监控**看架构分层与每次提交对功能的影响
- 需要多个专精 Agent 分工协作时，用 **Buzz 房间**里 `@` 交接，而不是一个 Agent 全包

常见用法：解释仓库结构、排查近期改动、补测试、总结认证 / 架构；或在 Buzz 房间里让 ZeroClaw 检索、Unity 操作引擎、DocSmith 出文档。任务权限：只读 / 询问 / 允许编辑 / 完全控制；也可用 `--ask-permissions` 全局要求人工批准。

**产品品牌与桌面壳为 Bony Build**；agent / TUI 运行时对齐开源上游 [`xai-org/grok-build`](https://github.com/xai-org/grok-build)（见 [与上游关系](#与上游关系)）。仓库：[`phuhao00/bony`](https://github.com/phuhao00/bony)。

---

## 预编译包

GitHub Releases 提供桌面 zip（需本机另装 `grok` CLI）：

- [**Bony Build v0.1.4**](https://github.com/phuhao00/bony/releases/tag/v0.1.4)
  - `bony-build-v0.1.4-windows-x86_64.zip`
  - `bony-build-v0.1.4-macos-aarch64.zip`
  - `bony-build-v0.1.4-macos-x86_64.zip`

由 [`.github/workflows/release-desktop.yml`](.github/workflows/release-desktop.yml) 在推送 `v*` tag 时构建（`release-dist` profile）。本地打包产物目录 `.local-dist/` 已写入 [`.gitignore`](.gitignore)，不要提交 exe / zip。

---

## 功能

| 能力 | 说明 |
|------|------|
| 对话工作区 | Codex 风格侧栏 + 时间线；Markdown、用户气泡 / 助手卡片、工具结果内联 |
| 新建对话 | 顶层「新建对话」不强制绑项目；侧栏有「最近对话」收件箱 |
| 按项目分组 | 侧栏「按项目」聚合对话；可删除 / 归档；标题自动建议 |
| 任务与 worktree | 新建 / 切换任务；可选隔离 worktree 与分支 |
| 详情 · 版本管理 | 右侧可拖宽详情：工作副本文件列表、着色 diff、描述并提交、历史 → 变更文件 → 单文件 patch |
| 会话级插件 | 输入框旁 **`+`**：添加文件、启用 Unity / Bevy、管理插件；芯片可 × 移除 |
| 插件商店 | 「插件」页：标签、搜索、已安装条、整宽卡片布局 |
| 权限模式 | 任务级：只读 / 询问 / 允许编辑 / 完全控制；CLI 支持 `--ask-permissions` |
| 模型切换 | 输入区点模型名切换会话，并写入 `~/.grok/config.toml` 默认值 |
| 多供应商 | Kimi / Qwen / 智谱 / OpenAI 兼容 / Anthropic Messages 等 BYOK |
| Unity 控制 | 「插件」页安装与工程绑定；聊天芯片 + 快捷按钮 / `/unity`；**本地 CLI，不经 Agent** |
| Bevy | 可选 Rust ECS 游戏开发集成（插件页启用） |
| 使用统计 | 轮次与 Token 用量面板（折线 / 柱状） |
| 中文界面 | 系统中文字体（如微软雅黑），避免乱码 |
| 快捷键 | **Enter** 发送，**Shift+Enter** 换行 |
| Web 监控 | 架构分层、「怎么工作」、功能影响矩阵与提交影响时间线 |

侧栏主导航当前为：**新建对话** · **聊天** · **插件**。站点 / PR / 定时等仍为占位。

---

## Buzz 本地协作房间

本仓库另含 [Block/Buzz](third_party/buzz) 的本地多 Agent 协作房间：**Grok** 担任房间协调员，**ZeroClaw**（检索）/ **Unity**（游戏引擎）/ **OpenMontage**（剪辑）/ **DocSmith**（文档产出）等专员在共享频道里按 `@` 串行交接协作——工具调用中途状态可见，消息可加表情反馈，还能按需开子线程深入追问。

后端已完成**单机化重构**，不再依赖 Docker / Postgres / Redis：持久化用 **SQLite**（WAL 模式 + 30s busy timeout，支持多 Agent 并发写入不报错）；发布订阅、限流、在线状态用**进程内实现**取代 Redis；语义检索接入嵌入式 **[LanceDB](https://github.com/lancedb/lancedb)** 向量库；对象存储仍走 S3 兼容 OSS，且为可选项——不配置也能本机跑起来。

![Buzz 房间：Grok 交接 ZeroClaw 查询深圳天气，右侧子线程展示完整天气播报](docs/buzz-room-local-room.png)

一键启动（仅限白名单脚本，编译一律走 `cargo build -p <crate>`）：

```powershell
powershell -File .\scripts\buzz-room\start-room-stack.ps1 -SkipBuild   # relay + 进程内 pubsub + SQLite
powershell -File .\scripts\buzz-room\start-desktop.ps1                 # Buzz Desktop
powershell -File .\scripts\buzz-room\stop-room-stack.ps1               # 停止
```

策略与架构细节见 [`docs/buzz-room-collab.md`](docs/buzz-room-collab.md)、[`third_party/buzz/README.md`](third_party/buzz/README.md)、[`scripts/buzz-room`](scripts/buzz-room)。

---

## 详情与版本管理

打开右侧 **详情** 面板可看到会话信息与 Git 工作区：

1. **工作副本** — 扫描**项目主仓库**（不是 agent 隔离 worktree），列出 A/M/D 等改动  
2. **描述并提交** — 有改动时出现说明输入框，填完后提交  
3. **最近历史** — 点击某条提交 → **变更文件**列表（增减条）→ 点文件看该文件 patch  
4. 面板左缘可**向左拖宽**，方便阅读长 diff  

非 Git 目录不会弹错误窗，只显示「不是 Git 仓库」。刷新约每 2 秒自动更新，也可点「刷新」。

---

## 插件与 Unity

### 插件模型

1. 侧栏 **「插件」**：启用 / 关闭 Unity、Bevy 等，打开设置或说明  
2. 聊天输入框旁 **`+`**：本会话添加文件或扩展；出现可关闭的上下文芯片  
3. Unity 启用后，输入区提供快捷操作（保存场景、刷新资源、Play 等）与「说明」

Unity 操作走 **本机 Unity CLI**，不经过 grok Agent，避免在 worktree 里挂死 `unity pipeline install`。

### 推荐安装步骤

安装 CLI → 重新检测 → 确认含 `Assets` 的工程根 → 安装 Pipeline → 打开编辑器后探测 → 跑闭环。Windows 默认 CLI：`%LOCALAPPDATA%\Unity\bin\unity.exe`。

```powershell
$env:UNITY_CLI_CHANNEL='beta'; irm https://public-cdn.cloud.unity3d.com/hub/prod/cli/install.ps1 | iex
```

更细说明见 [`crates/codegen/bony-build/README.md`](crates/codegen/bony-build/README.md)。

---

## Web 监控

本地仪表盘，查看 **整体架构**、端到端「怎么工作」，以及 **每一次改动带来的影响**：

```powershell
powershell -ExecutionPolicy Bypass -File .\scripts\run-monitor.ps1
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
# 开发：构建并运行
powershell -ExecutionPolicy Bypass -File .\scripts\run-desktop.ps1

# 干净重启：结束旧进程 → release 构建 → 启动
powershell -ExecutionPolicy Bypass -File .\scripts\run-bony-build.ps1

# 或
$env:CARGO_TARGET_DIR = "$PWD\target"
cargo run -p bony-build
```

常用参数：

```text
--cwd <path>        会话工作目录（默认当前目录）
--grok-bin <path>   grok 可执行文件路径
--ask-permissions   工具需手动批准（默认自动批准）
```

Windows 若遇到 **os error 4551**（Smart App Control），请在可信终端中构建，或关闭 SAC 后重试。

### 也可使用终端 TUI

本仓库仍包含完整 `grok` TUI / agent 源码：

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
  UI["Bony Build<br/>egui 桌面壳"]
  ACP["ACP JSON-RPC<br/>over stdio"]
  Agent["grok agent stdio<br/>MvpAgent / SessionActor"]
  Sample["采样 · 多 backend"]
  Tools["工具 · 终端 / 文件 / 搜索"]
  WS["Workspace / MCP / 子 agent"]
  Unity["旁路 · Unity CLI<br/>本机进程，不经 Agent"]
  Git["旁路 · 详情面板 Git<br/>主仓库 status / show"]

  UI --> ACP --> Agent
  Agent --> Sample
  Agent --> Tools
  Agent --> WS
  UI -.-> Unity
  UI -.-> Git
```

分层与一次 turn 流程：

![架构分层](docs/architecture-layers.png)

![Turn 流程](docs/architecture-turn-flow.png)

- 桌面 crate：[`crates/codegen/bony-build`](crates/codegen/bony-build)
- 文字说明：[`ARCHITECTURE.md`](ARCHITECTURE.md)

桌面端**不**内嵌完整 agent 运行时，而是驱动已安装的 `grok` 子进程。

---

## 与上游关系

| 层级 | 来源 |
|------|------|
| Agent / TUI / 工具栈 | 定期对齐 [`xai-org/grok-build`](https://github.com/xai-org/grok-build)（`Synced from monorepo`） |
| 产品壳 | 本仓库自有：`bony-build`、`bony-monitor`、品牌文档、桌面 release 工作流 |
| 源码钉 | 根目录 [`SOURCE_REV`](SOURCE_REV) 记录上游 monorepo 同步点 |

### 同步上游

```powershell
git remote add upstream https://github.com/xai-org/grok-build.git   # 仅首次
git fetch upstream
git rebase upstream/main
# 历史改写后推送：
git push --force-with-lease origin main
```

回滚可用 tag `backup/pre-upstream-sync`（若本地仍保留）。

---

## 仓库布局（摘要）

| 路径 | 说明 |
|------|------|
| `crates/codegen/bony-build` | Bony Build 桌面客户端（详情 VCS、Unity / Bevy / 插件 UX） |
| `crates/codegen/bony-monitor` | 架构与改动影响 Web 监控 |
| `crates/codegen/xai-grok-shell` | Agent 运行时、stdio / headless |
| `crates/codegen/xai-grok-pager*` | 官方 TUI（`grok`） |
| `crates/codegen/xai-grok-agent` / `*-tools` / `*-workspace` | Agent、工具、工作区 |
| `crates/codegen/xai-acp-lib` | ACP stdio 辅助库（桌面桥接使用） |
| `scripts/run-desktop.ps1` | 桌面端构建运行 |
| `scripts/run-bony-build.ps1` | 结束旧进程 + release 构建 + 启动 |
| `scripts/run-monitor.ps1` | 启动 Web 监控（默认 :8787） |
| `.github/workflows/release-desktop.yml` | 多平台桌面 zip release |
| `docs/` | 截图与架构图（含 Buzz 房间协作） |
| `scripts/buzz-room/` | 本地 Buzz 房间：relay / Desktop / 外部 agent |
| `third_party/buzz` | Buzz 源码（in-tree 工作区成员） |
| `SOURCE_REV` | 上游 monorepo 同步修订 |

完整上游说明见各 crate 文档与 [user guide](crates/codegen/xai-grok-pager/docs/user-guide/)。

---

## 开发

**项目强制规范**（终止目标 · 只用 Rust · 除启动外禁止脚本 · 性能 / 协作默认）：[`docs/PROJECT_STANDARDS.md`](docs/PROJECT_STANDARDS.md) · [`AGENTS.md`](AGENTS.md)

```powershell
$env:CARGO_TARGET_DIR = "$PWD\target"
$env:PROTOC = "$PWD\.tools\protoc\bin\protoc.exe"   # 若已放置 protoc
cargo check -p bony-build -p bony-monitor
cargo build -p bony-build --profile release-dist
cargo run -p bony-build -- --cwd $PWD
```

建议忽略本地产物：`target/`、`.tools/`、`.local-dist/`、各类 `*.log`。

打 release：推送 annotated tag（如 `v0.1.4`）触发桌面工作流，或 `workflow_dispatch` 指定已有 tag。

---

## 文档与许可

- 项目规范：[`docs/PROJECT_STANDARDS.md`](docs/PROJECT_STANDARDS.md)
- 用户指南：[`crates/codegen/xai-grok-pager/docs/user-guide/`](crates/codegen/xai-grok-pager/docs/user-guide/)
- 认证：[`02-authentication.md`](crates/codegen/xai-grok-pager/docs/user-guide/02-authentication.md)
- 自定义模型：[`11-custom-models.md`](crates/codegen/xai-grok-pager/docs/user-guide/11-custom-models.md)
- 上游开源仓：[`xai-org/grok-build`](https://github.com/xai-org/grok-build)

本仓库含从 SpaceXAI monorepo / `xai-org/grok-build` 同步的 agent / TUI 源码；桌面产品层为 Bony Build。许可证见根目录 [`LICENSE`](LICENSE) 及各 crate 声明。

---

## 致谢

Agent 运行时与 `grok` CLI 能力来源于 [SpaceXAI / Grok Build](https://x.ai/cli) 与 [`xai-org/grok-build`](https://github.com/xai-org/grok-build)。Bony Build 在其上提供多供应商桌面体验、任务 / worktree、详情面板版本管理、会话级插件（Unity / Bevy），以及改动可观测。
