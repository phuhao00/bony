---
type: 可选计算运行时
title: Desktop Mesh LLM 与 Mesh Compute
description: feature-gated Mesh LLM 在 Desktop 中管理本地模型节点、可信 Nostr 发现、Iroh 传输及 Agent provider 消费。
tags: [mesh, llm, desktop, compute]
---
# Desktop Mesh LLM 与 Mesh Compute

`mesh_llm` feature 下，`lib.rs` 在 Tauri async runtime 前设置更大 worker stack；`mesh_llm/mod.rs` 负责 embedded serve/client runtime、model catalog 和状态机 Off/Starting/Running/Stopping/Failed。它与 Relay QUIC mesh 不同：这是 Desktop 内嵌模型计算面。

启动包含模型下载超时、API/console port、health/readiness 与 recovery/stop timeout。MeshLLM SDK 的管理端 ready 不等同于 Buzz agent 所需 OpenAI ingress ready，二者均需监督。`transport_policy.rs` 验证签名 discovery、relay/address policy；owner/device identity 与 allowlist `TrustPolicy` 约束可连接成员，Iroh 配置属于此信任边界。

`commands/mesh_llm.rs` 暴露 startup、selection、restore、shutdown；`features/mesh-compute` 是 UI consumer。修改时覆盖 stale runtime/re-arm recovery、信任拒绝、模型失败和端口清理；运行 `cargo test -p buzz-desktop` 与 `desktop/tests/e2e/mesh-compute.spec.ts`。