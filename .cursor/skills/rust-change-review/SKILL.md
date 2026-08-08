---
name: rust-change-review
description: >-
  Review repository Rust diffs for correctness, regressions, concurrency,
  performance, architecture boundaries, and missing tests. Use for code review,
  PR review, pre-merge audits, or requests to assess an existing change; do not
  edit files unless the user separately asks to address findings.
---

# Rust 变更评审

只报告可操作、可证明、由本次变更新增的问题；评审默认只读。

## 工作流

1. 固定评审范围。
   - 查看 `git status --short`、目标 diff 和相关提交基线。
   - 识别用户已有但不在本次范围的改动，不把它们混入结论。
2. 读完整行为链。
   - 从变更点读到调用方、错误处理、持久化/协议边界和已有测试。
   - 用 `rg` 检查是否已有权威实现，避免接受平行副本。
3. 检查高风险类别。
   - 正确性：边界值、状态转换、错误映射、资源释放。
   - 并发：锁顺序、取消、背压、任务退出、共享状态可见性。
   - 数据：事务、迁移、序列化兼容、事件 kind/filter、幂等性。
   - 性能：热路径分配、clone、阻塞 I/O、子进程、重复查询。
   - 架构：入口是否过厚、逻辑是否落错 crate、是否扩大 TS/脚本业务。
   - 验证：成功/失败路径测试是否能在错误实现下失败。
4. 验证发现。
   - 能安全运行时，用最窄 `cargo check|test -p <crate>` 或现有测试证明。
   - 不为制造证据修改源码；无法验证时标明推断与缺失条件。
5. 输出。
   - 发现按严重度排序：P0 数据/安全灾难，P1 主路径错误，P2 有条件回归，P3 低风险但应修。
   - 每条包含文件/行、触发条件、影响、原因和最小修复方向。
   - 没有发现时明确写“无可操作问题”，再列残余测试风险。

## 不报告

- 纯风格偏好，除非违反项目硬规则或会导致真实维护风险。
- 与 diff 无关的历史问题。
- 没有触发路径或证据的泛化担忧。
- 仅因为“可以更抽象”而提出的新层次。
