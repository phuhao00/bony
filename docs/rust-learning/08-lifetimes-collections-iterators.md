# 08. 生命周期、集合、迭代器与闭包

> **本章学到什么**：借用规则与生命周期标注（含 `'_` 省略）、`Cow` 写时复制、零拷贝反序列化、常用集合的选型、迭代器链与组合子、闭包捕获与 `move`、`VecDeque` 队列。
>
> **真实入口**：`crates/codegen/xai-grok-secrets/src/sanitizer.rs`、`xai-grok-shell` 会话存储、`xai-circuit-breaker`。

## 1. 借用的三条规则（速记）

1. 任意时刻，要么有一个 `&mut T`，要么有任意多个 `&T`（读写互斥）。
2. 引用必须永远有效（不能比被引用的东西活得久）——编译器用**生命周期**检查这一点。
3. 移动（move）之后，原绑定不可再用。

大部分时候编译器自动推导；需要显式标注的场景是：**函数签名里有多个引用输入、且返回引用**，或**结构体里存引用**。

## 2. `Cow<'_, str>`：零分配的快速路径

日志脱敏是热路径——每条日志都要过。99% 的文本里没有密钥，如果为了「可能要替换」就先复制一份 `String`，纯属浪费。`crates/codegen/xai-grok-secrets/src/sanitizer.rs:94-111`：

```rust
pub fn redact_secrets(input: &str) -> Cow<'_, str> {
    if !MATCH_ANY.is_match(input) {
        return Cow::Borrowed(input);
    }
    let s = PEM_PRIVATE_KEY_REGEX.replace_all(input, REDACTED);
    let s = API_KEY_PREFIX_REGEX.replace_all(&s, REDACTED);
    let s = AWS_ACCESS_KEY_REGEX.replace_all(&s, REDACTED);
    let s = GITHUB_TOKEN_REGEX.replace_all(&s, REDACTED);
    // ... 更多模式 ...
    let s = SECRET_ASSIGNMENT_REGEX
        .replace_all(&s, format!("$1$2$3{REDACTED}"))
        .into_owned();
    Cow::Owned(s)
}
```

`Cow`（Clone on Write）是一个 enum：

```rust
enum Cow<'a, B: ?Sized + 'a> {
    Borrowed(&'a B),   // 零拷贝：指向输入
    Owned(<B as ToOwned>::Owned),  // 真替换了：新分配的 String
}
```

- 无密钥 → `Borrowed(input)`：**零分配**返回。
- 有密钥 → `Owned(s)`：返回新串。
- 调用方统一用 `as_ref()` / `&*result` 拿 `&str`，不用关心哪种情况。

签名里的 `'_'` 是生命周期省略写法：返回值的生命周期与 `input` 相同（`Cow::Borrowed` 借的就是它）。

**判据**：函数「有时需要分配、经常不需要」时，返回 `Cow`。

## 3. 结构体里的生命周期：零拷贝解析

会话存储解析 JSONL 行时，不想为每条消息把 `params` 复制成 `Value`。`crates/codegen/xai-grok-shell/src/session/storage/mod.rs:649-668`：

```rust
pub(crate) fn from_str(line: &str) -> Result<SessionUpdate, serde_json::Error> {
    #[derive(serde::Deserialize)]
    struct BorrowedEnvelope<'a> {
        #[serde(default)]
        method: Option<&'a str>,
        #[serde(borrow)]
        params: &'a serde_json::value::RawValue,
    }

    // Try to parse as envelope first (has "method" + "params")
    if let Ok(envelope) = serde_json::from_str::<BorrowedEnvelope<'_>>(line) {
        let raw_params = envelope.params.get();
        return if envelope.method == Some(XAI_SESSION_UPDATE_METHOD) {
            let notification: SessionNotification = serde_json::from_str(raw_params)?;
            Ok(SessionUpdate::Xai(Box::new(notification)))
        } else {
            let notification: acp::SessionNotification = serde_json::from_str(raw_params)?;
            Ok(SessionUpdate::Acp(Box::new(notification)))
        };
    }
    // Backwards compatibility: legacy format without envelope
    let notification: acp::SessionNotification = serde_json::from_str(line)?;
    Ok(SessionUpdate::Acp(Box::new(notification)))
}
```

关键点：

- `BorrowedEnvelope<'a>` 的两个字段都是**借用**输入行的 `&'a str` / `&'a RawValue`——解析过程零拷贝，`params` 只是指向原 JSON 文本某个区间的指针。
- 结构体一旦存引用，就必须标生命周期：它不能活得比 `line` 久。这里 envelope 是函数内临时值，天然满足。
- `#[serde(borrow)]` 告诉 serde「这个字段可以从输入借」。
- 兼容性设计：先试新格式，失败回退旧格式——**读宽容、写规范**（和第 01 章 `PauseKind` 同一思想）。

## 4. 集合选型

| 类型 | 本项目用例 | 选它的理由 |
|---|---|---|
| `Vec<T>` | 工具列表、survivor 索引 | 顺序 + 追加，缓存友好 |
| `VecDeque<T>` | 熔断器滑动窗口、spawn 队列 | 两端 O(1) 进出（FIFO） |
| `HashMap<K,V>` | 连接表、去重表 | 按键 O(1)，无序 |
| `BTreeSet/BTreeMap` | checkpoint 文件名集合（`copy.rs`） | 有序遍历、确定性输出 |
| `String` vs `&str` | 拥有 vs 借用 | 见第 2 节 |

真实例子：**滑动窗口熔断器**，`crates/common/xai-circuit-breaker/src/window.rs:1-72`（节选）：

```rust
pub(crate) struct SlidingWindow {
    entries: VecDeque<(Instant, bool)>,
    failures: usize,
}

impl SlidingWindow {
    pub(crate) fn push(&mut self, is_failure: bool, at: Instant) {
        if self.entries.len() >= MAX_WINDOW_ENTRIES
            && let Some((_, was_failure)) = self.entries.pop_front()
            && was_failure
        {
            self.failures -= 1;
        }
        self.entries.push_back((at, is_failure));
        if is_failure { self.failures += 1; }
    }

    pub(crate) fn evict(&mut self, window: Duration, now: Instant) {
        let Some(cutoff) = now.checked_sub(window) else { return; };
        while let Some(&(ts, was_failure)) = self.entries.front() {
            if ts < cutoff {
                self.entries.pop_front();
                if was_failure { self.failures -= 1; }
            } else { break; }
        }
    }

    pub(crate) fn error_rate(&self) -> f64 {
        if self.entries.is_empty() { return 0.0; }
        self.failures as f64 / self.entries.len() as f64
    }
}
```

学两点：

1. **`VecDeque` = 双端队列**：`push_back` 进、`pop_front` 出、`front()` 窥视——窗口滑动就是不停从队头驱逐过期项。
2. **增量维护 `failures` 计数**：`error_rate()` 是 O(1)；如果每次全量扫一遍窗口，热路径就是 O(n)。数据结构设计要连着**查询模式**一起想。

## 5. 迭代器链与组合子

`crates/codegen/xai-grok-announcements/src/lib.rs:156-166`：

```rust
/// Return only announcements with non-empty (trimmed) messages.
pub fn visible_announcements(announcements: &[RemoteAnnouncement]) -> Vec<&RemoteAnnouncement> {
    announcements
        .iter()
        .filter(|a| {
            a.message
                .as_ref()
                .map(|m| !m.trim().is_empty())
                .unwrap_or(false)
        })
        .collect()
}
```

- `iter()` 借用遍历（还有 `into_iter()` 消费所有权、`iter_mut()` 可变借用）。
- filter 闭包里用 `Option::as_ref().map().unwrap_or()` 链处理 `Option<String>`——**Option/迭代器的组合子是同一种思维**。
- `collect()` 根据返回类型 `Vec<&RemoteAnnouncement>` 推断收集目标。

消费型版本（`into_iter`），`storage/jsonl/mod.rs:1396-1402`：

```rust
let filtered: Vec<RewindPoint> = points
    .into_iter()
    .filter(|p| p.prompt_index < from_index)
    .collect();
```

`into_iter()` 之后 `points` 被移动，不能再用——所有权语义渗透进迭代器 API。

## 6. 闭包捕获与 `move`

闭包自动捕获用到的变量：默认借用，`move` 强制按值捕获（移动所有权）。

传给另一个线程时必须 `move`，因为闭包可能在调用方栈帧消失后才执行——借用会悬垂，编译器直接拒绝。`storage/jsonl/mod.rs:248-256`：

```rust
async fn append_jsonl_line_blocking(
    path: PathBuf,
    line: Vec<u8>,
    durability: AppendDurability,
) -> io::Result<()> {
    tokio::task::spawn_blocking(move || Self::append_jsonl_line_sync(&path, line, durability))
        .await
        .map_err(io::Error::other)?
}
```

复杂一点的真实形态（同文件 1411-1435）：闭包要用 `&self` 和 `info`，但 `spawn_blocking` 要求 `'static`——解法：**先 clone，再 move**：

```rust
async fn sync_session_files(&self, info: &Info) -> io::Result<()> {
    let info_clone = info.clone();
    let adapter_clone = self.clone();
    tokio::task::spawn_blocking(move || -> io::Result<()> {
        let adapter = adapter_clone;
        let files_to_sync = [
            adapter.updates_file(&info_clone),
            adapter.chat_file(&info_clone),
            // ...
        ];
        for file_path in &files_to_sync {
            if file_path.exists()
                && let Ok(file) = OpenOptions::new().write(true).open(file_path)
            {
                let _ = file.sync_all();
            }
        }
        Ok(())
    })
    .await
    .map_err(io::Error::other)?
}
```

这是 async Rust 的标准范式，值得背下来。

Fn 家族（第 09 章细讲）：`Fn`（只读捕获）⊂ `FnMut`（可变捕获）⊂ `FnOnce`（消费捕获）。`for_each` 要 `FnMut`，`spawn_blocking` 要 `FnOnce + Send + 'static`。

## 7. 动手练习

1. **Cow 练习**：写一个 `normalize_whitespace(s: &str) -> Cow<'_, str>`：无连续空白返回 `Borrowed`，否则压缩为单空格返回 `Owned`。加测试断言两种路径。
2. **生命周期标注练习**：手写 `struct FirstWord<'a> { text: &'a str, start: usize, len: usize }` 和 `fn first_word(text: &str) -> FirstWord<'_>`；`cargo` 不用跑，向别人解释 `'a` 的含义。
3. **集合选型**：`surviving_line_indexes` 返回 `Vec<usize>` 然后 `peekable()` 消费。如果换成 `HashSet` 会丢什么性质？（提示：顺序、`next_if_eq`。）
4. **追踪**：`SlidingWindow::evict` 的 `while let Some(&(ts, was_failure)) = self.entries.front()` 为什么用 `&(...)` 模式？去掉 `&` 编译报什么？

## 自检

- [ ] 能解释 `Cow` 的两个变体与适用场景
- [ ] 理解结构体生命周期标注的含义（不能活得比被借用的久）
- [ ] 会根据读写模式选 Vec/VecDeque/HashMap/BTreeMap
- [ ] 能写出「先 clone 再 move 进 spawn_blocking」的范式
- [ ] 理解 `iter`/`iter_mut`/`into_iter` 的所有权差别

> 下一章：[09. Trait、泛型、高阶约束与高级类型](09-traits-generics-advanced-types.md)
