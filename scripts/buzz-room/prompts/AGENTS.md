# Buzz Room Prompt 编辑规范

本目录只存内置运行时角色的默认契约；用户自建 Agent 通过现有 persona/managed-agent/catalog 保存，不在这里动态生成文件。先遵守仓库根 `AGENTS.md`、`docs/PROJECT_STANDARDS.md`，并使用 `.cursor/skills/buzz-agent-contracts/SKILL.md`。

## 每个 Prompt 必须明确

1. 唯一职责与不处理的领域。
2. 唤醒条件及订阅模式。
3. 可用工具的具体名称。
4. 开始执行所需的完整输入。
5. 成功输出、证据和工件路径。
6. 完成/阻塞时交给谁，以及消息里必须携带什么。
7. 禁止工具、禁止输出和无任务时的静默行为。

## 路由硬约束

- 内置角色名只是默认 capability 实现，不得写成不可扩展的封闭枚举。
- 通用路由规则按 capability 表达；只有安全回归或固定链才 pin 到具体 Agent。
- 每条消息最多一个 `@Agent`。
- 单域请求直接执行；非固定多域请求才短讨论。
- `Grok → ZeroClaw → DocSmith` 是检索后出文档的固定两跳。
- handoff 在正文内携带完整可消费内容；不得只给下游不可访问的附件或沙盒路径。
- specialist 与用户自建 Agent 默认 `subscribe=mentions`；只有被房间策略显式选中的单一协调者使用 `subscribe=all`。
- 除固定链外，specialist 完成或阻塞后回调 `@Grok`。

## Prompt 不能替代引擎

涉及工具拒绝、权限、数据隔离、重复唤醒或 meta 噪声时，先定位 Rust/配置中的权威硬拦，再让 Prompt 描述结果。不要复制一套只能靠模型自觉的业务判断。

## 同步与验证

- 新增/改名内置角色时同步 capability/policy 表、seed/catalog、mention 拼写和 UI 展示名。
- 用户自建 Agent 不要求修改内置 prompt；验证其 stable ID、capability、权限、readiness 与退场状态。
- 改工具边界时同步 ACP allow/deny 配置与 Rust 回归测试。
- 用成功、缺输入、工具拒绝、下游不可达四类示例检查契约。
- 不新增注册、mint、构建或测试脚本；需要运行能力时进入 Rust crate。
