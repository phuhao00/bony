---
type: 运维与集成设计
title: 设备配对、Git 凭据与 Agent 经济
description: 本页说明 NIP-AB 配对 sidecar、Nostr Git helper 和文件型 Agent 经济系统的独立信任与持久化边界。
tags: [pairing, git, economy, operations]
---
# 设备配对、Git 凭据与 Agent 经济

## NIP-AB 配对

`buzz-pair-relay` 是 loopback-only、短生命周期的 handshake sidecar：无持久化、无认证、无历史，只转发 kind `24134`。它强制 Schnorr 验签、created_at freshness、128 WS connection、4 KiB frame、120 秒 TTL、每连接 6 events、全局 dedup、每个 `#p` delivery/map 容量预算。TLS、反向代理 `/pair` 和慢连接防护属于部署者责任。`reserve_id` 在 TTL 清理后区分 duplicate/new/capacity exhausted；投递失败必须 `unreserve_id`。`deliver_single` 要求恰一个 subscriber，并在同一临界区检查/增加 per-`#p` budget；writer `try_send` 失败不增加 delivery counter。`ConnGuard::drop` 清理 subscription 和 connection count。`buzz-pair` CLI 用相同 `buzz-core::pairing` 契约作互操作诊断。验证 `cargo test -p buzz-pair-relay`。

## Git + Nostr

`git-credential-nostr` 按 Git credential-helper stdin/stdout 协议把 NIP-98 Authorization challenge 生成 credential，支持 NIP-OA auth tag；非 Buzz remote 静默退出以允许其他 helper。keyfile 在 Unix 限 0600/256 bytes 并零化 raw private key。`git-sign-nostr` 是 Unix GnuPG-compatible signing program，支持 Git commit/tag 的 BIP-340 sign/verify 及 status-fd。二者通过 `sprig` alias 可被开发工具使用。

它们是客户端证明，不是授权裁决；服务端 Smart HTTP、hook/HMAC policy 见[Relay Git 服务](../relay/git-service.md)。

## Agent 经济

`buzz-economy` 是余额、声望、拍卖、组织、招标、结算与能力成长的唯一实现。`EconomyPaths` 解析文件位置，`append_chained` 在独占文件锁下写 hash chain，因此 Desktop 和 `buzz-dev-mcp` 不得直接修改 ledger/tender/contract 文件。详细状态机、报价、奖励、Tauri/UI 表面见[Agent 经济与开放招标市场](../economy.md)。