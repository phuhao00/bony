# 01. 状态建模、类型系统与持久化

> **本章学到什么**：struct 聚合数据、enum 表达互斥状态、`Option`/`Result` 表达缺失与失败、`match` 穷尽检查、serde 标签化持久化、SQLite 打开策略。
>
> **真实入口**：`crates/codegen/xai-workflow/src/run.rs`、`src/journal.rs`、`crates/codegen/xai-sqlite-journal/src/lib.rs`。

## 1. 业务背景：一次工作流运行的「结局」

Bony 内置了一个工作流引擎（`xai-workflow`）：脚本编排多个 agent 调用，运行结束后必须把**结局**记录下来——成功？暂停？预算超了？被取消？失败了？

这些结局**互斥**：一次运行不可能既 `Completed` 又 `Failed`。Rust 用 `enum` 精确表达这件事。

## 2. 真实代码：`WorkflowOutcome`

`crates/codegen/xai-workflow/src/run.rs:40-48`：

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "outcome", rename_all = "snake_case")]
pub enum WorkflowOutcome {
    Completed { result: serde_json::Value },
    Paused { kind: PauseKind, message: String },
    BudgetExceeded { message: String },
    Cancelled,
    Failed { error: String },
}
```

逐行拆解：

| 语言点 | 这里怎么用 | 为什么 |
|---|---|---|
| `enum` 带数据变体 | `Completed { result }`、`Cancelled`（无数据） | 每种结局携带**恰好属于它**的数据；`Failed` 必带 `error`，编译器保证 |
| `#[derive(Debug, Clone)]` | 自动生成打印与复制能力 | 日志、重试都要用到 |
| `Serialize, Deserialize` | serde 过程宏 | 结局要写进 JSON 持久化 |
| `#[serde(tag = "outcome")]` | 内部标签表示 | JSON 里形如 `{"outcome": "failed", "error": "..."}`，判别字段与数据打平 |
| `rename_all = "snake_case"` | `BudgetExceeded` → `"budget_exceeded"` | Rust 类型名用大驼峰，线上协议用小写蛇形，一处声明全局生效 |

对比 C/TypeScript 的「字符串 status 字段 + 可选字段」写法：那边 `status == "failed"` 时 `error` 可能为 `null`，也可能漏填——**非法状态可以表示出来**。Rust 的 enum 让非法状态**无法构造**。

## 3. 配套的小 enum：`PauseKind` 与字符串边界

同文件 `run.rs:3-38`：

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PauseKind {
    User,
    BackOff,
    NoProgress,
    Verification,
    Infra,
}

impl PauseKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::User => "user",
            Self::BackOff => "back_off",
            Self::NoProgress => "no_progress",
            Self::Verification => "verification",
            Self::Infra => "infra",
        }
    }
}

impl std::str::FromStr for PauseKind {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "user" => Ok(Self::User),
            "back_off" | "backoff" => Ok(Self::BackOff),
            "no_progress" => Ok(Self::NoProgress),
            "verification" | "blocked" => Ok(Self::Verification),
            "infra" => Ok(Self::Infra),
            other => Err(format!("unknown pause kind: {other}")),
        }
    }
}
```

注意三个工程细节：

1. **`Copy`**：无数据的 enum 可以按位复制，传参零成本。
2. **`as_str` 的 match 是穷尽的**：明天加一个 `PauseKind::RateLimited` 变体，所有没更新的 `match` 立刻编译失败——这就是「新增状态后遗漏处理」被编译器拦住的样子。
3. **`FromStr` 容忍历史别名**（`"backoff"`、`"blocked"`）：持久化格式会演化，读旧数据时要宽容；写出去时用 `as_str`/serde 保持唯一规范形。**宽进严出**。

## 4. 持久化：Journal 的一条记录

结局和过程记录以 JSONL（每行一个 JSON）落盘。`crates/codegen/xai-workflow/src/journal.rs:11-18`：

```rust
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct JournalEntry {
    pub seq: u64,
    pub kind: String,
    pub req_hash: String,
    pub result: serde_json::Value,
    pub at_ms: u64,
}
```

`struct` 与 `enum` 的分工一目了然：**struct 聚合「同时成立」的字段**（一条日志必然同时有序号、类型、哈希、结果、时间），**enum 表达「非此即彼」**。

Journal 还定义了上限常量（`journal.rs:6-7`）：

```rust
pub const MAX_JOURNAL_BYTES: u64 = 64 * 1024 * 1024;
pub const MAX_JOURNAL_ENTRIES: usize = crate::MAX_HOST_CALLS as usize;
```

以及一个真实生产风格的错误类型 `JournalError`（`journal.rs:20-40`，节选）：

```rust
#[derive(Debug, thiserror::Error)]
pub enum JournalError {
    #[error("journal io: {0}")]
    Io(#[from] std::io::Error),
    #[error("journal parse at line {line}: {error}")]
    Parse { line: usize, error: String },
    #[error("journal restore rejected (limit {limit}): {reason}")]
    UnsafeRestore { limit: u64, reason: String },
    #[error("journal full: appending seq {seq} would exceed the {limit}-byte cap \
             that restore enforces, which would strand the run unresumable")]
    Full { seq: u64, limit: u64 },
    // ...
}
```

提前记住这个形状，第 13 章会展开：`#[error("...")]` 生成 `Display`，`#[from]` 生成 `From` 让 `?` 自动转换，**错误也是 enum**——每种失败携带自己的上下文数据。

## 5. SQLite 侧：enum 带行为

Bony 的 agent 记忆、房间数据都在 SQLite 里。`xai-sqlite-journal` 用一个 enum 决定「这个库用什么日志模式打开」，`crates/codegen/xai-sqlite-journal/src/lib.rs:26-35`：

```rust
/// Journal mode chosen for a SQLite database based on where it lives.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum JournalMode {
    /// Write-ahead logging — the historical default, local filesystems only.
    Wal,
    /// Rollback journal truncated (not unlinked) at commit — safe on network
    /// filesystems...
    Truncate,
}
```

背景是一个真实事故：WAL 模式的 `-shm` 索引文件依赖 POSIX 锁与共享内存，`~/.grok` 若被 NFS 多机挂载，对端重建 `-shm` 会让本机读到 SIGBUS。所以：**检测文件系统类型 → 网络文件系统就用 TRUNCATE 模式**。

选择逻辑（同文件 `for_db_path`，节选）：

```rust
pub fn for_db_path(db_path: &Path) -> Self {
    let env = std::env::var("GROK_SQLITE_JOURNAL_MODE").ok();
    match mode_from_env(env.as_deref()) {
        EnvOverride::Mode(mode) => { return mode; }   // 环境变量紧急开关优先
        EnvOverride::Invalid => { /* 大声告警，不静默忽略 */ }
        EnvOverride::Unset => {}
    }
    let dir = match db_path.parent() {
        Some(p) if !p.as_os_str().is_empty() => p,
        _ => Path::new("."),
    };
    if is_network_fs(dir) { Self::Truncate } else { Self::Wal }
}
```

值得学的三点：

- **enum 可以有方法**：`for_db_path`（构造）、`as_str`（映射到 `PRAGMA journal_mode` 的值）、`effective_db_path`（TRUNCATE 模式下改成每主机独立文件名，防止对端旧二进制把共享库切回 WAL）。行为跟着类型走。
- **紧急开关（kill-switch）**：`GROK_SQLITE_JOURNAL_MODE` 允许线上不重新发版就强切模式，而且非法取值**大声 warn** 而不是静默忽略。
- **注释写「为什么」**：每个变体的 doc 注释解释了 WAL 为什么不能上网络盘。半年后读代码的人（包括你）会感谢这种注释。

## 6. `Option` 与 `Result`：缺失与失败的分工

本章代码里两个模式反复出现：

```rust
// 允许缺失 → Option
let env = std::env::var("GROK_SQLITE_JOURNAL_MODE").ok();  // Result 转 Option：没有就是没有

// 操作可能失败 → Result
fn from_str(s: &str) -> Result<Self, Self::Err>
```

口诀：**「可以合理地没有」用 `Option<T>`；「想拿到但可能拿不到/做不成」用 `Result<T, E>`**。环境变量没设置是正常状态（`None`）；解析失败是错误（`Err`），错误必须带原因。

## 7. 动手练习

1. **追踪调用链**：从 `xai-workflow` 的 `run_workflow`（`engine.rs`）出发，找到 `WorkflowOutcome` 是在哪里被构造、又在哪里被序列化的。
   ```powershell
   cargo check -p xai-workflow
   ```
2. **穷尽性实验**：在 `PauseKind` 里临时加一个变体 `RateLimited`，跑 `cargo check -p xai-workflow`，数一数编译器在几处报错（`as_str`、serde 生成的代码……）。体会「新增状态 → 编译器列出所有待改点」。改回去。
3. **写个小测试**：为 `PauseKind::from_str` 写单元测试覆盖别名（`"backoff"` → `BackOff`）和未知值（返回 `Err`）：
   ```powershell
   cargo test -p xai-workflow
   ```
4. **思考题**：`JournalError::Full` 的错误消息为什么要把 `seq` 和 `limit` 都带上？如果只写 `"journal full"`，排障时会缺什么？

## 自检

- [ ] 能说出 struct 与 enum 的分工判据（聚合 vs 互斥）
- [ ] 能解释 `#[serde(tag = "...")]` 输出的 JSON 形状
- [ ] 理解 match 穷尽检查如何防止「新增状态漏处理」
- [ ] 知道 `Option` 与 `Result` 各自适用什么场景
- [ ] 跑通了 `cargo check -p xai-workflow` 与 `cargo test -p xai-workflow`

> 下一章：[02. Agent 的所有权、借用与 Builder](02-agent-ownership-and-builder.md)
