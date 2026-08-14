---
type: 本地状态与安全设计
title: Desktop 身份、备份、归档与同步安全
description: Desktop 将人类身份、受管 Agent、备份和离线同步保存在本机，并在所有 relay 出站边界禁止密钥材料泄露。
tags: [desktop, identity, security, sync]
---
# Desktop 身份、备份、归档与同步安全

人类 identity 的解析优先级是有效环境 private key → OS keyring → app-data `identity.key` → 新生成并持久化。`IdentityStorage` 区分 Ephemeral/SystemKeyring/LocalFile/Environment；recovery state 下 `AppState::signing_keys()` fail closed。它与 agent store 不同：后者见[受管 Agent](managed-agents.md)。

`key_backup.rs` 使用 NIP-49 `ncryptsec`；创建后必须解密验证同一 pubkey，读取不可信备份时限制 KDF 成本，写入使用 atomic owner-only/create-new 保护。`egress_guard.rs` 是硬不变量：`ncryptsec` 绝不经 relay/native WebSocket/huddle transcription 等出站点发送；新增出站构造点必须接 guard 并补全覆盖测试。

`event_sync.rs` 在 identity 解析后处理 persona、team、managed-agent 的 retention-first reconciliation：先耐久化本地 SQLite/pending sync，再在 relay 可用时发布已签名事件；坐标内容比较和单调时间避免漏同步与无意义重发。`archive/pipeline.rs` 校验签名、kind 范围、subscription scope、relay 复核，区分 ephemeral/persistent 并允许受控 partial failure。

变更需运行身份 backup、egress、migration、event-sync/archive 相邻测试与 `cargo test -p buzz-desktop`。Relay migration 的协作数据边界见[平台持久化](../platform/persistence.md)。