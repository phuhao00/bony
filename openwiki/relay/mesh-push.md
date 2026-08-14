---
type: 分布式与推送设计
title: Relay Mesh 与 Push Gateway
description: Relay mesh 以 QUIC/Iroh 提供可选跨节点传输，Push Gateway 以 capability 和 App Attest 保护 APNs 最后一跳。
tags: [mesh, quic, push, apns]
---
# Relay Mesh 与 Push Gateway

## Relay mesh

`buzz-relay-mesh` 由 Relay `mesh_boot` 在 `BUZZ_MESH` seam 后装配。`MeshRuntime`、`MeshConfig`、`RelayMeshMembership`、`RelayPeerTransport` 与 `wire` 提供 Iroh/QUIC connection、gossip membership、可靠 stream 与 realtime datagram。membership 仅为路由提示；所有权仲裁仍由 Redis fenced generation 完成。`FencedHeader` 显式拒绝 `StaleGeneration`、`NoActiveLease`、`OwnerMismatch` 与 `FutureGeneration`。mesh off 时必须保留单实例进程内路径。

## Push Gateway

`buzz-push-gateway` 是独立 binary/service，不依赖其它 Buzz crate。`main.rs` 支持 `--migrate-only`，装配 SQLite `AuthorityStore`、APNs `ApnsTransport`、`AppAttestVerifier`、grant/token keyring，启动 public 与 health listener，并运行过期记录 reaper。它实现盲化、capability-gated 的 NIP-PL 到 APNs last hop；不要将可识别 payload 或未经证明的设备请求扩散到 authority store。

`MeshMembership::apply_ready_records` 在缺少 `expected_relay_pubkey` 时 fail closed；先匹配 relay pubkey，再验证 attestation，忽略 local runtime/self runtime record。`apply_gossip_record` 只接受足够新的 version，才更新 peer、connection state 与 phi observation。`has_peer`、records、gossip digest 仅用于成员/可拨号路由提示，绝不能推导 session ownership 或 failover。

App Attest `AppAttestVerifier::new` 固定 app ID 与 Apple root PEM SHA-256。attestation 依序限制 base64/大小、UTF-8 challenge、库验证和 32-byte key id；assertion 限制 CBOR 大小，提取 counter，并把 client-data hash、challenge、app ID、公钥、旧 counter 交给验证库。counter 只接受封闭 CBOR map/字段形状，未知字段、无 auth data 或错误长度均拒绝；外部格式/验证异常收敛为不泄露细节的 `AppAttestError::Invalid`。

## 验证

mesh 修改运行 `cargo test -p buzz-relay-mesh` 与 `cargo test -p buzz-relay`，覆盖 mesh off、fence 拒绝和 membership churn。Push 修改运行 `cargo test -p buzz-push-gateway`，特别检查 App Attest、grant/token rotate、SQLite reaper 和 APNs 失败。默认 Desktop 不要求这些服务已启动。