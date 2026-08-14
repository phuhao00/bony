---
name: buzz-agent-contracts
description: >-
  Create or revise built-in and user-created Buzz agents, capability profiles,
  registry lifecycle, specialist prompts, subscriptions, routing, handoffs,
  memory participation, and tool guardrails. Use when adding agents, enabling
  user-defined agents, changing prompts or @Agent orchestration, evolving the
  agent catalog, or hardening ACP permissions and meta-message filtering.
---

# Buzz Agent 契约

把角色设计成可发现、可路由、可演进、可验证的窄职责契约；内置 Agent 只是默认实现，不能成为封闭枚举。

## 读取基线

先读取：

- `docs/PROJECT_STANDARDS.md`
- `docs/AGENT_COLLABORATION.md`
- `docs/buzz-room-collab.md`
- `docs/buzz-room-agent-orchestration-plan.md`
- `desktop/src-tauri/prompts/AGENTS.md`
- `desktop/src-tauri/src/managed_agents/types.rs`
- 目标 agent prompt、persona/managed-agent/catalog、ACP 权限与相关测试

涉及动态注册、用户自建 Agent、能力 schema 或兼容迁移时，再读取 [`references/dynamic-agent-registry.md`](references/dynamic-agent-registry.md)。

## 先复用现有模型

- `AgentDefinition` 是定义级行为真相；`ManagedAgentRecord` 是实例与运行状态。
- `AcpRuntimeCatalogEntry` / custom harness 管运行时；`CatalogSource` 管共享来源；`TeamRecord` 管组合。
- 不另建一套平行 manifest、角色表或密钥存储。新元数据优先作为带默认值的可选字段扩展现有定义。
- 稳定 `id/slug` 用于引用；`display_name` 只用于展示，不参与权限或唯一性判断。

## 角色契约

为每个新增或修改角色明确：

1. 稳定身份：不可变 `id/slug`、可变显示名、来源和 schema 版本。
2. 能力声明：稳定、命名空间化的 capability ID，以及输入/输出契约；名称和 prompt 不能替代能力声明。
3. 唯一职责：一句话说明它比其它候选更适合做什么。
4. 唤醒条件：`subscribe=all` 或 `subscribe=mentions`，以及允许的明确入口。
5. 输入契约：开始工作所需正文、路径、参数和上下文；缺失时如何返回。
6. 工具边界：允许的具体工具名、禁止的工具类别和权限模式。
7. 输出契约：结果格式、证据、工件路径和完成判定。
8. 交接契约：下一能力、候选责任人、传递的完整内容、失败时回调对象。
9. 空操作：没有可执行输入时保持静默，不发身份说明或等待文学。

## 路由规则

- 用户明确选择某个已授权 Agent 时优先尊重选择。
- 否则先按 capability、输入兼容、权限、健康状态过滤，再按历史质量、负载和用户偏好排序。
- 简单单域任务直接给得分最高的 Agent，不先讨论。
- 检索默认 `@ZeroClaw`；编码由 Coding Workspace 里的 ACP Agent 自己做。
- 每条 Buzz 消息最多一个 `@Agent`；不得在同帖预唤醒后续角色。
- handoff 必须在消息正文中携带下游所需内容，不传不可访问的沙盒附件。
- specialist 完成或阻塞后回调当前 coordinator，除非固定链明确由它直接交给下一能力。
- 只有非固定的多域请求才进入 2~3 行短讨论。

## 用户自建 Agent 默认值

- 创建后先校验定义、运行时、工具和 readiness，再进入自动路由候选集。
- 默认 `subscribe=mentions`、`respond_to=owner-only`、非 coordinator、最小工具权限。
- capability 是能力声明，不是权限授予；有效权限取用户授权、ACP allow/deny、运行时能力和房间策略的交集。
- 未知 schema/capability 可以保留并展示，但不得自动路由或自动提升权限。
- 禁用/归档后立即退出新任务路由；运行中任务按明确的停止/排空策略收尾。

## Prompt 与硬拦分工

以下问题要优先落到 Rust/配置硬约束，并让 prompt 只解释行为：

- 禁止工具：ACP permission / `BUZZ_ACP_DENY_TOOLS`。
- 噪声、自我介绍、等待消息：发布前 meta 过滤。
- 唤醒范围：订阅策略和 mention 路由。
- 数据/权限隔离：类型、存储和服务端校验。

用 `rg` 先搜索现有 denylist、过滤器、seed、agent catalog 和路由测试；扩展权威实现，不复制第二套判断。

## 变更同步

- 内置角色才更新仓库 prompt；用户自建角色通过现有 AgentDefinition/persona/catalog 保存，不写进固定 prompt 目录。
- 更新 `docs/buzz-room-agent-orchestration-plan.md` 的 capability/policy 表；必要时更新 `docs/buzz-room-collab.md`。
- 新角色同步定义、catalog、订阅策略、工具清单、readiness 和 UI 可见性。
- 修改硬拦时在对应 Rust crate 添加回归测试。
- 不新增注册、mint、构建或测试脚本；需要工具能力时写 Rust。

## 验证

- 检查 stable ID、capability、mention、callback 与 catalog 投影一致。
- 覆盖旧定义无 capability、未知新 schema、重复显示名、Agent 离线、工具被拒、归档和并列候选。
- 对触及的 Rust package 运行 `cargo check -p <crate>` 与最窄 `cargo test -p <crate>`。
- 用一条真实路由示例确认每帖只有一个 `@Agent`，且下游拿到完整输入。
