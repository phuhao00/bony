---
name: rust-failure-diagnosis
description: >-
  Diagnose Rust compile, test, runtime, concurrency, persistence, ACP, or room
  failures in this repository using reproducible evidence and causal tracing. Use when
  the user asks why something fails, requests root-cause analysis, or wants a bug
  diagnosis; remain read-only unless the user also asks to implement the fix.
---

# Rust 故障诊断

先证明根因，再提出最小修复；诊断请求默认不改代码。

## 工作流

1. 固定症状。
   - 记录实际命令/UI 路径、期望、实际结果、首次出现时间和受影响 package。
   - 查看 `git status --short`，区分用户改动、生成物和故障证据。
2. 最小复现。
   - 从失败入口缩到单个 crate、单个测试或单个事件序列。
   - 使用根 workspace 的 `cargo check|test|run -p <crate>`；不建第二个 target。
   - 不用 `cargo clean`、删数据库或清空缓存来掩盖原因。
3. 沿因果链追踪。
   - 用 `rg` 找错误文本、类型、事件 kind、配置键和调用路径。
   - 从观察到的失败向上追到被破坏的不变量，再定位首次写坏或错误决策的位置。
   - 区分根因、放大因素、后续症状和无关噪声。
4. 证伪候选假设。
   - 每个假设至少找一条支持证据和一条可否定检查。
   - 优先检查边界：异步取消/锁、序列化、SQLite 事务、ACP 权限、跨 crate 类型、环境与 feature。
   - 不能复现时，报告缺失的观测点，不把推测写成结论。
5. 给出修复面。
   - 指出最小权威改动点、需要的回归测试和可能受影响调用方。
   - 只有用户明确要求修复时，转用 `rust-change-delivery` 实施。

## 诊断报告格式

```text
症状：
最小复现：
根因：
证据：
影响范围：
建议修复：
回归测试：
未确认项：
```

根因必须能解释全部关键症状；如果只能解释一部分，继续追踪或明确标注为阶段性结论。
