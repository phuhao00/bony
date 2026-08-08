---
name: rust-performance-optimization
description: >-
  Measure and optimize latency, throughput, memory, allocation, I/O, startup,
  or build-path performance in the repository Rust workspace while preserving
  correctness. Use for profiling, performance regressions, hot-path reviews,
  benchmark work, or explicit requests to make Rust code faster or leaner.
---

# Rust 性能优化

用同一负载的前后数据证明收益；没有基线时不宣称优化成功。

## 工作流

1. 定义指标和负载。
   - 明确测量延迟、吞吐、峰值内存、分配量、I/O、启动时间或编译时间。
   - 固定输入规模、feature、构建 profile、硬件状态和成功判定。
2. 建立基线。
   - 优先复用现有 Criterion、集成测试、tracing 或应用指标。
   - 缺少基准时，用 Rust 增加最小可重复基准；不要新增 Python/Node/PowerShell 性能脚本。
   - 预热后多次运行，记录中位数和离散程度；Debug 与 Release 数据不得混比。
3. 定位主导成本。
   - 先查算法复杂度、跨进程/网络边界、磁盘 I/O 和锁竞争。
   - 再查大缓冲 clone、短命 `String`/`Vec`、重复序列化、过宽 channel、无界队列和多余 wakeup。
   - 不凭直觉批量微调；每次只改变一个主要假设。
4. 落实最小优化。
   - 优先同进程 crate API、批处理、借用/共享缓冲、缩小临界区和消除重复工作。
   - 保持错误语义、取消语义、顺序保证和持久化一致性。
   - 不为跑分关闭默认功能或另建 target。
5. 复测与回归。
   - 使用与基线完全相同的命令和负载。
   - 同时运行正确性测试；并发优化需覆盖取消、背压、关闭和失败重试。
   - 收益未超过噪声时撤回复杂度，不保留无证据优化。

## 交付格式

| 项目 | 内容 |
|------|------|
| 瓶颈 | 主导成本及证据 |
| 基线 | 命令、profile、样本数、指标 |
| 改动 | 唯一权威实现点 |
| 结果 | 同负载前后数据与变化比例 |
| 正确性 | 已运行测试 |
| 风险 | 仍需观察的负载或平台 |
