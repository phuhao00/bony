# 12. 同步并发、Channel 与原子内存序

> **本章学到什么**：有界 `mpsc` 与背压、单 writer 模式、cancellation-safe 的 `select!`、`Mutex` 临界区纪律、原子操作与内存序（Relaxed / Acquire / Release / CAS）、`Send`/`Sync` 实战。
>
> **真实入口**：`third_party/buzz/crates/buzz-pair-relay/src/lib.rs`、`buzz-relay/src/state.rs`、`xai-computer-hub-core`、`xai-circuit-breaker`。

## 1. 业务背景：一个 WebSocket 连接的收发

Bony 的配对中继（`buzz-pair-relay`）给每个 WebSocket 连接做三件事：收消息、按订阅 fan-out、把出站消息写回 socket。出站方向是经典的**单 writer** 问题：多个生产者（各订阅的 fan-out）都想往同一个 socket 写——怎么办？

答案：**只给一个任务写 socket 的权利**，其它人通过 channel 排队。

## 2. 有界 channel + 唯一 writer

连接建立时（`lib.rs:620-637` 节选）：

```rust
async fn handle_conn(relay: Arc<Relay>, conn_id: u64, stream: WebSocketStream<...>) {
    let _guard = ConnGuard { relay: Arc::clone(&relay), conn_id };
    let (sink, mut source) = stream.split();
    let (tx, rx) = mpsc::channel::<OutMsg>(CHANNEL_CAP);   // 有界！CAP=4
    let cancel = CancellationToken::new();
    let writer_handle = tokio::spawn(writer_task(sink, rx, cancel.clone()));
    // ...
}
```

writer 任务（`lib.rs:597-618`）：

```rust
async fn writer_task(mut sink: WsSink, mut rx: mpsc::Receiver<OutMsg>, cancel: CancellationToken) {
    loop {
        let msg = tokio::select! {
            _ = cancel.cancelled() => break,
            m = rx.recv() => match m { Some(m) => m, None => break },
        };
        let ws_msg = match msg {
            OutMsg::Text(s) => Message::Text(s.into()),
            OutMsg::Pong(d) => Message::Pong(d.into()),
            OutMsg::Close => Message::Close(None),
        };
        let result = tokio::select! {
            _ = cancel.cancelled() => break,
            r = timeout(Duration::from_secs(5), sink.send(ws_msg)) => r,
        };
        match result {
            Err(_) => break,      // 发送超时
            Ok(Err(_)) => break,  // 发送失败
            Ok(Ok(())) => {}
        }
    }
}
```

逐个拆解：

- **`mpsc::channel(N)` 有界**：容量就是并发缓冲上限。`Receiver` 唯一（move 进 writer），`Sender` 克隆给所有生产者——多生产者单消费者。
- **每个 await 都与 `cancel.cancelled()` 竞争**：这就是 **cancellation-safe** 写法——无论卡在哪个 await，取消信号来了都能立刻退出。
- **`rx.recv()` 返回 `None`** = 所有 Sender 已 drop → 优雅退出。channel 结构自带关闭语义。
- **发送加 timeout**：慢客户端不能无限拖住 writer。

## 3. 背压：满了不是等，而是做决定

主 relay（`buzz-relay/src/state.rs:449-476`，节选）处理 `try_send` 失败的方式值得背：

```rust
fn try_send_ws_message(&self, conn_id: Uuid, msg: WsMessage) -> bool {
    if let Some(entry) = self.connections.get(&conn_id) {
        let conn = entry.value();
        match conn.tx.try_send(msg) {
            Ok(_) => {
                conn.backpressure_count.store(0, Ordering::Relaxed);
                true
            }
            Err(TrySendError::Full(_)) => {
                let count = conn.backpressure_count.fetch_add(1, Ordering::Relaxed) + 1;
                if count >= conn.grace_limit {
                    tracing::warn!(%conn_id, count, "fan-out: sustained backpressure — cancelling slow client");
                    conn.cancel.cancel();   // 持续慢 → 断开
                }
                false
            }
            Err(TrySendError::Closed(_)) => false,
        }
    } else {
        false
    }
}
```

- **`try_send` 非阻塞**：fan-out 热路径绝不等待某个慢客户端。
- **Full 的处理是策略**：累计背压计数，偶发满给宽限（grace），**持续**满就取消这个客户端——保护整体吞吐优先于单连接。
- `Ordering::Relaxed`：这里只是计数器，不需要跨线程同步语义（见第 5 节）。

## 4. `Mutex`：锁的纪律

共享状态拆成多个小锁而不是一个大锁（`pair-relay lib.rs:104-129`）：

```rust
pub struct Relay {
    subs: Mutex<Vec<Sub>>,
    conn_count: AtomicU32,
    next_conn_id: AtomicU64,
    seen_ids: Mutex<Vec<([u8; 32], tokio::time::Instant)>>,   // 全局去重
    delivered: Mutex<HashMap<[u8; 32], (u32, tokio::time::Instant)>>,
}
```

**check-then-act 必须在同一临界区内**（`lib.rs:745-767` 节选）：

```rust
{
    let mut subs = relay.subs.lock();
    if subs.iter().any(|s| s.p_value == p_value) {
        // #p 已有订阅者：拒绝（错误响应通过 try_send 发，非阻塞所以可在锁内）
        let _ = tx.try_send(OutMsg::Text(make_closed(&client_sub_id, "error: ...")));
        continue;
    }
    if tx.try_send(OutMsg::Text(make_eose(&client_sub_id))).is_err() {
        break 'conn;
    }
    subs.push(Sub { conn_id, sub_id: client_sub_id.clone(), p_value, writer_tx: tx.clone() });
}
```

如果「检查唯一性」和「注册」分成两次加锁，两个并发 REQ 可能都通过检查。临界区里只允许**快操作**；`try_send` 非阻塞所以可以进锁，`.await` 绝不可以。

## 5. 原子操作与内存序

**Relaxed 计数**——只要原子性、不要顺序保证：上面的 `backpressure_count`、这里的连接计数。

**CAS 循环构造单调时钟**，`crates/common/xai-computer-hub-core/src/registry.rs:243-265`：

```rust
static REGISTRATION_CLOCK: AtomicU64 = AtomicU64::new(0);

pub fn next_registration_seq() -> u64 {
    let now_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0);
    let candidate = now_ms << 10;
    let mut prev = REGISTRATION_CLOCK.load(Ordering::Relaxed);
    loop {
        let next = candidate.max(prev + 1);
        match REGISTRATION_CLOCK.compare_exchange_weak(
            prev, next, Ordering::Relaxed, Ordering::Relaxed,
        ) {
            Ok(_) => return next,
            Err(actual) => prev = actual,  // 别人先改了，用实际值重来
        }
    }
}
```

「时间戳左移 10 位 + 保证比上次大」= 混合逻辑时钟：即使 NTP 回拨也不退步。`compare_exchange_weak` 允许虚假失败（在 ARM 上更快），配合 loop 重试。

**Acquire/Release 状态机**，熔断器（`xai-circuit-breaker/src/breaker.rs:298-327` 节选）：

```rust
if now.saturating_sub(claimed) >= lease_millis
    && self.inner.probe_claimed_at_millis
        .compare_exchange(claimed, now, Ordering::AcqRel, Ordering::Acquire)
        .is_ok()
{
    return Ok(());  // 抢到探测权
}
```

内存序心法（够用版）：

| 序 | 何时用 |
|---|---|
| `Relaxed` | 纯计数/统计，不借原子量传递「其它数据已就绪」的信息 |
| `Acquire`（读） | 读到这值后，要看到对方在写之前做的一切 |
| `Release`（写） | 写之前做的事，要对读到这值的人可见 |
| `AcqRel` | CAS 同时读写 |
| `SeqCst` | 拿不准时的安全默认（稍慢） |

「发布-获取」对是锁的底层原理：Mutex 解锁是 Release，加锁是 Acquire。

## 6. 取消安全的工具函数

`sampler/actor/request_task.rs:456-462`：

```rust
async fn sleep_or_cancel(duration: Duration, cancel_token: &CancellationToken) -> bool {
    tokio::select! {
        biased;
        _ = cancel_token.cancelled() => false,
        _ = tokio::time::sleep(duration) => true,
    }
}
```

`biased` 让取消分支优先被 poll。退避等待、轮询间隔等一切「睡一会」都应该可取消——直接 `tokio::time::sleep` 会让取消信号干等。

## 7. 动手练习

1. **追踪 fan-out**：在 `buzz-relay` 里找一条消息从 WebSocket 进入到 `try_send_ws_message` 的路径，说出它经过哪些锁/channel。
2. **背压策略设计**：如果 grace_limit = 1（一次满就断），什么正常场景会被误伤？如果 = 1000，最坏情况是什么？
3. **内存序实验（纸上）**：把 `next_registration_seq` 的 CAS 改成直接 `store(candidate)`，构造两个线程交错使其**退步**的例子。
4. **思考题**：`writer_task` 里如果去掉 `cancel` 分支只留 `rx.recv()`，当所有 Sender 还活着但对端已断开时会发生什么？

## 自检

- [ ] 能搭建「有界 mpsc + 单 writer + select 取消」三件套
- [ ] 理解 try_send 背压策略与 grace 的取舍
- [ ] 知道 check-then-act 必须在同一临界区
- [ ] 能区分 Relaxed / Acquire / Release 的使用场景
- [ ] 会写 `sleep_or_cancel` 这类取消安全原语

> 下一章：[13. 错误、RAII、事务与可靠性](13-errors-raii-reliability.md)
