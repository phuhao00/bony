# Bony Build 项目规范

> Agent 与人工共用的强制约定。Cursor 侧镜像：`.cursor/rules/*.mdc`（`alwaysApply`）。

## 1. 终止目标

交付 **原生高性能桌面 AI 编程助手 + 本地多 Agent 协作房间**：

| 面 | 终态 |
|----|------|
| Buzz Desktop | 唯一 Tauri 桌面壳 · Coding Workspace · ACP · 本地多 Agent 房间 |
| 运行时 | 本地 agent + BYOK · 权限模式 · 逻辑在 Rust |
| Buzz 房间 | 单 workspace / 单 `target/` · Grok 协调 · 串行 `@` 交接 · 硬拦兜底 |
| 协作智能 | 分工执行 → 记忆写回 → 下次检索（见 `buzz-room-agent-orchestration-plan.md`） |

**任务完成** = 可验证 + 无脚本/多语言债务 + 职责在正确 crate + 最短协作路径 + 性能默认最优 + **无多余重复实现**。

**非目标**：脚本农场、多语言胶水、第二套构建树、靠 prompt 软约束代替引擎硬拦、复制粘贴式并行实现。

## 2. 语言

- **新增实现只允许 Rust**（根 `Cargo.toml` workspace 成员）。
- 构建：`cargo build|test|run|clippy -p <crate>`（仓库根）。
- Buzz Desktop 既有 TS **不扩大业务**；agent/seed/编排进 Tauri Rust 与 `buzz-*` crate。
- 禁止为新能力引入 Python / Node / Go / bash 业务程序。

## 3. 脚本

**只许启动入口**（无业务）：

```powershell
powershell -File .\scripts\buzz-room\start-room-stack.ps1 -SkipBuild
powershell -File .\scripts\buzz-room\start-desktop.ps1
powershell -File .\scripts\buzz-room\stop-room-stack.ps1
```

- 编译、seed、注册、清理策略 → **Cargo 或 Rust 代码**。
- **禁止新增**自动化脚本；**禁止**依赖 `build-*.ps1` / `mint-*` / `register-*` 等非启动脚本。

## 4. 性能默认

1. 同进程 crate 调用 > 子进程 > 解释器。
2. 单根 `target/`；有 exe 不盲目重编。
3. 零额外脚本运行时；热路径少分配。
4. 不因编译时长默认关功能（如 STT）。

## 5. 协作默认

| 场景 | 动作 |
|------|------|
| 单域 | 直接指派最擅长 agent；不讨论 |
| 资讯→文档 | `@ZeroClaw` → `@DocSmith`（两帖，各一 `@`） |
| 跨域 | 短讨论仅在必要；再串行执行 |
| 动态 / 用户 Agent | capability 路由；默认 mention-only + owner-only；高权限显式授权 |
| 防回归 | `BUZZ_ACP_DENY_TOOLS`、meta 过滤、工具名级禁令 |

开发 Agent 分工、所有权与交接：`docs/AGENT_COLLABORATION.md`。

Buzz 运行时协作：`docs/buzz-room-collab.md`、`docs/buzz-room-agent-orchestration-plan.md`。

## 6. 去冗余 · 抽象 · 模块化

**写前**：仓库内搜索同位逻辑 → 复用或抽薄公共点 → 再写。

**分层**（固定方向）：

```
入口（command / bin / UI）→ 薄编排 → 领域逻辑 crate/mod → 基础设施
```

- 业务规则只放领域层；UI / Tauri command / prompt **不**堆完整业务副本。
- **一次真相**：校验、映射、策略、常量单点定义。
- 抽象门槛：≥2 处重复或跨模块共享再抽；禁止为一处调用铺空 framework / trait 金字塔。
- 扩现有模块 API 优于平行新开同名能力；跨 bony/Buzz/grok 的共享下沉到底层 crate。
- 合并近重复分支，删死代码；入口保持薄。

完成自检：无大段 copy-paste；有唯一权威实现；无「未来也许用」的过度抽象。

Cursor 规则：`.cursor/rules/modularity-dry.mdc`。

## 7. 布局（摘要）

| 路径 | 职责 |
|------|------|
| `third_party/buzz/desktop` | Buzz Desktop 界面与 Tauri 本机集成 |
| `third_party/buzz/crates/buzz-acp` | Coding Agent ACP 会话池与队列 |
| `crates/codegen/xai-grok-*` | Agent / 工具 / TUI |
| `third_party/buzz` | Buzz in-tree sources |
| `scripts/buzz-room/prompts/` | Agent 文案（非业务运行时） |
| `.cursor/rules/` | 本规范的 agent 强制镜像 |

## 8. 冲突优先级

1. 用户当次明确指令  
2. 本文件 + `.cursor/rules/*`  
3. `docs/AGENT_COLLABORATION.md` / `docs/buzz-room-*.md` / skills
4. 历史脚本或 README 过时段落（应用本规范纠正）
