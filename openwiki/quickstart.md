---
type: 代码库导航
title: Bony 代码库快速开始
description: Bony 是本地优先的桌面 AI 编程与协作平台；本页将改动意图压缩到权威系统页面、源码符号、测试和验证命令。
tags: [bony, rust, tauri, agents]
---
# Bony 代码库快速开始

Bony 是一个根 Cargo workspace：`buzz-desktop` 提供 Tauri/React 本机工作区，`buzz-relay` 提供 Nostr 协作服务，`buzz-acp`/`buzz-agent` 运行 coding 与 room agent，SQLite 持久化共享协作数据。产品源码在 `crates/` 与 `desktop/`；`buzz-*` 是技术 crate 前缀。

## 系统地图

- [架构概览](architecture/overview.md)：所有运行平面和端到端交接。
- 平台基础：[核心协议](platform/core-protocol.md)、[SQLite 持久化](platform/persistence.md)、[认证与一致性](platform/auth-conformance.md)、[搜索与审计](platform/search-audit.md)。
- Relay：[服务 API](relay/service-api.md)、[Git 服务](relay/git-service.md)、[工作流与媒体](relay/workflows-media.md)、[Mesh 与 Push](relay/mesh-push.md)。
- Agent：[ACP Harness](agents/acp-harness.md)、[ACP Agent](agents/acp-agent.md)、[开发者工具](agents/developer-tooling.md)。
- Desktop：[架构与 IPC](desktop/architecture.md)、[本地状态安全](desktop/local-state-security.md)、[受管 Agent](desktop/managed-agents.md)、[Personas](desktop/personas.md)、[Coding Workspace](desktop/coding-workspace.md)、[Huddle/终端](desktop/huddle-terminal.md)、[Mesh Compute](desktop/mesh-compute.md)。
- 运行与业务：[Local Room](room-collaboration.md)、[配对/Git/经济](operations/pairing-git-economy.md)、[经济市场](economy.md)、[开发与验证](operations/development-testing.md)。

## 任务路由

| 意图 | 页面 | 源码入口/符号 | 聚焦验证 |
|---|---|---|---|
| 新增 Nostr kind、tenant 契约或 filter | [核心协议](platform/core-protocol.md) | `buzz-core::{kind,tenant,filter}` | `cargo test -p buzz-core` |
| 修改 schema、SQLite 并发或 event 查询 | [持久化](platform/persistence.md) | `buzz-db::sqlite_connect_options`、`migrations/0001_initial_schema.sql` | `cargo test -p buzz-db` |
| 改 REST/WS handler、订阅或 Relay startup | [Relay 服务](relay/service-api.md) | `router::build_router`、`AppState`、handlers | `cargo test -p buzz-relay` |
| 改 Git push/fetch/权限 | [Relay Git 服务](relay/git-service.md) | `api/git/{transport,policy,hook}` | relay + Git E2E |
| 改 ACP event 到 Agent session | [ACP Harness](agents/acp-harness.md) | `EventQueue`、`AgentPool`、`PoolLifecycle` | `cargo test -p buzz-acp` |
| 改 agent JSON-RPC/provider/MCP | [ACP Agent](agents/acp-agent.md) | `run`、`Session`、`Provider` | `cargo test -p buzz-agent` |
| 改 Tauri command/React surface | [Desktop 架构](desktop/architecture.md) | `lib.rs::run`、`generate_handler!`、`tauri.ts` | `pnpm --filter desktop typecheck`（TypeScript）或 `cargo check -p buzz-desktop`（Rust） |
| 改 managed agent/room seed | [受管 Agent](desktop/managed-agents.md) / [Local Room](room-collaboration.md) | `create_managed_agent`、`spawn_agent_child`、`seed_room_agents` | `cargo test -p buzz-desktop` |
| 改本地项目/Git diff | [Coding Workspace](desktop/coding-workspace.md) | `canonical_project_path`、`build_workspace_snapshot` | desktop test + E2E |
| 改 identity/backup/sync | [本地状态安全](desktop/local-state-security.md) | `key_backup`、`egress_guard`、`event_sync` | `cargo test -p buzz-desktop` |
| 改 Huddle、TTS 或 PTY | [Huddle/终端](desktop/huddle-terminal.md) | `start_huddle`、`terminal_attach` | desktop tests/E2E |
| 改市场账本、招标或奖励 | [经济市场](economy.md) | `EconomyPaths`、`append_chained`、tender APIs | `cargo test -p buzz-economy --lib --quiet` |
| 维护代码知识库或 OpenWiki 自动更新 | [开发与验证](operations/development-testing.md#文档维护自动化) | `.github/workflows/openwiki-update.yml`、`openwiki code --update --print --language zh-CN` | 审阅 `openwiki/` 差异；无需运行产品构建 |

## 默认命令

```powershell
cargo check -p buzz-desktop
cargo test -p buzz-acp
cargo build -p buzz-relay -p buzz-desktop
```

完整启动、条件性集成验证与 test-client 使用见[开发与验证](operations/development-testing.md)。

## Backlog

无。所有当前 workspace member 均已归入上列 canonical 系统页面；已从仓库删除的 `crates/codegen/xai-grok-pager`、`crates/codegen/xai-grok-shell` 和 `third_party/buzz/mobile` 不再是可运行或可变更组件，故不保留独立页面。纯 test helper 或单一分发 alias 合并到其运行时所有者页面。