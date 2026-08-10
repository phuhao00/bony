# 18. ACP 协议桥接与会话池（buzz-acp）

> **本章学到什么**：宿主如何经 ACP（Agent Client Protocol）驱动外部 coding agent、JSON-RPC over stdio 的线协议处理、事件队列与并发门禁、权限请求的自动批准与**硬拦截**、用环境变量做运维开关。
>
> **真实入口**：`third_party/buzz/crates/buzz-acp/src/`：`acp.rs`（单会话 ACP 客户端）、`pool.rs`（会话池）、`queue.rs`（事件队列）、`filter.rs`。

## 1. 业务背景：房间消息 → coding agent

Bony 房间里的 `@Grok 帮我改这个项目` 要变成一次真实编码会话：

```text
房间事件(mention + 工程路径标记)
   → buzz-acp 队列排队
   → 会话池找到/启动 agent runtime（Grok CLI / Codex / Claude Code）
   → ACP: initialize → session/new(cwd=工程目录) → session/prompt
   → agent 流式回事件 → buzz-acp 转成房间回复
```

`buzz-acp` 就是这座桥。关键结构（`pool.rs` / `queue.rs`）：

```rust
// queue.rs
pub struct EventQueue { /* 按频道/线程排队房间事件 */ }
pub struct QueuedEvent { /* ... */ }
pub enum CancelReason { /* ... */ }

// pool.rs
pub struct AgentPool { /* 管理所有 agent 的会话生命周期 */ }
pub struct SessionState { /* 单个 ACP 会话：进程、session_id、忙闲、能力 */ }
pub struct OwnedAgent { /* 一个被池子拥有的 agent 实例 */ }
pub struct PromptResult { /* 一次 prompt 的结果 */ }
pub struct SteerRequest { /* 对进行中会话的插话/转向 */ }
```

规模提示：`pool.rs` 约 300KB、`queue.rs` 约 180KB——这是整个系统最复杂的枢纽之一。本章只取三瓢：**线协议处理、权限自动化、硬拦截**。

## 2. JSON-RPC over stdio：用 `serde_json::Value` 走线协议

ACP 是 JSON-RPC 2.0。桥接层不急于把所有消息变成强类型，而是先用 `Value` 做稳健分派——因为**对端是各种外部 CLI，行为不完全受控**。看权限请求处理（`acp.rs:1985+`，节选）：

```rust
async fn handle_permission_request(&mut self, msg: &serde_json::Value) -> Result<(), AcpError> {
    // JSON-RPC 2.0 的 id 允许数字或字符串——按 Value 原样存，别假设类型
    let id = msg.get("id").cloned()
        .ok_or_else(|| AcpError::Protocol("permission request missing id".into()))?;

    // 记住挂起的权限请求，cancel_with_cleanup 才能应答它
    self.pending_permission_id = Some(id.clone());
    self.permission_responded = false;   // 防双应答竞态

    let options = msg["params"]["options"].as_array()
        .ok_or_else(|| AcpError::Protocol("permission request missing options".into()))?;

    let denied = is_denied_tool_call(&msg["params"]["toolCall"]);

    // 按 kind 找选项——绝不硬编码 optionId（doc 注释原话：Critical）
    let allow_once = if denied { None } else {
        options.iter().find(|opt| opt.get("kind").and_then(|k| k.as_str()) == Some("allow_once"))
    };

    let response = if let Some(opt) = allow_once {
        permission_response_selected(&id, opt["optionId"].as_str() /* ... */)
    } else {
        // denied 或没有 allow_once → 找 reject_once；再没有 → 协议错误
        // ...
    };
    // 发送响应 ...
}
```

工程要点：

1. **协议宽容**：id 可以是数字或字符串；选项按 `kind`（语义）查找，不按 `optionId`（实现细节）——对端换版本也不脆。
2. **双应答防护**：`permission_responded` 布尔 + `pending_permission_id` 记录。JSON-RPC 里对同一 id 应答两次是协议错误。
3. **错误即协议异常**：缺字段 → `AcpError::Protocol`，不是 panic。

响应构造（同文件）直接拼 JSON-RPC 形状：

```rust
fn permission_response_selected(id: &serde_json::Value, option_id: &str) -> serde_json::Value {
    serde_json::json!({
        "jsonrpc": "2.0",
        "id": id,
        "result": { "outcome": { "outcome": "selected", "optionId": option_id } }
    })
}
```

何时用 `Value`、何时上强类型？**边界处宽容（Value + 逐字段校验），进入领域后尽快转强类型**——这是桥接层的通用分层。

## 3. 硬拦截：prompt 管不住的，harness 管

背景（`docs/buzz-room-collab.md` 记录的真实回归）：检索专员 ZeroClaw 的外部 CLI 自带 `file_write`/`deliver_file` 工具。即使系统提示写死「把正文贴在消息里」，模型仍可能主动选工具——生成的附件只在它自己的沙盒里，别的 agent 永远读不到（死链接）。

**Prompt 约束治不了「模型主动选工具」。所以不靠 prompt，在权限层硬拦**（`acp.rs:2147+`）：

```rust
/// Tool-name substrings this agent must never be allowed to execute, from
/// `BUZZ_ACP_DENY_TOOLS` (comma-separated, case-insensitive substring match
/// against the tool call's `title`/`kind`/`toolCallId`). Empty when unset —
/// the common case, so most agents pay zero cost for this check.
fn denied_tool_name_tokens() -> Vec<String> {
    std::env::var("BUZZ_ACP_DENY_TOOLS")
        .unwrap_or_default()
        .split(',')
        .map(|s| s.trim().to_ascii_lowercase())
        .filter(|s| !s.is_empty())
        .collect()
}

fn is_denied_tool_call(tool_call: &serde_json::Value) -> bool {
    tool_call_matches_denylist(tool_call, &denied_tool_name_tokens())
}

/// Pure matcher behind `is_denied_tool_call`, split out so tests can check
/// the substring logic without mutating the process-global env var
/// (Rust tests run in parallel; env mutation across threads is racy).
fn tool_call_matches_denylist(tool_call: &serde_json::Value, denied: &[String]) -> bool {
    if denied.is_empty() { return false; }
    let haystack = [
        tool_call.get("title").and_then(|v| v.as_str()),
        tool_call.get("kind").and_then(|v| v.as_str()),
        tool_call.get("toolCallId").and_then(|v| v.as_str()),
        tool_call.get("name").and_then(|v| v.as_str()),
    ]
    .into_iter().flatten().collect::<Vec<_>>()
    .join(" ").to_ascii_lowercase();
    denied.iter().any(|token| haystack.contains(token.as_str()))
}
```

然后 `handle_permission_request` 里 `denied == true` 时直接走 `reject_once`（第 2 节已看到 `allow_once = if denied { None } ...`）。

逐个学习点：

| 设计 | 为什么 |
|---|---|
| 环境变量配置 | 运维开关：不同 specialist 启动时注入不同 deny 列表，不改代码不发版 |
| 空列表零成本 | `denied.is_empty()` 早返回；99% 的 agent 没设 deny，不为安全机制付热路径代价 |
| 四个字段子串匹配 | 不同 agent CLI 把工具名放在不同字段——防御式匹配，宁可多查 |
| 纯函数拆出 | `tool_call_matches_denylist` 不读 env → 可并行单测（注释明确解释：Rust 测试并行跑，改全局 env 有竞态） |
| 拦在**权限层** | 工具调用必须经过 `session/request_permission`——这是外部 agent 的必经之路，一个卡点管所有 |

拦截后的行为闭环：工具被拒 → 模型只好把内容写回消息文本 → 下游 agent 可读。安全机制要设计到「被拦之后模型还能走通正路」。

## 4. 会话生命周期（鸟瞰）

`pool.rs` 的职责（读代码时的地图）：

- **启动**：房间需要某 agent 时，池子 spawn 其 CLI 子进程（stdio），跑 ACP `initialize`，然后 `session/new`——`cwd` 设为消息里 `coding-workspace-v1` 标记携带的工程路径（只接受该标记传来的受信路径）。
- **复用与边界**：同一 agent + 同一工程复用会话；工程切换 → 新会话边界，防止工作目录串线。
- **排队**：`EventQueue` 按频道/线程串行化 prompt（一次一个 turn），`SteerRequest` 允许对进行中的会话插话。
- **回收**：进程退出/超时/取消 → `SessionState` 清理，`cancel_with_cleanup` 还要应答挂起的权限请求（第 2 节存 `pending_permission_id` 的用意）。

## 5. 动手练习

1. **测试阅读**：读 `acp.rs` 尾部测试（如 `find_reject_once_fallback_when_no_allow_once`），说出它们如何用 JSON 字面量模拟 ACP 消息。
2. **deny 列表实验**：为 `tool_call_matches_denylist` 写三个新测试：大小写混合命中、`BUZZ_ACP_DENY_TOOLS` 为空串、多 token 逗号分隔。
3. **追踪**：在 `pool.rs` 里找 `session/new` 的发送点，说出 `cwd` 从哪个字段来、哪里校验它可信。
4. **思考题**：如果把 deny 列表做成硬编码常量而不是环境变量，新增一个自带危险工具的外部 agent 时需要什么流程？两者的运维成本差别？

## 自检

- [ ] 能画出「房间事件 → 队列 → 会话池 → ACP 三步 → 回复」链路
- [ ] 理解边界处 `Value` + 语义查找选项的稳健协议处理
- [ ] 能解释「硬拦优于 prompt 软约束」的原因与实现位置
- [ ] 知道 deny 机制的零成本路径与可测性拆分
- [ ] 理解 pending_permission_id 防双应答/防悬挂的作用

> 下一章：[19. Prompt 装配与上下文工程](19-prompt-assembly-and-context-engineering.md)
