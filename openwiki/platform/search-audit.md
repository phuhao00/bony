---
type: 检索与审计设计
title: 搜索与审计链
description: "`buzz-search` 提供社区隔离的 FTS5 与 LanceDB 检索，`buzz-audit` 提供按社区分区的哈希链审计记录。"
tags: [search, audit, fts5, lancedb]
---
# 搜索与审计链

搜索提高发现能力，但不是授权边界；审计提高可验证性，但不取代业务存储。两者都以 `CommunityId` 分区，依赖 Relay 在输出前执行访问判断。

## 搜索

`SearchService`、`SearchQuery`、`SearchResult` 和 `SearchMode` 处理 SQLite FTS5；`VectorSearchService`、`VectorSearchQuery` 与 `EmbeddingGenerator` 处理嵌入式 LanceDB。每个 query 必须带 `CommunityId`，SQL 的首个谓词绑定该 tenant。FTS index 由数据库 trigger 维护，不存在独立异步索引服务。

搜索命中后，Relay 必须重取 canonical event 并按命中执行访问检查；不能因结果已经带 community id 就跳过私有频道/成员规则。聚焦集成测试是 `crates/buzz-search/tests/fts_integration.rs`。

## 审计

`AuditService` 使用 `AuditEntry`、`NewAuditEntry`、`AuditAction` 与 `compute_hash` 维护 SHA-256 链。每个 community 有独立 sequence 与链头，且 community id 被纳入 hash，避免将记录跨租户搬运后仍能通过验证。DDL 归 `buzz-db` migration，链运算归 `buzz-audit`。

修改检索字段、索引或审计 action 时，先确认 DB migration、service API、Relay caller 及访问后置过滤都同步。最小检查为 `cargo test -p buzz-search` 与 `cargo test -p buzz-audit`。Relay 集成见[Relay 服务](../relay/service-api.md)。