# 16. 测试、质量闸门、Benchmark 与性能分析

> **本章学到什么**：单元/集成测试组织、fake 与可观测 fixture、契约测试、快照/属性/Fuzz 测试、Criterion 基准、RSS 内存预算、dhat 堆剖析、lint 即规范。
>
> **真实入口**：`crates/common/xai-tool-runtime/tests/`、`crates/codegen/xai-grok-shell/benches/`、`tests/`、`crates/codegen/xai-grok-markdown/fuzz/`、根 `clippy.toml`。

## 1. 测试分层（本仓库实况）

| 层 | 位置 | 例子 |
|---|---|---|
| 单元测试 | crate 内 `#[cfg(test)] mod tests` 或 `*_tests.rs` 伴生文件 | `copy.rs` 的 `#[path = "copy_tests.rs"] mod tests` |
| 集成测试 | crate 的 `tests/` 目录 | `xai-tool-runtime/tests/tool_blocking.rs` |
| soak / 内存测试 | `tests/` + `#[ignore]` | `test_fork_copy_memory.rs` |
| 基准 | `benches/`（Criterion） | `fork_copy.rs` |
| 快照 | insta | `xai-grok-pager` 渲染快照 |
| 属性 | proptest（buzz 子树） | `buzz-conformance/tests/proptest_checker.rs` |
| Fuzz | libFuzzer | `xai-grok-markdown/fuzz/` |

伴生测试文件的习惯（`copy.rs` 头部）：

```rust
#[cfg(test)]
#[path = "copy_tests.rs"]
mod tests;
```

大 crate 里测试单独成文件，源码与测试互不膨胀。

## 2. 契约测试：fake 工具钉住流形状

第 03 章看过结论，这里看手法（`tests/tool_blocking.rs:88-106`）：

```rust
#[tokio::test]
async fn blocking_ok_wraps_into_single_terminal() {
    let tool = BlockingOk;
    let mut stream = tool.execute(ToolCallContext::default(), EchoArgs { text: "hello".into() }).await;
    let first = stream.next().await.expect("expected one item");
    assert!(first.is_terminal());
    match first {
        ToolStreamItem::Terminal(Ok(EchoOutput { text })) => assert_eq!(text, "hello"),
        other => panic!("expected Terminal(Ok), got {other:?}"),
    }
    assert!(stream.next().await.is_none(), "stream should be exhausted");
}
```

`BlockingOk`/`BlockingErr`/`UnimplementedTool` 是同文件里 `impl Tool` 的最小 fake。写契约测试的套路：

1. 为 trait 写**最小的几个 fake**，各自占据契约的一个角落（成功、失败、未实现）。
2. 断言**不变量**（形状、顺序、次数），不断言实现内部。
3. 失败消息带实际值（`got {other:?}`）。

## 3. Fixture 要「暴露可观测性」

验证「HTTP 客户端复用了连接」，fake server 必须**数得清**连接。`crates/codegen/xai-grok-test-support/src/counting_server.rs`（节选）：

```rust
pub async fn spawn_counting_server() -> (String, Arc<AtomicUsize>, Arc<Mutex<Vec<String>>>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();  // 端口 0：系统分配，免冲突
    let base_url = format!("http://{}/v1", listener.local_addr().unwrap());
    let accepts = Arc::new(AtomicUsize::new(0));
    let heads: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
    // ... accept 循环：每次连接 fetch_add，记录请求头 ...
    (base_url, accepts, heads)
}
```

返回句柄（计数器、请求头记录）让测试**断言行为而不是猜测**。日志也一样——把 tracing 做成可计数探针（`xai-test-utils/src/tracing_capture.rs`）：

```rust
/// A `tracing_subscriber::Layer` counting, per registered prefix, the events
/// whose `message` starts with it.
#[derive(Clone)]
pub struct MessagePrefixCounter {
    counters: Arc<Vec<(&'static str, AtomicUsize)>>,
}
```

被测代码导出 `pub const LOG_PREFIX`，测试断言「该路径恰好执行了 N 次」——不用 mock 整个依赖。

## 4. 快照、属性、Fuzz：三种「广覆盖」武器

**insta 快照**（`xai-grok-pager` 的 diff 渲染测试）：

```rust
#[test]
fn snapshot_diff_basic() {
    let outputs = render_diff_hunk_highlighted(&make_hunk(), path, &theme, 80, &config);
    insta::assert_snapshot!("diff_basic", diff_outputs_to_string(&outputs));
}
```

渲染输出与 `snapshots/*.snap` 对比；变化时 `cargo insta review` 人工确认。适合**输出复杂、手写断言太累**的场合。

**proptest 属性测试**（`buzz-conformance/tests/proptest_checker.rs`，节选）：

```rust
proptest! {
    #![proptest_config(ProptestConfig::with_cases(128))]

    /// P2 — completeness / no false reject.
    #[test]
    fn clean_trace_is_accepted((_resolved, trace) in arb_clean_trace()) {
        let sc = Scenario::unstructured(trace);
        prop_assert!(check_trace(&sc).is_ok(), "clean trace was rejected: {:?}", check_trace(&sc));
    }
}
```

属性测试断言**不变量**（「干净的 trace 必被接受」），而不是枚举样例。仓库注释提醒：属性要来自 spec，**不是把实现再抄一遍**当预言机。

**libFuzzer**（`crates/codegen/xai-grok-markdown/fuzz/fuzz_targets/render_all.rs`）：

```rust
fuzz_target!(|data: &[u8]| {
    let Ok(s) = std::str::from_utf8(data) else { return; };
    for pretty in [true, false] {
        let _ = render_markdown_ratatui_full(s, STYLE, pretty, None);
    }
    // 分块流式渲染，轮转 chunk 大小暴露边界 bug
    let mut r = StreamingMarkdownRenderer::new(STYLE, pretty);
    // ... 按 [1,16,32] 轮转切块喂入 ...
});
```

Fuzz 的断言就是「**不 panic**」。markdown 渲染直接吃模型输出——不可信输入必须 fuzz。

## 5. 性能证据链：Criterion → RSS 预算 → dhat

第 05 章引入过，这里给全景。

**Criterion 基准**（`benches/fork_copy.rs`）：`benchmark_group` + `SamplingMode::Flat`（I/O 型负载波动大，用平坦采样）+ `Throughput::Bytes`（报告 MB/s）+ `black_box` 防优化掉结果；环境变量 `FORK_BENCH_MB` 参数化规模。

**RSS 预算测试**（`tests/test_fork_copy_memory.rs`）：后台采样器盯住进程 RSS，断言增长低于预算；预算设得「连整体读一遍文件都会超」，直接证伪非流式实现。`#[ignore]` 标记 + CI 显式 `--ignored` 触发——soak test 不拖慢日常 `cargo test`。

**dhat 堆剖析**（`tests/test_session_load_memory.rs` 头部）：

```rust
#[global_allocator]
static DHAT_ALLOC: dhat::Alloc = dhat::Alloc;
```

dhat 记录每次分配的调用栈与存活时长，测试结束时给出「峰值堆、分配热点」报告——RSS 回答「超没超」，dhat 回答「**谁**在分配」。

## 6. Lint 即规范

根 `clippy.toml`（第 10 章见过 workspace lints，这里看 disallowed-methods）：

```toml
disallowed-methods = [
    { path = "std::fs::canonicalize", reason = "returns \\\\?\\ verbatim paths on Windows; use dunce::canonicalize" },
    { path = "std::path::Path::canonicalize", reason = "... use dunce::canonicalize" },
    { path = "tokio::fs::canonicalize", reason = "... spawn_blocking + dunce::canonicalize" },
    { path = "std::process::Command::spawn", reason = "an unenrolled child outlives its session; use xai_tty_utils::ProcessScope::enroll" },
    { path = "tokio::process::Command::spawn", reason = "... ProcessScope::enroll" },
    { path = "portable_pty::SlavePty::spawn_command", reason = "... enroll_terminal_pid" },
]
```

每条禁令带 `reason` 指向**替代品**。团队的踩坑经验（Windows verbatim 路径、孤儿子进程）变成了 `cargo clippy` 的机器检查——规范不再依赖人记。豁免必须写注释（第 13 章的 `#[allow(clippy::disallowed_methods)]` 例子）。

## 7. 质量工作流（本项目标准链）

```powershell
cargo check -p <crate>          # 1. 先过编译
cargo clippy -p <crate>         # 2. lint 闸门
cargo test -p <crate>           # 3. 单元 + 集成
# 按需：
cargo test -p <crate> -- --ignored --nocapture   # soak / 内存预算
cargo bench -p <crate>                           # Criterion
```

纪律（来自项目规范）：最窄测试先跑、同根 target 不并行跑多个验证任务、失败先读错误不盲重试。

## 8. 动手练习

1. **契约测试**：为第 03 章练习的小 Tool 写流形状测试；再写一个「发两个 Terminal」的违约 fake，确认测试红。
2. **fixture 设计**：设计一个「计数的假 channel」验证某逻辑恰好发了 N 条消息，写出类型签名。
3. **跑一次基准**：`cargo bench -p xai-grok-shell -- fork_copy`（若耗时太长，把 `FORK_BENCH_MB` 设小），读 Criterion 的 HTML 报告。
4. **思考题**：为什么内存测试用 `#[ignore]` 而不是放进普通测试？如果每个 PR 都跑 64MB soak，代价是什么？

## 自检

- [ ] 能说出本仓库测试分层与各自职责
- [ ] 会写 fake + 契约测试
- [ ] 理解快照/属性/Fuzz 各自适合什么输出形状
- [ ] 能解释 RSS 预算测试如何证伪非流式实现
- [ ] 理解 disallowed-methods「规范机器化」的做法

> 第二阶段结束。进入 Agent 开发进阶：[17. Turn 循环：一次消息的完整旅程](17-turn-loop-anatomy.md)
