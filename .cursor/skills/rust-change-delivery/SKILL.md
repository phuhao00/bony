---
name: rust-change-delivery
description: >-
  Implement, fix, or refactor product behavior in the repository Rust workspace
  with a minimal diff, a single authoritative implementation, and package-scoped
  verification. Use for requested code changes in bony-*, xai-*, buzz-* or Tauri
  Rust crates; do not use for diagnosis-only or review-only requests.
---

# Rust 变更交付

把用户要求落到正确 crate，以最小改动完成实现、验证和可复现交接。

## 工作流

1. 对齐验收点。
   - 复述可观察行为、必须保持的旧行为和不在范围内的事项。
   - 读取从仓库根到目标目录的 `AGENTS.md`、`.cursor/rules/*` 与命中的专项 Skill。
   - 涉及 Buzz 编译或启动时，同时使用 `buzz-room-build-gate`。
2. 找唯一归属。
   - 先用 `rg` 搜同类类型、函数、错误、配置、测试和调用方。
   - 选已有领域 crate/mod 作为权威实现点；入口只做解析、校验、调用和错误映射。
   - 只有现有边界无法承载稳定职责时才新增模块；不要为单处调用新增框架或 trait 层。
3. 设计最小 diff。
   - 列出要改的权威实现、调用方、测试和公开契约。
   - 跨 crate 时先固定共享类型/API，再改消费者。
   - 若发现用户已有改动与目标重叠，停止覆盖并说明冲突。
4. 实现。
   - 新实现只写 Rust；不新增脚本、解释器 sidecar 或 TS 业务副本。
   - 保持热路径少分配、少 clone、少进程边界；复用已有错误和领域类型。
   - 用小函数表达边界，不做与任务无关的清理。
5. 验证。
   - 先跑最窄测试，再扩大到受影响 package：

     ```powershell
     cargo fmt --check -p <crate>
     cargo check -p <crate>
     cargo test -p <crate> [test-filter]
     ```

   - 只有风险或仓库闸门要求时再跑 `cargo clippy -p <crate> --all-targets -- -D warnings`。
   - 不并行启动多个 Cargo 验证任务争用根 `target/`；不使用嵌套 workspace 或第二个 target。
   - 不因已有无关失败掩盖本次结果；记录命令、退出状态和失败归属。
6. 交接。
   - 先写完成的行为，再写关键文件、验证命令、剩余风险。
   - 给出用户可复现的命令或 UI 路径。

## 停止条件

- 需要删除数据、重写历史、推送或提交，但用户未授权。
- 验收点会导致明显不同的产品行为，且无法从仓库现状推断。
- 正确修复必须扩大到未授权系统或生产环境。

## 完成闸门

- 只有一个权威实现点，没有平行副本。
- 入口薄、领域逻辑落对 crate。
- 相关测试覆盖成功路径和关键失败路径。
- 所有验证结论都有实际命令或可观察证据。
