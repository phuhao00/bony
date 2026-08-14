---
type: 开发与验证指南
title: 构建、启动与分层验证
description: 本页说明 Bony 的根 workspace 构建入口、本地房间启动脚本、operator 工具与从单元到 Relay/Desktop E2E 的验证路径。
tags: [development, testing, operations, cargo]
---
# 构建、启动与分层验证

根 `Cargo.toml` 是单一 workspace；所有 crate 共享根 `target/`。不要为 Desktop 或子目录创建第二个 target，也不要无确认执行 `cargo clean`。

## 本地启动

```powershell
cargo build -p buzz-relay -p buzz-desktop
powershell -File .\scripts\buzz-room\start-room-stack.ps1 -SkipBuild
powershell -File .\scripts\buzz-room\start-desktop.ps1
powershell -File .\scripts\buzz-room\stop-room-stack.ps1
```

`start-room-stack.ps1` 启动可选 infra、迁移/relay 并等待 health；`start-infra.ps1` 默认不启动 Docker，SQLite + in-process pubsub 是默认单实例路径。`start-desktop.ps1` 检查 relay、准备 sidecar/Vite 并启动 Desktop。脚本不拥有 Agent provisioning：Desktop native `seed_room_agents` 才是 Local Room seat 的权威。

## 分层验证

| 改动 | 优先检查 | 何时扩大 |
|---|---|---|
| 核心/DB/auth/search | 对应 `cargo test -p <crate>` | 改 Relay 接入时加 relay/test-client |
| Relay API/workflow/media/Git | `cargo test -p buzz-relay` | 真实 NIP-01/HTTP 行为加 `buzz-test-client` |
| ACP/agent/tooling | `cargo test -p buzz-acp` 或 `buzz-agent` | 跨 Desktop spawn/room 时加 Desktop 检查 |
| Tauri/本地系统 | `cargo test -p buzz-desktop`、`cargo check -p buzz-desktop` | UI 生命周期运行相关 Playwright spec |
| React/TypeScript、样式或 Desktop 前端检查 | `pnpm --filter desktop typecheck`（类型）或 `pnpm --filter desktop check`（格式、lint 与 UI guard） | 改打包链路时加 `pnpm --filter desktop build`；仅 UI 运行行为变更时加对应 Playwright spec |
| mesh/push/pair relay | 对应 crate test | 仅改部署/网络时做条件性集成 smoke |

`buzz-test-client` 是运行中 Relay 的 NIP-01 WebSocket 黑盒 client，承载 relay、multitenant conformance、media、persona、project、team、Git、mesh 等 E2E。`buzz-admin` 是 operator CLI，连接 DB/Auth/PubSub/Search/Audit/Workflow/Media；变更其命令时不要把 operator-global 读取混入 community-scoped product path。

## 前端包管理与检查

根 `package.json` 固定 `pnpm@11.4.0`，根 `pnpm-workspace.yaml` 当前只包含 `desktop` 包。`desktop/package.json` 中 `typecheck` 只执行 `tsc --noEmit`，是 TypeScript/React 逻辑改动的最小检查：

```powershell
pnpm --filter desktop typecheck
```

`pnpm --filter desktop check` 还会运行 Biome、文件大小、`px` 文本和 pubkey 截断 guard，适合样式、格式或相应 UI 约束变更。根脚本 `pnpm check` 会递归执行所有 workspace 包的 `check`，仅当改动跨包或需要确认完整前端 workspace 时再使用。`pnpm-workspace.yaml` 同时集中锁定 `@radix-ui/react-dismissable-layer`，以避免嵌套 modal 关闭后遗留 `pointer-events: none`；修改 Radix 依赖解析、modal 交互或 lockfile 时须保留并验证此约束。该前端路径与本机 Rust command surface 共同构成 [Desktop 架构](../desktop/architecture.md) 所述的 shipped surface；纯 Rust 改动仍优先使用该页的 `cargo check -p buzz-desktop`。

## 文档维护自动化

`.github/workflows/openwiki-update.yml` 在 `main` 的 push（`openwiki/**` 自身变更除外）或手动触发后，安装 OpenWiki 并运行 `openwiki code --update --print --language zh-CN`，随后创建仅包含 `openwiki`、`AGENTS.md` 与 `CLAUDE.md` 的更新 PR。它以提交信息前缀 `docs: update OpenWiki` 防止循环触发。

改动源码、产品流程或运行配置后，可在本地运行同一条 OpenWiki 命令来更新代码知识库；该操作应先审阅生成差异，不能代替本页的 crate、Tauri 或 E2E 验证。工作流使用仓库 secrets 提供模型访问；不要将其值写入文档、日志或提交。