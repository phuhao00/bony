# 04. Sampler Actor、异步并发与取消

> **本章学到什么**：Actor 模式（唯一状态所有者 + 命令通道）、`mpsc`/`oneshot` channel、`JoinSet` 管理一组任务、`CancellationToken`、RAII 取消守卫、async 是状态机不是线程。
>
> **真实入口**：`crates/codegen/xai-grok-sampler/src/actor/mod.rs`、`handle.rs`、`commands.rs`。

## 1. 业务背景：并发地向模型发请求

会话要采样模型：多个请求可能同时在途（主 turn、压缩摘要、侧边提问），每个请求是一条 HTTP/SSE 长流，可随时被用户取消。问题：

- 在途请求表（谁在跑、怎么取消）放在哪里？
- 怎么避免多个调用方同时改这张表时加一堆锁？

Bony 的答案是 **Actor**：起一个 tokio 任务独占这张表，其他人只许通过 channel 发命令。

## 2. 命令协议：一个 enum 就是整个 API 面

`crates/codegen/xai-grok-sampler/src/commands.rs`：

```rust
/// Commands sent from a SamplerHandle to the actor task.
///
/// Large payloads (`ConversationRequest`, `SamplerConfig`) are boxed so
/// every command stays cheap to copy through the mpsc channel.
pub(crate) enum SamplerCommand {
    Submit {
        request_id: RequestId,
        request: Box<ConversationRequest>,
        config: Option<Box<SamplerConfig>>,
        completion_tx: Option<
            oneshot::Sender<Result<(ConversationResponse, InferenceLatencyStats), SamplingError>>,
        >,
    },
    Cancel { request_id: RequestId },
    UpdateConfig { config: Box<SamplerConfig> },
    IsActive { request_id: RequestId, reply: oneshot::Sender<bool> },
    ActiveCount { reply: oneshot::Sender<usize> },
}
```

注意两点：

1. **`pub(crate)`**：这是 actor 与句柄之间的「内线」，不是公开 API。外部永远走 `SamplerHandle`。
2. **大负载装箱（`Box<...>`）**：channel 传递时会 clone/move 命令，把大 struct 装箱后移动只有一个指针那么大。
3. **查询命令自带 `reply: oneshot::Sender`**：请求-响应模式用一次性 channel 完成——actor 不持有调用方状态，调用方 `await` 自己的接收端。

## 3. Actor 本体：单线程处理命令，派生子任务干活

`actor/mod.rs` 的 `run` 循环（节选）：

```rust
async fn run(mut self) {
    loop {
        tokio::select! {
            biased;
            // 优先清理已完成任务，防止 active_requests 陈旧
            Some(joined) = self.tasks.join_next(), if !self.tasks.is_empty() => {
                match joined {
                    Ok(request_id) => { self.state.remove(&request_id); }
                    Err(join_err) => {
                        tracing::warn!(error = %join_err, "request task panicked or was aborted");
                    }
                }
            }
            cmd = self.cmd_rx.recv() => {
                match cmd {
                    Some(cmd) => self.handle_command(cmd),
                    None => break, // 所有 Handle 都 drop 了 → 收工
                }
            }
        }
    }

    // 退出前取消所有还在跑的任务，防泄漏
    for (_, active) in self.state.active_requests.drain() {
        active.cancel_token.cancel();
    }
    self.tasks.shutdown().await;
}
```

这个循环是 actor 模式的教科书形态：

- **`tokio::select!`** 同时等多个事件：子任务完成（`join_next`）或新命令（`recv`）。`biased` 让前面的分支优先 poll——先清理再收新命令，状态不会陈旧。
- **`cmd_rx.recv()` 返回 `None`** = 所有 `Sender` 都被 drop = 没有任何 Handle 还活着 → actor 自行退出。**关闭语义由 channel 结构天然给出**，不需要额外的 shutdown 标志。
- **`JoinSet<RequestId>`**：actor 把每个请求 `spawn` 成独立子任务并登记在这里；任务返回值就是 `RequestId`，完成时按 id 清理。
- **退出前清理**：`drain()` 出所有在途请求逐个 cancel，再 `tasks.shutdown()`——资源泄漏防线。

`Submit` 命令的处理（同文件）展示了「重复 id 防御」：

```rust
let cancel_token = CancellationToken::new();
let active = ActiveRequest { cancel_token: cancel_token.clone() };
if let Some(prev) = self.state.register(request_id.clone(), active) {
    // 调用方交了重复 id：取消旧任务，避免泄漏
    prev.cancel_token.cancel();
}
// ...
self.tasks.spawn(request_task::run_request_task(
    request_id, request_inner, effective_config, retry_policy,
    event_tx, cancel_token, completion_tx,
));
```

`CancellationToken` 克隆一份留在 actor 的登记表里，另一份交给子任务——取消时 actor 调 `cancel()`，子任务在自己的 await 点上感知到。

## 4. Handle：廉价克隆的遥控器

`handle.rs`：

```rust
/// Cheaply-cloneable handle to the sampler actor.
///
/// Internally just an `mpsc::UnboundedSender<SamplerCommand>`. All
/// methods are non-blocking (fire-and-forget) except for the
/// `*_async` queries which return a future awaiting an `oneshot::Receiver`.
#[derive(Clone)]
pub struct SamplerHandle {
    cmd_tx: mpsc::UnboundedSender<SamplerCommand>,
}
```

- `Clone` 只是克隆一个 `Sender`（内部引用计数），随手发给任何组件都便宜。
- `submit`/`cancel` 是发后即忘（事件从另一条公共通道流向 UI）；`is_active`/`active_count` 要答复，所以带上 `oneshot`。
- 还有一个 `noop()`：构造一个接收端立即被 drop 的假句柄，供测试和「actor 还没接好线」的占位场景——所有发送点都写 `let _ = ...` 忽略失败，所以安全。

## 5. RAII 取消：`submit_and_collect` 与 `CancelOnDrop`

顺序调用方（压缩、摘要）想要「await 到结果」的体验：

`handle.rs` 的 `submit_and_collect`（节选）：

```rust
pub async fn submit_and_collect(
    &self,
    request_id: RequestId,
    request: ConversationRequest,
) -> Result<(ConversationResponse, InferenceLatencyStats), SamplingError> {
    // RAII 守卫：本 future 被 drop（取消 / panic / 正常返回）时，
    // 通知 actor 取消在途请求。
    struct CancelOnDrop {
        cmd_tx: mpsc::UnboundedSender<SamplerCommand>,
        request_id: RequestId,
    }
    impl Drop for CancelOnDrop {
        fn drop(&mut self) {
            let _ = self.cmd_tx.send(SamplerCommand::Cancel {
                request_id: self.request_id.clone(),
            });
        }
    }

    let (completion_tx, completion_rx) = oneshot::channel();
    let _guard = self.cmd_tx.send(SamplerCommand::Submit { /* ... */ })
        .ok()
        .map(|_| CancelOnDrop { cmd_tx: self.cmd_tx.clone(), request_id: cancel_id });

    completion_rx.await.unwrap_or_else(|_| {
        Err(SamplingError::auth_unknown("sampler actor dropped before completion"))
    })
}
```

这段代码是「**取消安全性**（cancellation safety）」的范本：

- 在 async Rust 里，`await` 的 future 随时可能被外层 drop（用户按了 Stop、父任务被取消）。如果「我这边 future 没了」但「actor 那边请求还在烧 token」，就是资源泄漏。
- `CancelOnDrop` 把「发取消命令」绑定到**栈上守卫的 Drop**：无论 future 以何种方式终结，Drop 都会执行。这和 C++ 的 RAII、Java 的 try-finally 是同一思想，但由类型系统自动接线。
- 注意 `completion_rx.await` 的失败分支：oneshot 发送端被 drop（actor 死了）时返回 `Err`，转成稳定的错误而不是 panic。

## 6. 心法：async 是状态机，不是线程

`SamplerActor::spawn` 只是 `tokio::spawn(actor.run())`——没有新线程被「专门创建」，actor 和几百个其它 future 共享工作线程池，在每个 `.await` 点让出执行权。推论：

1. **不要在 async 里做长阻塞**（同步文件 I/O、`thread::sleep`）——会卡住整个工作线程；用 `spawn_blocking`（第 05 章）。
2. **await 点就是取消点**：`CancellationToken::cancelled()` 通常与业务 future 一起放进 `select!`，谁先就绪走谁。
3. **可变状态只有一个所有者**（actor），所以整条链路没有一个 `Mutex` 保护请求表。

## 7. 动手练习

1. **追一条 Cancel 的完整路径**：从 `SamplerHandle::cancel` → `SamplerCommand::Cancel` → `state.cancel` → `CancellationToken` → `request_task` 内的检测点。说出请求任务在哪个 await 点退出。
2. **画时序图**：Submit（带 `completion_tx`）到 `submit_and_collect` 返回的完整消息流：谁持有哪个 channel 端。
3. **实验**：把 `run()` 循环里 `biased;` 删掉，思考「先收新命令、后清理完成任务」可能让 `active_count` 短暂多报一个——为什么这通常可接受，什么场景下不可接受？
4. **思考题**：为什么命令通道用 `unbounded_channel`？如果改成有界并在 Handle 侧 `.await send()`，actor 模式会被破坏成什么样？（提示：死锁条件。）

## 自检

- [ ] 能说出 actor 模式的三要素（私有状态、命令 enum、channel）
- [ ] 理解 `mpsc` 与 `oneshot` 的分工（多消息流 vs 单应答）
- [ ] 能解释 `CancelOnDrop` 为什么保证取消安全
- [ ] 知道 `recv() == None` 如何驱动 actor 优雅退出
- [ ] 跑通 `cargo test -p xai-grok-sampler`

> 下一章：[05. 流式 JSONL、内存上界与性能证据](05-streaming-io-and-performance.md)
