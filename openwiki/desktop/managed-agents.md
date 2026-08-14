---
type: Agent 生命周期设计
title: Desktop 受管 Agent 与 Local Room Seed
description: Desktop 以持久化 Agent record 和按 agent/relay 隔离的运行时 pair 管理 ACP 子进程，并幂等 seed Local Room 的 ZeroClaw。
tags: [desktop, agents, acp, lifecycle]
---
# Desktop 受管 Agent 与 Local Room Seed

运行时 key 是 `(agent pubkey, effective workspace relay URL)`，由 `workspace_pair_key` 表达；不要只按 pubkey 汇总 status。`create_managed_agent` 校验输入，生成 Nostr keys 与 NIP-OA auth tag，解析 persona/team/runtime，再保存 `ManagedAgentRecord` 并可选启动。`spawn_agent_child` 以 `buzz-acp` 子进程注入 agent key、relay、harness/MCP、readiness、owner/allowlist gate 与 metadata。

存储在 app-data `agents/managed-agents.json`，日志按 `{pubkey}__{relay_hash}.log` 隔离。agent key 缺失时 spawn 必须拒绝。非 Windows keyring 可迁移并读回验证后剥离 inline key；Windows 因 Credential Manager 尺寸限制保留 0600 JSON fallback。

`seed_room_agents` 是本地 seat 的权威，而不是 PowerShell 脚本。当前 `room_agent_specs()` 仅定义 ZeroClaw：`research.web`、`mentions`、本地 ACP、拒绝 `deliver_file,file_write`。它幂等创建/复用 Local Room，清理遗留 Grok/Unity/OpenMontage/DocSmith seat，并加入 Local Room、welcome-everyone、general。脚本中旧的多 seat 文案不能推翻 Rust spec。

测试入口包括 `storage_tests.rs`、`agents_tests.rs`、`agent_config_tests.rs` 及 desktop E2E lifecycle/readiness。运行 `cargo test -p buzz-desktop`；ACP 行为见[ACP Harness](../agents/acp-harness.md)，persona contract 见[Personas](personas.md)。