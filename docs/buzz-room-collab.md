# Bony Room Collaboration

> 强制规范：[`docs/PROJECT_STANDARDS.md`](./PROJECT_STANDARDS.md) · [`AGENTS.md`](../AGENTS.md) · `.cursor/rules/`

Bony 是共享工作区。**默认内置房间座席是 ZeroClaw**（检索，`subscribe=mentions`）。用户可在 Desktop 里添加其它 ACP Agent。规格外的旧固定座席会在 `seed_room_agents` reconcile 时剥离。

产品源码在仓库根：`crates/`、`desktop/`。单一 `Cargo.toml` / `Cargo.lock` / `target/`。

## Layout

```
.
  Cargo.toml
  target/
  crates/                     # relay, acp, db, pubsub, …
  desktop/                    # Tauri (buzz-desktop)
  desktop/src-tauri/prompts/  # room seat copy
  scripts/buzz-room/          # start-room-stack / start-desktop / stop-room-stack
```

## Quick start

```powershell
cargo build -p buzz-relay
cargo build -p buzz-desktop
powershell -File .\scripts\buzz-room\start-room-stack.ps1 -SkipBuild
powershell -File .\scripts\buzz-room\start-desktop.ps1
powershell -File .\scripts\buzz-room\stop-room-stack.ps1
```

- 单一缓存：仓库根 `target/`。
- Desktop 默认含 local-stt（sherpa **shared DLL**）。
- 需要 CMake + VS 2022 C++（Opus 等）。

## Agent 发现

权威模型：`AgentDefinition` / `ManagedAgentRecord`。内置座席走 `room_seed`；用户 Agent 走创建 / catalog。**禁止** mint/register 脚本。

频道成员要看到座席，需加入 `Local Room`（`ws://127.0.0.1:3000`；本机勿走会劫持 `localhost` 的代理）。

### 动态 Agent 原则

- 不要平行再建一套角色清单。
- 用户新建默认 `subscribe=mentions`、`respond_to=owner-only`、最小工具权限。
- capability 不是权限；权限 = 用户授权 ∩ ACP allow/deny ∩ 运行时 ∩ 房间策略。
- 一个房间最多一个 `subscribe=all` coordinator（当前默认 seed **没有** coordinator；需要时由 owner 显式添加）。

### 检索与编码

| 能力 | 做法 |
|------|------|
| 今天资讯 / 天气 / 检索 | `@ZeroClaw` |
| 重编码 | Coding Workspace 选工程 + 已授权 ACP Agent（`coding-workspace-v1`） |

### ZeroClaw 硬拦

`BUZZ_ACP_DENY_TOOLS=deliver_file,file_write`（`crates/buzz-acp` 对 `session/request_permission` `reject_once`）。正文必须写在频道消息里，禁止沙盒附件死链。

## 更多

完整记忆与分工设计：[`docs/buzz-room-agent-orchestration-plan.md`](./buzz-room-agent-orchestration-plan.md)。编译闸门：`.cursor/skills/buzz-room-build-gate/SKILL.md`。角色契约：`.cursor/skills/buzz-agent-contracts/SKILL.md`。
