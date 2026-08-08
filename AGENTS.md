# Agent 须知（bony-build）

完整规范：[`docs/PROJECT_STANDARDS.md`](docs/PROJECT_STANDARDS.md)  
Cursor 规则：`.cursor/rules/`（始终生效）

## 五条硬约束

1. **终止目标**：原生高性能桌面 AI 编程 + 本地多 Agent 房间；交付可验证、无旁路债务。
2. **只用 Rust** 做新实现；`cargo -p` 构建；不扩 TS/脚本业务。
3. **除启动脚本外禁止脚本** — 仅 `start-room-stack` / `start-desktop` / `stop-room-stack`；其它一律 Cargo 或 Rust。
4. **性能优先 + 最短协作路径** — 同进程、单 `target/`、一次一个 `@Agent`、硬拦优先于 prompt。
5. **去冗余 / 模块化** — 搜再写、一次真相、入口薄 / 领域厚；≥2 次重复再抽象，禁止复制并行实现与空框架。

## 编译 / 启动

- 编译：`cargo build -p buzz-desktop`（或相关 crate），根目录，根 `target/`。
- 启动：白名单 `scripts/buzz-room/start-*.ps1`。
- 细节与闸门：`.cursor/skills/buzz-room-build-gate/SKILL.md`（与规则冲突时以 **rules + PROJECT_STANDARDS** 为准）。

## 房间协作

见 `docs/buzz-room-collab.md`、`docs/buzz-room-agent-orchestration-plan.md`。

## 开发 Agent 协作

完整职责、并行边界与交接格式：[`docs/AGENT_COLLABORATION.md`](docs/AGENT_COLLABORATION.md)。

| 任务 | 必用 Skill |
|------|------------|
| Rust 实现 / 修复 / 重构 | `.cursor/skills/rust-change-delivery/SKILL.md` |
| 只诊断根因 | `.cursor/skills/rust-failure-diagnosis/SKILL.md` |
| 性能优化 | `.cursor/skills/rust-performance-optimization/SKILL.md` |
| 代码评审 | `.cursor/skills/rust-change-review/SKILL.md` |
| Buzz Agent / Prompt / 动态能力注册 | `.cursor/skills/buzz-agent-contracts/SKILL.md` |

- 一个文件 / 权威实现点同时只允许一个写入者；共享契约先串行固定，再并行独立模块。
- 调查、实现、评审、验证职责分离；交接必须带实际证据，禁止让下一 Agent 从零重查。
- 同一根 `target/` 不并行跑多个 Cargo 验证任务；不主动 commit/push。
