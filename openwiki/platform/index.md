# 文件

- [认证、授权与一致性检查](auth-conformance.md) - `buzz-auth` 在 Relay 边界实施 NIP-42、NIP-98、访问控制和速率限制，`buzz-conformance` 独立重放多租户 trace。
- [核心协议与租户模型](core-protocol.md) - `buzz-core` 是零 I/O 的共享契约层，定义 Nostr 事件、社区租户、频道、过滤、配对和 Git 权限类型。
- [Relay SQLite 持久化与迁移](persistence.md) - `buzz-db` 与根 `migrations/0001_initial_schema.sql` 共同定义单实例、多租户 Relay 的 SQLite 存储、并发和数据边界。
- [搜索与审计链](search-audit.md) - `buzz-search` 提供社区隔离的 FTS5 与 LanceDB 检索，`buzz-audit` 提供按社区分区的哈希链审计记录。
