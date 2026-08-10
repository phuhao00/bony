# 03. 类型化 Tool、泛型约束与流式结果

> **本章学到什么**：trait 定义与关联类型、`Send + Sync + 'static` 约束、静态分发 vs 动态分发、blanket impl、`Pin<Box<dyn Stream>>`、异步流契约。
>
> **真实入口**：`crates/common/xai-tool-runtime/src/tool.rs`、`tests/tool_blocking.rs`。

## 1. 业务背景：模型调用工具

Agent 的循环是「模型回复 → 若带 tool_calls → 执行工具 → 结果喂回模型」。工具五花八门（读文件、跑终端、搜代码……），但运行时希望只面对**一个统一接口**：

- 输入：JSON 参数（模型给的）
- 输出：一串进度事件 + 恰好一个终态结果
- 约束：能跨线程跑（tokio 多线程运行时）

这三个要求分别对应 Rust 的三组类型工具。

## 2. `Tool` trait：关联类型让每个工具携带自己的参数/输出类型

`crates/common/xai-tool-runtime/src/tool.rs:32-112`（节选）：

```rust
/// The unified tool trait used by every tool source.
///
/// Implement either `run` (blocking) or `execute` (streaming). The
/// runtime only ever invokes `execute`.
pub trait Tool: Send + Sync {
    /// Typed input. Must be deserialisable from JSON for wire dispatch.
    type Args: for<'de> Deserialize<'de> + JsonSchema + Send + 'static;

    /// Typed output.
    type Output: Serialize + ToolOutput + Send + 'static;

    /// Stable identity used by the runtime to route to this tool.
    fn id(&self) -> ToolId;

    /// Model-facing description and argument schema.
    fn description(&self, _ctx: &ListToolsContext) -> ToolDescription;

    /// Per-tool capability flags (concurrency, scope, frame caps, ...).
    fn capabilities(&self) -> ToolCapabilities {
        ToolCapabilities::default()
    }

    /// Streaming entry point. Default impl wraps `run` into a single-item
    /// stream so blocking tools just override `run`.
    fn execute(
        &self,
        ctx: ToolCallContext,
        args: Self::Args,
    ) -> impl Future<Output = ToolStream<Self::Output>> + Send {
        async move {
            let result = self.run(ctx, args).await;
            terminal_only(result)
        }
    }

    /// Blocking convenience entry point. Default returns an error so a
    /// tool that overrides neither method fails loudly at the first call.
    fn run(
        &self,
        _ctx: ToolCallContext,
        _args: Self::Args,
    ) -> impl Future<Output = Result<Self::Output, ToolError>> + Send {
        async move {
            Err(ToolError::not_implemented(
                "Tool must implement either `run` or `execute`",
            ))
        }
    }
}
```

逐个概念拆：

| 语法 | 含义 | 为什么这么设计 |
|---|---|---|
| `trait Tool: Send + Sync` | 实现者必须能跨线程共享 | 工具在 tokio 工作线程上执行 |
| `type Args` / `type Output` | **关联类型**：每个实现自带参数/输出类型 | 比泛型参数 `Tool<Args, Output>` 更收敛——一个类型只实现一种 Tool |
| `for<'de> Deserialize<'de>` | **高阶 trait 约束（HRTB）**：对任意生命周期 `'de` 都能反序列化 | serde 借用输入缓冲区也能工作；第 09 章展开 |
| `+ 'static` | 类型不含短命借用 | 要放进跨线程的异步任务里 |
| `-> impl Future<...> + Send` | trait 方法返回不透明 future（RPITIT） | 写 `async fn` 风格但可加 `Send` 约束 |
| 两个方法互相默认实现 | 只写 `run` 或只写 `execute` 都行 | 阻塞工具写 `run` 即可，流式工具才需要 `execute` |

**流形状契约**（写在 `ToolStream` 的 doc 里）：

```text
[Progress(_)*, Terminal(Result<T, ToolError>)]
```

零或多个进度事件，**恰好一个**终态。这是运行时、UI 进度条、测试共同依赖的不变量。

## 3. `ToolStream`：三个修饰符叠在一个类型别名里

`tool.rs:114-116`：

```rust
/// Stream of items a tool produces during a single call. Shape:
/// `[Progress(_)*, Terminal(Result<T, ToolError>)]`.
pub type ToolStream<T> = Pin<Box<dyn Stream<Item = ToolStreamItem<T>> + Send>>;
```

从里往外读：

1. `dyn Stream<Item = ...>` —— 类型擦除：具体 stream 类型（map、unfold、自定义 struct）各不相同，统一成 trait 对象才能放进异构集合。
2. `Box<...>` —— trait 对象是 DST（大小未知），必须装在堆上的盒子里才有固定大小。
3. `Pin<...>` —— Stream 的 `poll_next` 要求自引用结构地址稳定；`Pin` 承诺「不会再移动它」。
4. `+ Send` —— future/stream 要能在执行器线程间迁移。

第一次见会觉得密，但这是「trait 方法返回异步流」的**标准工业写法**，以后你会在很多地方再见到它。

## 4. 从静态到动态：`ToolDyn` 与 blanket impl

`Tool` 有 `impl Future` 返回类型和关联类型，**不是 object-safe** 的，不能写 `Vec<Box<dyn Tool>>`。但运行时确实需要一个装着所有工具的异构集合。解法：再造一个 object-safe 的镜像 trait，用 blanket impl 自动桥接。

`tool.rs:304-405`（节选）：

```rust
/// Type erased tool trait. Auto-generated for every typed Tool implementation.
#[async_trait]
pub trait ToolDyn: Send + Sync {
    fn id(&self) -> ToolId;
    fn description(&self, ctx: &ListToolsContext) -> ToolDescription;
    // ...
    /// JSON-typed streaming entry point. The returned stream MUST honour
    /// the same `[Progress*, Terminal]` invariant as [`ToolStream`].
    async fn execute(&self, ctx: ToolCallContext, args: Value) -> ToolStream<TypedToolOutput>;
}

#[async_trait]
impl<T: Tool> ToolDyn for T {
    fn id(&self) -> ToolId { Tool::id(self) }
    // ... 其余方法一一转发 ...

    async fn execute(&self, ctx: ToolCallContext, args: Value) -> ToolStream<TypedToolOutput> {
        let typed_args: T::Args = match serde_json::from_value(args) {
            Ok(v) => v,
            Err(e) => return terminal_only(Err(ToolError::invalid_arguments(e.to_string()))),
        };
        let tool_id = Tool::id(self);
        let typed_stream = Tool::execute(self, ctx, typed_args).await;

        Box::pin(typed_stream.map(move |item| {
            match item {
                ToolStreamItem::Progress(p) => ToolStreamItem::Progress(p),
                ToolStreamItem::Terminal(Ok(out)) => {
                    // 序列化为 JSON + 提取模型可见内容块
                    // ...
                }
                ToolStreamItem::Terminal(Err(e)) => ToolStreamItem::Terminal(Err(e)),
            }
        }))
    }
}

/// Convenience alias for the most common [`ToolDyn`] handle shape.
pub type ArcTool = Arc<dyn ToolDyn>;
```

这个设计的分工值得背下来：

- **`Tool`（静态分发）**：工具作者面对的接口，有类型化 `Args`/`Output`，编译期检查，零开销。
- **`ToolDyn`（动态分发）**：只在「异构集合 / 运行时路由」这个**边界**使用。JSON 进，反序列化成 `T::Args`，调静态实现，输出再序列化回 JSON。
- **`impl<T: Tool> ToolDyn for T`** 是 **blanket impl**：任何实现了 `Tool` 的类型**自动**获得 `ToolDyn`，工具作者完全不用管第二个 trait。

## 5. 测试如何钉住流形状契约

`crates/common/xai-tool-runtime/tests/tool_blocking.rs:88-106`：

```rust
#[tokio::test]
async fn blocking_ok_wraps_into_single_terminal() {
    let tool = BlockingOk;
    let mut stream = tool
        .execute(
            ToolCallContext::default(),
            EchoArgs { text: "hello".into() },
        )
        .await;
    let first = stream.next().await.expect("expected one item");
    assert!(first.is_terminal());
    match first {
        ToolStreamItem::Terminal(Ok(EchoOutput { text })) => assert_eq!(text, "hello"),
        other => panic!("expected Terminal(Ok), got {other:?}"),
    }
    assert!(stream.next().await.is_none(), "stream should be exhausted");
}
```

`BlockingOk` 是同文件里只实现了 `run` 的假工具。这个测试验证的正是第 2 节的契约：**阻塞工具的默认 `execute` 会把结果包成「恰好一个 Terminal」的流**。注意测试手法：断言第一项是 Terminal、断言流随后耗尽——**契约测试**不测实现细节，测不变量。

运行它：

```powershell
cargo test -p xai-tool-runtime --test tool_blocking
```

## 6. 动手练习

1. **写一个最小 Tool**：在草稿里实现 `Tool`，`Args` 为 `{ text: String }`，`run` 返回大写后的文本。对照现有工具（`crates/codegen/xai-grok-tools/src/implementations/` 下任选一个简单工具）检查形状。
2. **读一遍 `ToolDyn::execute`**：说出 JSON → `T::Args` 失败时发生什么（`invalid_arguments` 终态），以及为什么进度事件可以原样透传。
3. **思考题**：为什么 `ToolStream` 需要 `Pin`？如果把 stream 从 `Box` 里「移出来」继续 poll，什么会坏？（提示：自引用 future。）
4. **思考题**：为什么不用 `enum AnyTool { ReadFile(ReadFileTool), ... }` 而用 trait 对象？枚举方案在「插件/MCP 动态注册工具」场景下会遇到什么问题？

## 自检

- [ ] 能说出关联类型与泛型参数的取舍
- [ ] 能逐层解释 `Pin<Box<dyn Stream + Send>>` 四个成分各自解决什么
- [ ] 理解 blanket impl 如何让工具作者只写一份实现
- [ ] 能背出流形状契约 `[Progress*, Terminal]`
- [ ] 跑通 `cargo test -p xai-tool-runtime`

> 下一章：[04. Sampler Actor、异步并发与取消](04-sampler-actor-and-cancellation.md)
