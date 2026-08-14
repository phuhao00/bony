---
type: 领域服务设计
title: 工作流自动化与媒体存储
description: "`buzz-workflow` 执行频道范围的 YAML 自动化，`buzz-media` 提供经校验的对象存储、处理和归因能力。"
tags: [workflow, media, relay, security]
---
# 工作流自动化与媒体存储

## Workflow

`buzz-workflow` 的 `WorkflowEngine` 解析 `WorkflowDef`、`TriggerDef`、`ActionDef` 与 `Step`，通过 `on_event`、scheduler `run` 触发顺序执行。`ActionSink` 是副作用 seam；Relay 的 `workflow_sink` 实现实际消息/动作投递。

工作流每次执行前都会重新验证 owner 当前频道权限；有 webhook 的外传能力需要 owner/admin，查询失败即拒绝。默认最大并发 100，单 step 默认 5 分钟。approval gate 尚未实现：遇到它会失败，而不是留下无法恢复的 waiting 状态。新增 action 必须定义 schema、executor 行为、权限重检、超时、trace 和 sink 消费者。

## Media

`buzz-media` 不拥有 HTTP handler，handler 在 Relay；它拥有 `MediaConfig`、`MediaStorage`、`process_upload`、`process_file_upload`、`process_video_upload`、`BlobDescriptor` 与 bucket accounting。上传先校验内容/类型，视频走 `validate_video_file` 和 metadata，存储采用对象存储/S3 seam；`serve_inline` 定义安全内联交付边界。

```mermaid
flowchart LR
  Upload["认证上传请求"] --> Handler["Relay media handler"]
  Handler --> Process["buzz-media process upload"]
  Process --> Validate["文件或视频校验"]
  Validate --> Storage["MediaStorage object store"]
  Storage --> Meta["Blob metadata and attribution"]
```

图示 Relay HTTP 边界与 media crate 处理/存储职责的分离。

### Workflow schema 与模板

YAML/JSON `WorkflowDef` 有 event/cron/schedule trigger、顺序 steps 和默认 `enabled`。`ActionDef` 是封闭 action schema：频道动作、延迟、审批与 `call_webhook` 各有对应字段，未知/不匹配 action 形状不能作为可执行定义保存。`validate` 强制唯一且受字符/长度限制的 step id，因为它会变成 evalexpr 变量名；schedule 必须恰有 cron 或 interval。`resolve_template` 解析 `trigger.*` 与 `steps.<id>.output.<field>`；未知变量报错，未闭合 `{{` 保留原文本。`truncate`、`npub`/`truncate_pubkey` 是支持 filter，未知 filter 报错；任意 `call_webhook` 都是 elevated authority，因为可能外传频道数据。

### 媒体发布顺序

`process_buffered_upload` 在 blocking 工作中先验证内容、计算 SHA-256，并以请求绑定 tenant host 取得 Blossom authorization。blob key 是 content-addressed，sidecar key 是 tenant-scoped。只有 blob 与 sidecar 都存在才能幂等短路（仍可写 upload audit record）。新上传依序写 blob、派生 thumbnail/metadata、审核记录、sidecar；metadata 或 audit 失败可留下 blob/派生对象，但没有 sidecar 就不得 serve，这正是 sidecar gate。

验证 workflow 使用 `cargo test -p buzz-workflow`；media 使用 `cargo test -p buzz-media`，其中 `tests/static_creds_minio.rs` 覆盖 MinIO 静态凭据路径。跨 Relay handler 时追加 `cargo test -p buzz-relay`。