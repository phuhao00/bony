# 13. 错误、RAII、事务与可靠性

> **本章学到什么**：`Result`/`?` 机制、thiserror（库错误）与 anyhow（应用上下文）的分工、panic 边界、Drop guard 的进阶形态、SQLite 事务、原子文件写、重试与退避。
>
> **真实入口**：`third_party/buzz/crates/buzz-db`、`xai-grok-workspace`、`xai-grok-mcp`、`xai-grok-sampler/src/retry.rs`。

## 1. 错误建模：错误也是 enum

第 01 章已预告。`third_party/buzz/crates/buzz-db/src/error.rs:1-55`：

```rust
use thiserror::Error;

/// Errors produced by database operations.
#[derive(Debug, Error)]
pub enum DbError {
    /// A SQLx driver-level error.
    #[error("database error: {0}")]
    Sqlx(#[from] sqlx::Error),

    /// A SQLx migration error.
    #[error("migration error: {0}")]
    Migrate(#[from] sqlx::migrate::MigrateError),

    /// Attempted to store an AUTH event (kind 22242), which is forbidden.
    #[error("AUTH events (kind 22242) must not be stored")]
    AuthEventRejected,

    /// Attempted to store an ephemeral event (kinds 20000–29999).
    #[error("ephemeral event (kind {0}) must not be stored")]
    EphemeralEventRejected(u16),

    /// The requested channel does not exist.
    #[error("channel not found: {0}")]
    ChannelNotFound(uuid::Uuid),

    #[error("serialization error: {0}")]
    Serde(#[from] serde_json::Error),

    #[error("invalid data: {0}")]
    InvalidData(String),
}

/// Convenience alias for `Result<T, DbError>`.
pub type Result<T> = std::result::Result<T, DbError>;
```

拆解：

- **`#[error("...")]`** 生成 `Display`：每个变体自己决定怎么向人描述。
- **`#[from]`** 生成 `From<sqlx::Error> for DbError`：调用处一个 `?` 就把驱动错误包装进领域错误。
- **领域语义变体**：`AuthEventRejected`、`ChannelNotFound(Uuid)`——上层可以 `match` 错误做差异化处理（NotFound 走创建流程、Rejected 直接回绝），而不是解析错误字符串。
- **crate 级 `type Result<T>`**：`db::Result<Channel>` 比 `Result<Channel, DbError>` 简洁；标准库也这么干。

分工口诀：**库用 thiserror 定义稳定错误类型；应用/胶水层用 anyhow 附加上下文**。

## 2. anyhow：给错误链加上下文

`crates/codegen/xai-grok-workspace/src/restore_fetch.rs:351-369`（节选）：

```rust
fn spawn(mut cmd: std::process::Command) -> Result<Self> {
    #[allow(clippy::disallowed_methods)] // enrolled in ProcessScope / Drop teardown below
    let mut child = cmd.spawn().context("spawning git fetch")?;
    let stderr = spawn_stderr_reader(child.stderr.take());

    let mut group = match ProcessGroup::new() {
        Ok(group) => group,
        Err(err) => {
            let _ = child.kill();
            let _ = child.wait_timeout(FETCH_KILL_WAIT);
            return Err(err).context("creating fetch process group");
        }
    };
    if let Err(err) = group.attach_std(&child) {
        let _ = child.kill();
        let _ = child.wait_timeout(FETCH_KILL_WAIT);
        return Err(err).context("attaching fetch to process group");
    }
    // ...
}
```

- `.context("...")` / `.with_context(|| format!(...))` 把「正在做什么」附加到错误链上，原始错误保留在 `source()` 里。
- 注意失败路径的**清理纪律**：创建进程组失败 → 先 kill 已 spawn 的子进程再返回错误。错误处理代码自己也要处理资源。
- `#[allow(clippy::disallowed_methods)]` 加注释：本仓库禁用裸 `Command::spawn`（见根 `clippy.toml`），这里因为后面紧跟 ProcessScope 登记，显式豁免并写明理由——**规则是机器执行的，豁免必须留痕**。

## 3. panic 的边界

- **可恢复的错误走 `Result`**；panic 留给「不变量被破坏、继续运行会更糟」（索引越界、断言失败）。
- tokio 任务的 panic 不会炸掉整个进程：`JoinSet::join_next` 返回 `Err(join_err)`（第 04 章 actor 就是这么记日志的）。
- 库代码尽量不 panic（第 15 章 fuzz 的目标就是「任何输入不 panic」）；`Drop` 里绝不 panic。

## 4. Drop guard 进阶

第 11 章看了两个简单守卫。生产里有三种更复杂的形态：

**a) 状态恢复守卫 + disarm**（`xai-grok-mcp/src/servers.rs:2467-2503`，节选）：

```rust
struct InitGuard<'a> {
    state: &a Mutex<ClientState>,
    init_done: &a Notify,
    /// `Some` until [`Self::disarm`] is called.
    restore: Option<PendingTransport>,
}

impl InitGuard<'_> {
    /// Mark the guard as having published a result. Subsequent `Drop` becomes a no-op.
    fn disarm(&mut self) { self.restore = None; }
}

impl Drop for InitGuard<'_> {
    fn drop(&mut self) {
        let Some(restore) = self.restore.take() else { return; };
        // Best-effort restore. `try_lock` cannot block the runtime from inside Drop.
        if let Ok(mut guard) = self.state.try_lock()
            && matches!(&*guard, ClientState::Initializing)
        {
            *guard = ClientState::Pending(restore);
        }
        self.init_done.notify_waiters();
    }
}
```

MCP 客户端初始化开始时把守卫放栈上：成功路径调 `disarm()`；任何失败/panic 路径 Drop 把状态从 `Initializing` 恢复成 `Pending`，并唤醒等待者——**没有等待者会因为一次失败的初始化永久挂起**。注意 Drop 里用 `try_lock`（Drop 不能阻塞）。

**b) 关闭守卫：cancel + join**（`xai-grok-pager/src/acp/spawn.rs:63-99`，节选）：

```rust
pub struct AgentShutdownGuard {
    cancel: CancellationToken,
    thread: Option<thread::JoinHandle<Result<()>>>,
}

impl Drop for AgentShutdownGuard {
    fn drop(&mut self) {
        self.cancel.cancel();
        let Some(handle) = self.thread.take() else { return; };
        match join_agent_thread(handle, SESSION_FLUSH_GRACE + AGENT_JOIN_SLACK) {
            JoinOutcome::Joined => {}
            JoinOutcome::TimedOut => {
                tracing::warn!("agent worker did not exit within grace after cancel; \
                    SessionEnd teardown (hooks/telemetry/uploads) may be incomplete");
            }
            // ...
        }
    }
}
```

先取消、再限时等待工作线程真正退出——让遥测 flush、上传等 SessionEnd 钩子有机会跑完。

**c) 连接守卫**（第 11 章的 `ConnGuard` 属于此类）：无论函数怎么退出都注销订阅、递减计数。

## 5. SQLite 事务：不 commit 就 rollback

`third_party/buzz/crates/buzz-db/src/channel.rs:112-148`（节选）：

```rust
let mut tx = pool.begin().await?;

sqlx::query(r#"
    INSERT INTO channels (id, community_id, name, channel_type, visibility, ...)
    VALUES ($1, $2, $3, $4, $5, ...)
"#)
.bind(id)
.bind(community_id.as_uuid())
// ...
.execute(&mut *tx)
.await?;

sqlx::query(r#"
    INSERT INTO channel_members (community_id, channel_id, pubkey, role, invited_by)
    VALUES ($1, $2, $3, 'owner', $4)
    ON CONFLICT (community_id, channel_id, pubkey) DO UPDATE SET
        removed_at = NULL, removed_by = NULL, role = EXCLUDED.role
"#)
// ...
.execute(&mut *tx)
.await?;

tx.commit().await?;
```

- `tx` 是 RAII 事务句柄：任何 `?` 提前返回，`tx` 被 drop，自动 **rollback**——「要么全做，要么全不做」由类型系统兜底。
- `&mut *tx`：把事务对象解引用成连接引用（`DerefMut`）。
- `ON CONFLICT ... DO UPDATE`（upsert）让「创建频道 + 把创建者设为 owner」幂等。
- 本项目 SQLite 约定：WAL + busy timeout（见 `xai-sqlite-journal` 与房间栈的 30s 配置）。

## 6. 原子文件写：tmp + rename（+ fsync）

配置文件、checkpoint 的写入要扛进程崩溃。同步版（`xai-grok-shell/src/agent/models/cache.rs:176-192`）：

```rust
pub(crate) fn atomic_write(&self, cache: &ModelsCache) {
    // ...
    let tmp = self.unique_tmp_path();
    if std::fs::write(&tmp, &json).is_ok() {
        if std::fs::rename(&tmp, &self.path).is_err() {
            let _ = std::fs::remove_file(&tmp);
        }
    } else {
        let _ = std::fs::remove_file(&tmp);
    }
}
```

异步加强版（`xai-grok-workspace/src/session/checkpoint_store.rs:246-272`，节选）说明了**为什么 rename 还不够**：

```rust
// Flush the blob to disk *before* the rename: atomic rename gives visibility,
// not data persistence, so without this fsync the durability mechanism (a
// rootfs snapshot carrying these files) could capture a zero-length/short blob.
{
    let mut f = tokio::fs::File::create(&tmp_path).await?;
    f.write_all(&json).await?;
    f.sync_all().await?;          // fsync 数据
}
tokio::fs::rename(&tmp_path, &final_path).await?;
// Best-effort dir fsync so the rename (the new dir entry) is itself durable.
if let Ok(dir) = tokio::fs::File::open(&self.dir).await {
    let _ = dir.sync_all().await;
}
```

三层持久化知识：rename 原子（目录项替换）→ `sync_all` 保证数据落盘 → 目录 fsync 保证目录项本身持久。临时文件名带 pid + 原子序号，避免并发写者互相覆盖。

## 7. 重试：指数退避 + 抖动 + 错误分类

`crates/codegen/xai-grok-sampler/src/retry.rs:99-125`（节选）：

```rust
/// Exponential backoff (2s, 4s, 8s, ..., capped) with +/-20% jitter to
/// prevent thundering-herd retry storms.
pub fn retry_backoff_with_jitter(retry_count: u32) -> Duration {
    let shift = retry_count.saturating_sub(1);
    let base_ms = 2000u64
        .checked_shl(shift)
        .unwrap_or(u64::MAX)
        .min(MAX_RETRY_BACKOFF.as_millis() as u64);
    jittered(Duration::from_millis(base_ms))
}
```

决策是**纯函数**（同文件 `classify_error`，节选）：输入错误 + 重试计数 → 输出 `RetryDecision` enum：

```rust
if err.is_rate_limited() {
    // 429：尊重 Retry-After；重试次数另受 rate_limit_threshold 限制
    // ...
    return RetryDecision::RetryWithBackoff { backoff, is_rate_limited: true };
}
if err.is_retryable() {
    // 5xx/传输错误：第一次重试还要 RetryWithClientRebuild（连接池可能中毒）
    // ...
}
RetryDecision::Fatal(clone_error(err))   // 401/403/404 等不重试
```

三个可迁移的原则：

1. **退避必须带抖动**：一批客户端同时失败时，同步重试会把恢复中的服务再次打爆（惊群）。
2. **重试上限 + 封顶时长**：`checked_shl` 防移位溢出，`min(cap)` 防无限等待。
3. **分类先于重试**：限流、可重试传输错误、致命错误走不同决策；「哪些错误值得重试」是领域知识，写成纯函数就能单测。

## 8. 动手练习

1. **错误审计**：给 `xai-workflow` 的某个函数加一个 anyhow 上下文，或为一个库错误 enum 加变体；跑 `cargo test -p` 确认 `From` 链路通。
2. **事务实验**：在草稿里写「插 A 成功、插 B 失败」的 sqlx 事务，验证 A 被回滚（可用 `:memory:` SQLite）。
3. **守卫练习**：实现 `struct TempDirGuard(PathBuf)`，Drop 时删除目录；写测试用 panic 路径验证清理发生（`catch_unwind`）。
4. **思考题**：`atomic_write` 的失败分支为什么都要 `remove_file(tmp)`？不清理会积累什么问题？（提示：`sweep_stale_tmp`。）

## 自检

- [ ] 能说出 thiserror 与 anyhow 的分工
- [ ] 理解 `#[from]` 与 `?` 的配合
- [ ] 知道事务句柄 drop = rollback 的 RAII 语义
- [ ] 能完整说出「tmp + fsync + rename + dir fsync」每步防什么
- [ ] 理解退避抖动与错误分类的必要性

> 下一章：[14. 宏、属性、derive 与 Serde](14-macros-attributes-serde.md)
