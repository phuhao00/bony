---
type: 安全与验证设计
title: 认证、授权与一致性检查
description: "`buzz-auth` 在 Relay 边界实施 NIP-42、NIP-98、访问控制和速率限制，`buzz-conformance` 独立重放多租户 trace。"
tags: [security, auth, conformance, nostr]
---
# 认证、授权与一致性检查

`buzz-auth` 是认证/授权域库，`buzz-conformance` 则刻意不依赖任何生产 Buzz crate。前者决定 live request 是否可以继续，后者从抽象 trace 检查多租户 non-interference 与状态迁移；不要让 production type 的缺陷污染 checker。

## Relay 认证路径

| 机制 | 主要符号 | 目的 |
|---|---|---|
| NIP-42 | `generate_challenge`、`verify_nip42_event` | WebSocket challenge/response。AUTH event 不落库。|
| NIP-98 | `verify_nip98_event`、`Nip98ReplayGuard` | HTTP 签名请求与 replay 防护。|
| 授权 | `AuthService`、`check_read_access`、`check_write_access`、`require_scope` | channel/member/scope 判定。|
| 限流 | `RateLimiter` | 在请求边界施加资源约束。|

认证成功不自动授予频道访问：Relay 仍按 tenant、成员与资源策略检查。NIP-OA owner attestation 可服务于 managed agent ownership，但不应把 profile 显示信息作为授权。

## 独立 conformance

`buzz-conformance` 暴露 `check_trace`、`TraceStep`、`TraceAction`、`AbstractState` 和 `Verdict`。Relay 在 ingest/read 的 accept-reject 边界发出 trace；checker 使用自身的 `CommunityLabel` 等标签验证合法迁移、错误字母表、关键路径覆盖及跨社区隔离。代表性测试为 `tests/proptest_checker.rs` 与 `tests/replay_fixtures.rs`。

## 变更面

认证变更至少审查 WS 与 HTTP 两条入口、replay/限流实现、Relay handler 的 access gate、错误泄露面以及 conformance trace 是否仍表达同一决定。运行 `cargo test -p buzz-auth` 和 `cargo test -p buzz-conformance`；若触及 Relay 接入，追加 `cargo test -p buzz-test-client`。相关 tenant 数据约束见[持久化](persistence.md)。