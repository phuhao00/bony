# 14. 宏、属性、derive 与 Serde

> **本章学到什么**：`macro_rules!`（片段说明符、重复、卫生性直觉）、属性宏/derive 的消费、Serde 的协议设计武器库（tag/rename/default/skip/alias/transparent）、线上格式演进的兼容手法。
>
> **真实入口**：`crates/codegen/xai-acp-lib/src/message.rs`、`crates/common/xai-tool-protocol/src/error_wire.rs`、`xai-tool-types/src/task.rs`。
>
> 先说实话：本 workspace **没有自定义 proc-macro crate**（全是 serde/thiserror/tracing/derive_more 等第三方宏的消费者）。日常工程 90% 的场景就是「用好过程宏」；写过程宏是独立的编译器 API 领域，本课程不伪造生产案例。

## 1. `macro_rules!`：声明式代码生成

ACP 协议有十几个「请求类型 → 响应类型 + 方法名」的配对 impl。手写是十几份复制粘贴；用宏一次定义、处处调用。`crates/codegen/xai-acp-lib/src/message.rs:93-108`：

```rust
macro_rules! acp_define_request_response {
    ($request:ty, $response:ty, $method:expr $(,)?) => {
        impl AcpRequest for $request {
            type Response = $response;
        }

        impl AcpMethod for $request {
            fn method_name(&self) -> &'static str {
                $method
            }
        }
    };
}

acp_define_request_response!(acp::ExtRequest, acp::ExtResponse, "ext_method");
acp_define_request_response!(acp::ExtNotification, (), "ext_notification");
```

调用点（同文件 357+）：

```rust
acp_define_request_response!(
    acp::InitializeRequest,
    acp::InitializeResponse,
    acp::AGENT_METHOD_NAMES.initialize,
);
acp_define_request_response!(
    acp::NewSessionRequest,
    acp::NewSessionResponse,
    acp::AGENT_METHOD_NAMES.session_new,
);
// ... PromptRequest / CancelNotification / SetSessionModelRequest ...
```

语法拆解：

| 元素 | 含义 |
|---|---|
| `($request:ty, $response:ty, $method:expr)` | 三个**片段说明符**：类型、类型、表达式 |
| `$(,)?` | 允许（可选的）尾随逗号——宏的 API 友好性细节 |
| `$request` 在展开体中替换 | 宏在 token 层面工作，展开后才参与类型检查 |

什么时候值得写宏：**同一形状的 impl/样板出现 ≥3 次，且不能用泛型/trait 默认方法表达**。宏展开调试用 `cargo expand`（需要安装 cargo-expand）。

## 2. 属性宏：横切逻辑

`crates/codegen/xai-grok-shell/src/extensions/session_admin.rs:38-54`（节选）：

```rust
#[tracing::instrument(skip_all, fields(method = %args.method))]
pub(crate) async fn handle(agent: &MvpAgent, args: &acp::ExtRequest) -> ExtResult {
    if let Some(method) = InternalMethod::from_name(args.method.as_ref()) {
        return handle_internal(agent, args, method).await;
    }
    match args.method.as_ref() {
        "x.ai/session/rename" => handle_session_rename(agent, args).await,
        "x.ai/session/delete" => handle_session_delete(agent, args).await,
        "x.ai/session/fork" => handle_session_fork(agent, args).await,
        // ...
        _ => Err(acp::Error::method_not_found()),
    }
}
```

`#[tracing::instrument]` 在编译期把函数体包进一个 tracing span：每次调用自动有结构化日志（含 `method` 字段），函数体本身一行业务都不改。同类常用属性宏：`#[tokio::test]`、`#[serde(...)]`、`#[derive(...)]`、本仓库 `#[fastrace::trace]`。

顺带看这个函数本身：**字符串方法名 → match 分派 → 未知方法返回标准错误**，这是 JSON-RPC 扩展方法的常见骨架（`_ => method_not_found` 是协议要求，不是偷懒）。

## 3. Serde 武器库：一个 enum 全用上

工具错误的线上格式，`crates/common/xai-tool-protocol/src/error_wire.rs`（节选）：

```rust
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, thiserror::Error)]
#[serde(tag = "code", rename_all = "snake_case")]
pub enum ToolErrorWire {
    #[error("tool not found: {tool_id}")]
    ToolNotFound { tool_id: ToolId },

    #[serde(rename = "forbidden")]
    #[error("permission denied: {reason}")]
    PermissionDenied { reason: String },

    #[serde(rename = "connection_lost")]
    #[error("transport closed for {tool_id}")]
    TransportClosed { tool_id: ToolId },

    #[error("invalid arguments: {message}")]
    InvalidArguments {
        message: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        details: Option<serde_json::Value>,
    },

    /// Free-form forward-compat error. The outer `code` discriminator is
    /// always the literal `"custom"`; the producer-supplier subcode lives
    /// in `subcode` (the field can't be named `code` because it would
    /// collide with the serde discriminator).
    #[error("custom: {subcode} — {message}")]
    Custom {
        subcode: String,
        message: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        details: Option<serde_json::Value>,
    },
    // ...
}
```

属性清单（协议设计的日常弹药）：

| 属性 | 作用 | 本例中 |
|---|---|---|
| `tag = "code"` | 内部标签：判别字段与数据打平 | `{"code": "tool_not_found", "tool_id": ...}` |
| `rename_all = "snake_case"` | 全局命名转换 | `ToolNotFound` → `"tool_not_found"` |
| `rename = "..."` | 单变体/单字段重命名 | Rust 叫 `PermissionDenied`，线上稳定叫 `"forbidden"`（历史名不能动） |
| `default` | 反序列化缺字段时用默认值 | 旧 peer 不发 `details` 也能解析 |
| `skip_serializing_if = "Option::is_none"` | 序列化时省略 None | 线上 JSON 更小、更干净 |

**`rename` 的核心价值**：把「Rust 命名自由」与「线上协议稳定」解耦——代码里随时重构改名，只要 rename 不动，协议不变。

## 4. `alias`：读宽容的协议演进

子 agent 能力模式，`crates/common/xai-tool-types/src/task.rs:147-169`：

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum SubagentCapabilityMode {
    #[serde(alias = "readonly", alias = "readOnly", alias = "read_only", alias = "ReadOnly")]
    ReadOnly,
    #[serde(alias = "readwrite", alias = "readWrite", alias = "read_write", alias = "ReadWrite")]
    ReadWrite,
    #[serde(alias = "Execute", alias = "EXECUTE")]
    Execute,
    #[serde(alias = "All", alias = "ALL")]
    All,
}
```

- **写出**：`rename_all = "kebab-case"` 保证序列化只有一种规范形（`"read-only"`）。
- **读入**：历史客户端用过四种拼写，全部以 `alias` 接住。**宽进严出**再次出现（第 01 章 `PauseKind::from_str` 同款思想）。

`#[serde(default = "fn_name")]` 的兼容用法，`crates/common/xai-tool-protocol/src/turn_hook.rs:36-59`（节选）：

```rust
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BeforeTurnPayload {
    pub turn_number: u64,
    pub model_id: String,
    /// Whether the session is in YOLO / auto-approve mode.
    #[serde(default)]
    pub yolo_mode: bool,
    // ── Extended fields; all `#[serde(default)]` for old-shell / old-workspace interop. ──
    #[serde(default)]
    pub conversation_message_count: usize,
    #[serde(default = "default_session_relationship")]
    pub session_relationship: String,
    #[serde(default = "default_schema_version")]
    pub schema_version: String,
}
```

规则：**新加字段必须带 `#[serde(default)]`**，否则旧版本 peer 发来的消息直接解析失败。注释里明确写着这是为了「old-shell / old-workspace interop」——版本兼容是显式设计目标，不是运气。

## 5. `#[serde(transparent)]` 复习

第 09 章 `SessionId` 用过：newtype 的序列化与内部类型完全一致。**协议层看不见包装，类型层享受类型安全**——两全其美。

## 6. 动手练习

1. **宏练习**：写一个 `impl_display_for_error!` 宏，给多个错误 struct 批量生成 `Display`；与直接 derive thiserror 对比，说出什么场景下宏更合适。
2. **协议设计**：为一个新错误变体 `RateLimited { retry_after_secs: u64 }` 设计 serde 形状：tag 值、是否需要 rename、要不要 default 字段？写出 JSON 样例。
3. **兼容实验**：给 `BeforeTurnPayload` 加一个**不带** `#[serde(default)]` 的新字段，用旧 JSON（缺该字段）写反序列化测试，观察失败；再加上 default 修复。
4. **思考题**：`ToolErrorWire::Custom` 为什么需要存在？没有它，上游新增错误码时下游会发生什么？

## 自检

- [ ] 能读懂 `macro_rules!` 的片段说明符与重复语法
- [ ] 知道本仓库如何消费属性宏（instrument/derive）
- [ ] 掌握 tag/rename/default/skip/alias/transparent 各自用途
- [ ] 理解「新字段必须 default」的兼容纪律
- [ ] 能设计一个向前兼容的线上 enum 形状

> 下一章：[15. Unsafe Rust、FFI、ABI 与安全封装](15-unsafe-ffi-and-platform.md)
