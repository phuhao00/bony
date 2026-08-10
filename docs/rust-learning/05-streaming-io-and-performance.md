# 05. 流式 JSONL、内存上界与性能证据

> **本章学到什么**：`BufReader`/`BufWriter`、逐行流式处理 vs 整体读入、复用缓冲、损坏容忍（torn write）、`spawn_blocking` 桥接同步 I/O、用基准测试和 RSS 上界测试证明性能主张。
>
> **真实入口**：`crates/codegen/xai-grok-shell/src/session/storage/jsonl/copy.rs`、`benches/fork_copy.rs`、`tests/test_fork_copy_memory.rs`。

## 1. 业务背景：fork 一个会话

用户 fork 会话时，要把 `updates.jsonl`（会话全部增量事件的追加日志）复制成新会话的日志。这个文件**无上限**——长会话几十上百 MB。天真写法 `std::fs::read_to_string` + 整体解析会把峰值内存顶到文件大小级别。

`copy.rs` 的文件头注释就是设计决策：

```rust
//! The `updates.jsonl` transcript is unbounded, so the copy streams it line by
//! line: peak memory tracks a single capped line, plus one small per-line
//! record when a prompt cut is requested. Chat history stays materialized: its
//! transforms need random access and the compacted history is bounded by the
//! context window.
```

翻译：**无界的流式过，有界的才整体持有**。

## 2. 逐行读：一个复用的有界缓冲

`copy.rs:56-131`（节选）：

```rust
const MAX_UPDATE_LINE_BYTES: usize = 64 * 1024 * 1024;

fn for_each_jsonl_line<R: BufRead>(
    reader: R,
    f: impl FnMut(usize, &[u8]) -> io::Result<ControlFlow<()>>,
) -> io::Result<()> {
    for_each_jsonl_line_capped(reader, MAX_UPDATE_LINE_BYTES, f)
}

fn for_each_jsonl_line_capped<R: BufRead>(
    mut reader: R,
    cap: usize,
    mut f: impl FnMut(usize, &[u8]) -> io::Result<ControlFlow<()>>,
) -> io::Result<()> {
    let mut buf = Vec::new();
    let mut index = 0;
    let result = loop {
        buf.clear();                                  // 复用同一个 Vec
        let n = reader.by_ref().take(cap as u64 + 1)  // 只读 cap+1 字节
            .read_until(b'\n', &mut buf)?;
        if n == 0 { break Ok(()); }
        if buf.len() > cap && buf.last() != Some(&b'\n') {
            // 超限行：整行丢弃且不保留内容，只留一条 warn
            loop {
                buf.clear();
                let n = reader.by_ref().take(cap as u64).read_until(b'\n', &mut buf)?;
                if n == 0 || buf.last() == Some(&b'\n') { break; }
            }
            continue;
        }
        let line = buf.trim_ascii();
        if line.is_empty() { continue; }
        if f(index, line)?.is_break() { break Ok(()); }
        index += 1;
    };
    result
}
```

五个值得学的点：

1. **`buf.clear()` 复用缓冲**：`clear` 只把长度置零，**不释放容量**——整个循环只分配一次。峰值内存 ≈ 最大一行，而不是所有行之和。
2. **`take(cap + 1)` 探 cap**：只多读一个字节就能判断「这行是否超界」，超界行**不进缓冲**（排空丢弃），病态输入也撑不爆内存。
3. **回调返回 `ControlFlow`**：调用方用 `Break(())` 提前结束，不必抛错或传 bool 标志。
4. **传 `&[u8]` 不传 `String`**：行内容可能是非 UTF-8（损坏的行），分类逻辑必须能容忍；类型选择直接表达了容错需求。
5. **泛型 `R: BufRead`**：同一个函数既服务「文件」也服务「文件的某次重读」，测试里还能喂内存缓冲。

## 3. 逐行写：容错的 `UpdateLineWriter<'a>`

`copy.rs:156-241`（节选）：

```rust
/// Streaming writer for the fork target's `updates.jsonl`. Corruption-tolerant
/// like the load path: a torn or undecodable line is skipped with a warning
/// instead of failing the fork.
struct UpdateLineWriter<'a> {
    writer: BufWriter<std::fs::File>,
    source: &'a Path,
    target_session_id: &'a acp::SessionId,
    copied: CopiedUpdates,
    skipped_lines: usize,
}

impl<'a> UpdateLineWriter<'a> {
    fn copy_line(&mut self, line: &[u8]) -> io::Result<()> {
        let update = match std::str::from_utf8(line).map(SessionUpdateEnvelope::from_str) {
            Ok(Ok(update)) => update,
            Ok(Err(error)) => { self.skip_torn_line(&error); return Ok(()); }
            Err(error) => { self.skip_torn_line(&error); return Ok(()); }
        };
        // ... 过滤编排投影事件、收集 checkpoint 文件名 ...
        let update = transform_session_id_in_update(update, self.target_session_id);
        let envelope = SessionUpdateEnvelope::from_update(&update).map_err(invalid_data)?;
        serde_json::to_writer(&mut self.writer, &envelope).map_err(invalid_data)?;
        self.writer.write_all(b"\n")?;
        self.copied.count += 1;
        Ok(())
    }

    fn finish(mut self) -> io::Result<CopiedUpdates> {
        if self.skipped_lines > 1 { /* 汇总 warn */ }
        self.writer.flush()?;
        Ok(self.copied)
    }
}
```

- **生命周期 `'a`**：writer 借用 `source` 路径和目标 session id——它只是临时工具，不该拥有它们。结构体上标 `'a` 就是向编译器承诺「我不会活得比被借用的东西久」。
- **`finish(mut self)` 按值接收**：调用即消费，之后不可能再写入——「收尾」语义写进了类型。
- **损坏容忍**：解析失败的行**跳过 + 记数**，不让一条 torn write（上次进程崩溃写了一半的行）毁掉整个 fork。注意 warn 策略：第一条带完整错误详情，后续只累计，结束时汇总——日志既不聋也不刷屏。
- **`BufWriter`**：聚合小写入，减少 syscall。

## 4. 两遍扫描：有界索引，不搬数据

需要「截断到某个 prompt 之前」时（`copy_updates_streaming`，`copy.rs:243-283`）：

```rust
Some(target_idx) => {
    // 第一遍：只记录每行的「分类」，得到存活行号集合
    let survivors = surviving_line_indexes(BufReader::new(&mut file), target_idx)?;
    file.seek(io::SeekFrom::Start(0))?;
    // 第二遍：按行号精确复制
    let mut survivors = survivors.into_iter().peekable();
    for_each_jsonl_line(BufReader::new(file), |index, line| {
        if survivors.next_if_eq(&index).is_some() {
            writer.copy_line(line)?;
        }
        Ok(if survivors.peek().is_none() {
            ControlFlow::Break(())
        } else {
            ControlFlow::Continue(())
        })
    })?;
}
```

`surviving_line_indexes` 的注释解释了取舍：

```rust
/// Indexes (in non-empty-line order) of the source lines that survive rewind
/// filtering and the `target_prompt_index` cut, holding one classification per
/// line instead of the lines.
```

**每行只存一个小记录（行号 + 分类），不存行内容**。第一遍判断谁活，第二遍重读文件只写活着的行。内存上界从「文件内容」降到「行数 × 每条小记录」。同一个文件句柄 `seek(0)` 重读，两遍的行号不可能错位。

## 5. 放进 async 世界：`spawn_blocking`

上面的复制是同步代码（标准库文件 I/O），而存储适配器 API 是 async 的。桥接方式（同 crate `storage/mod.rs`）：

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

`spawn_blocking(move || ...)`：把阻塞工作丢到专用线程池，async 运行时的工作线程不被卡住；`move` 把数据所有权移进闭包（跨线程必须拥有，不能借用栈上变量）。第 12 章会从并发视角再看它。

## 6. 性能主张要有证据

「流式、内存有界」不是口号，仓库里有两类测试钉住它。

**基准测试**（`benches/fork_copy.rs`，Criterion）：对 16 MB（可配 `FORK_BENCH_MB`）合成会话测 fork 吞吐：

```rust
let mut group = c.benchmark_group("fork_copy");
group.sampling_mode(SamplingMode::Flat)
     .sample_size(10)
     .measurement_time(Duration::from_secs(30))
     .throughput(Throughput::Bytes(updates_len));
group.bench_function(BenchmarkId::new("copy_session_data", format!("{target_mb}MB")), |b| {
    b.iter(|| { /* fork 一次，black_box(result) */ });
});
```

**RSS 上界测试**（`tests/test_fork_copy_memory.rs`）：后台线程每几毫秒采样进程 RSS，断言 fork 一个 64 MB 会话时内存增长 **低于 48 MB**——「连整体物化一遍文件都会超标」的预算，直接证伪非流式实现：

```rust
let sampler = RssSampler::start();
let result = adapter.copy_session_data(&source, &target, options).await?;
let outcome = sampler.finish().against_budget(budget_mb);
assert!(outcome.within_budget(), /* 带实际数字的失败消息 */);
```

这个测试标了 `#[ignore]`（soak test），需要时显式跑：

```powershell
cargo test -p xai-grok-shell --features test-support --test test_fork_copy_memory -- --ignored --nocapture
```

## 7. 动手练习

1. **算内存账**：`MAX_UPDATE_LINE_BYTES` 是 64 MB。说出 fork 一个 1 GB 日志时的峰值内存组成（提示：行缓冲 + survivors 记录 + BufWriter 默认容量）。
2. **对比实验**（草稿代码即可）：写出 `read_to_string` 版本的 copy，再对照本章流式版，逐项比较峰值内存、错误行为（坏一行会怎样）、可恢复性。
3. **追踪**：`CopySessionResult` 从 `copy_session_data_sync` 返回后，谁消费了 `skipped_lines`？用户能看到吗？
4. **思考题**：为什么超限行选择「丢弃 + warn」而不是「失败」？如果这是金融对账日志，选择会反过来吗？

## 自检

- [ ] 能解释 `buf.clear()` 复用与峰值内存的关系
- [ ] 理解「两遍扫描、只存索引」的内存权衡
- [ ] 知道何时用 `spawn_blocking`、为什么闭包要 `move`
- [ ] 能说出本项目如何「证明」流式实现（bench + RSS 预算测试）
- [ ] 跑通 `cargo test -p xai-grok-shell --lib`（或任一相关测试目标）

> 下一章：[06. 分级实战作业](06-capstone-labs.md)
