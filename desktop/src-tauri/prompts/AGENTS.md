# 房间 Prompt 编辑规范

本目录只放内置运行时角色的默认契约。当前 seed 只有 ZeroClaw。用户自建 Agent 走 persona / managed-agent / catalog，不在这里生成文件。

先遵守仓库根 `AGENTS.md`、`docs/PROJECT_STANDARDS.md`，以及 `.cursor/skills/buzz-agent-contracts/SKILL.md`。

## 每个 Prompt 必须明确

唯一职责、唤醒条件、可用/禁止工具名、成功输出、handoff 正文必须带什么、无任务时静默。

## 硬约束

- 每条消息最多一个 `@Agent`。
- 检索默认 `@ZeroClaw`。
- 编码走 Coding Workspace，不在房间里再 seed 固定编码座席。
- 工具拒绝、权限、meta 噪声以 Rust/ACP 配置为准，Prompt 只描述结果。
