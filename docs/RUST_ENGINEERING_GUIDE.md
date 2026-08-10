# Rust 工程学习与高性能实践

本项目优先使用 Rust 实现原生桌面 AI 编程和本地多 Agent 房间。学习入口是
[Rust 项目实战课程](rust-learning/README.md)，完整知识域见
[Rust 完整知识地图](rust-learning/00-complete-knowledge-map.md)。

性能判断遵循：先定义用户场景和预算，再建立 baseline，使用测试/benchmark/profiling 定位瓶颈，最后以相同条件复测。
课程中的流式 JSONL、Actor、Tool runtime、原子计数和 FFI adapter 都配有 Mermaid 调用图。

## 最小质量闸门

```powershell
cargo metadata --locked --no-deps --format-version 1
cargo check -p <package> --all-targets
cargo test -p <package>
cargo fmt --all -- --check
```

不要把“编译通过”当成性能证明；同时记录测试结果、内存峰值、吞吐、延迟分位数和未验证风险。

