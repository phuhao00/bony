# 06. 分级实战作业

> 本章是第一阶段（01–05 真实调用链）的验收作业。每道题都要求：定位权威实现 → 先写测试 → 最小修改 → 用真实命令验证 → 复述所有权与风险。全部在本仓库完成。
>
> 工作流（与项目开发规范一致）：

```mermaid
flowchart LR
    Q["提出问题"] --> C["rg 定位权威实现"]
    C --> T["先写测试"]
    T --> I["最小 Rust 修改"]
    I --> V["cargo test -p <crate>"]
    V --> P["性能/内存证据"]
    P --> R["复述所有权和风险"]
```

**纪律**（来自 `docs/PROJECT_STANDARDS.md`，也是真实项目要求）：

- 只改任务需要的文件，不顺手大清理。
- 写之前先搜：同位逻辑是否已存在（`rg` 全仓库）。
- 编译测试都从仓库根走 `cargo -p <crate>`，不开第二个 target。
- 不 `cargo clean`；验证失败先读错误，不盲重试。

---

## L1 · 入门（单文件，30 分钟级）

### L1-1 给 `PauseKind` 加遥测友好的方法

在 `crates/codegen/xai-workflow/src/run.rs`：

1. 为 `PauseKind` 增加 `fn is_human_initiated(self) -> bool`（只有 `User` 为 true）。
2. 写单元测试覆盖全部 5 个变体。
3. `cargo test -p xai-workflow`。

**验收**：测试过；没有 clone；match 穷尽。

### L1-2 错误消息质量审计

在 `xai-workflow` 中找出所有 `#[error("...")]`，检查每条是否携带了排障所需的数据（seq、limit、行号……）。选一条最「干」的错误消息，补充上下文并加测试断言 `to_string()` 包含关键字段。

**验收**：改一处、测一处；能说出「错误消息是给半夜值班的人看的」。

---

## L2 · 进阶（跨函数，2–4 小时级）

### L2-1 Tool 流契约测试补全

在 `crates/common/xai-tool-runtime/tests/`：

1. 阅读现有 `tool_blocking.rs` 的 fake tool（`BlockingOk`/`BlockingErr`）。
2. 新增一个 fake tool：`execute` 发出 3 个 `Progress` 再发 `Terminal`，写测试断言形状 `[Progress, Progress, Progress, Terminal]` 与顺序。
3. 再写一个**违约** fake（发两个 Terminal），观察测试如何失败，体会契约测试的价值。

**验收**：`cargo test -p xai-tool-runtime`；测试名描述行为不描述实现。

### L2-2 Sampler 的乱序 Cancel

在 `xai-grok-sampler` 中：

1. 找到 `handle_command` 处理 `Cancel` 的路径（`actor/mod.rs` → `state.rs`）。
2. 回答问题并写测试：对一个**已完成**的 request_id 发 Cancel 会发生什么？对一个**从未提交**的 id 呢？应该 panic 吗？
3. 如果现有行为是静默忽略，写测试钉住它。

**验收**：测试覆盖「cancel 未知 id」；能解释为什么幂等的 cancel 对分布式调用方重要。

### L2-3 Cow 的性能账

在 `crates/codegen/xai-grok-secrets/src/sanitizer.rs`：

1. 阅读 `redact_secrets`（返回 `Cow<'_, str>`）。
2. 写基准或测试：对 1 MB 无密钥文本调用 1000 次，断言**零分配**难以直接测——改为断言 `matches!(result, Cow::Borrowed(_))`。
3. 思考：如果把返回值改成 `String`，调用侧要为「无密钥」这个 99% 场景付出什么代价？

**验收**：新增测试证明 Borrowed 快速路径存在。

---

## L3 · 综合（跨 crate，一天级）

### L3-1 fork copy 的新过滤维度

背景：`copy.rs` 的 `surviving_line_indexes` 现在按 rewind + prompt 截断过滤。假设需求：fork 时**丢弃所有工具进度事件、只保留终态**。

1. 不改生产代码，先在 `copy_tests.rs` 写失败测试（合成 JSONL → fork → 断言目标文件没有进度行）。
2. 找到唯一权威实现点（`rewind_step_for_line` / 分类处），最小扩展。
3. `cargo test -p xai-grok-shell` 全绿；跑一次内存 soak（第 05 章命令）确认没有整体物化。

**验收**：先测后改；只动分类/过滤一处；RSS 预算仍过。

### L3-2 为硬拦截加一个「审计日志」字段

背景：`buzz-acp` 的 `BUZZ_ACP_DENY_TOOLS` 拒绝工具调用时（`handle_permission_request`），目前只 `tracing::warn!`。需求：让拒绝原因进入结构化遥测。

1. 读 `third_party/buzz/crates/buzz-acp/src/acp.rs` 的 `is_denied_tool_call` 与权限响应路径。
2. 设计最小改动：复用现有遥测通道（先搜！），不新建并行机制。
3. 写测试覆盖「命中 deny → 响应为 reject_once」（已有 `find_reject_once_fallback_when_no_allow_once` 等测试可参考）。

**验收**：无复制粘贴实现；`cargo test -p buzz-acp` 过。

---

## 每次作业的报告模板

```text
任务：一句话目标
定位：权威实现文件:行（rg 命令）
变更：最小 diff 摘要
验证：cargo 命令 + 实际输出要点
所有权：改动数据现在归谁拥有/借用
风险：未覆盖平台、边界、并发场景
```

> 完成 L1–L3 后，进入第二阶段（07–16）把语言知识逐块补全；每章继续用本仓库代码做实验。

> 下一阶段：[07. 基础语法、表达式、模式与业务建模](07-language-foundations.md)
