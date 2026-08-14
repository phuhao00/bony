# 文件

- [Relay Git 服务与授权链](git-service.md) - Relay Git API 提供 Smart HTTP 传输、仓库数据生命周期与受 loopback/HMAC 保护的 hook policy 回调。
- [Relay Mesh 与 Push Gateway](mesh-push.md) - Relay mesh 以 QUIC/Iroh 提供可选跨节点传输，Push Gateway 以 capability 和 App Attest 保护 APNs 最后一跳。
- [Buzz Relay 服务与实时 API](service-api.md) - `buzz-relay` 组装 SQLite、认证、pubsub、搜索、审计和工作流，并提供 NIP-01 WebSocket 与 Axum REST 服务。
- [工作流自动化与媒体存储](workflows-media.md) - `buzz-workflow` 执行频道范围的 YAML 自动化，`buzz-media` 提供经校验的对象存储、处理和归因能力。
