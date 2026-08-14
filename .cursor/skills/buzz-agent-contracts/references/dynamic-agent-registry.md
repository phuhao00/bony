# 动态 Agent 注册与路由参考

## 目录

- [权威模型](#1-权威模型)
- [能力档案](#2-能力档案)
- [定义与运行状态分离](#3-定义与运行状态分离)
- [确定性路由顺序](#4-确定性路由顺序)
- [用户创建生命周期](#5-用户创建生命周期)
- [安全默认](#6-安全默认)
- [兼容策略](#7-兼容策略)
- [最小测试矩阵](#8-最小测试矩阵)

## 1. 权威模型

扩展现有模型，不创建平行注册表：

| 现有类型 | 权威职责 |
|----------|----------|
| `AgentDefinition` | 定义级身份、prompt、runtime、模型、行为默认值、来源 |
| `ManagedAgentRecord` | 实例身份、密钥引用、运行配置、状态与错误 |
| `AcpRuntimeCatalogEntry` / custom harness | 可启动运行时及 readiness |
| `CatalogSource` | 跨所有者共享定义的来源坐标 |
| `TeamRecord` | 多 Agent 组合与共享说明 |
| Agent snapshot | 可移植定义/实例快照与版本边界 |

未来 capability 元数据应作为 `AgentDefinition` 的可选、带默认值字段演进。不要用 display name、prompt 文本扫描或单独 JSON 文件作为第二真相。

## 2. 能力档案

概念结构（字段名以实现时的 Rust 类型为准）：

```yaml
schema_version: 1
capabilities:
  - id: research.web
    version: 1
    input_kinds: [text.request]
    output_kinds: [text.research]
  - id: document.pdf.create
    version: 1
    input_kinds: [text.body]
    output_kinds: [artifact.pdf]
routing:
  coordinator: false
  automatic: true
```

规则：

- capability ID 使用稳定命名空间，如 `research.web`、`document.pdf.create`、`code.rust.change`、`coordination.route`。
- display name 可重复、可修改；稳定 ID/slug 不随重命名变化。
- capability 版本只描述契约兼容性，不描述 Agent 产品版本。
- capability 声明不授予工具、文件、网络或响应权限。
- 输入/输出 kind 用于路由可组合性；不要把大段 schema 塞入 prompt。

## 3. 定义与运行状态分离

持久化定义保存稳定事实：身份、能力、契约版本、默认行为、来源。

运行时 registry 计算易变事实：

- 是否 active、ready、reachable、busy；
- 当前房间是否授权；
- 运行时/模型是否可用；
- 最近成功率、延迟、用户反馈；
- 是否处于 draining、archived 或 incompatible。

不要把健康状态写回 capability 定义，也不要因临时离线删除定义。

## 4. 确定性路由顺序

1. 用户显式选择已授权 Agent：直接使用。
2. 安全/合规 policy pin：固定到满足要求的能力链。
3. 过滤：active + ready + authorized + capability/version/input 匹配。
4. 排序：能力具体度 → 用户偏好 → 近期质量 → 负载/延迟 → 稳定 ID。
5. 没有候选：回到 coordinator 或请求用户选择，不猜测工具能力。

同分必须用稳定 ID 做确定性 tie-break，避免每次路由漂移。历史质量只能调序，不能绕过权限和输入契约。

## 5. 用户创建生命周期

```text
Draft → Validate → Ready → Active → Draining → Archived → Deleted
```

- Draft：允许编辑，不能自动接任务。
- Validate：校验 stable ID、runtime、capability schema、行为默认值、工具权限与敏感字段。
- Ready：运行时可用，但尚未必加入房间或自动路由。
- Active：可被明确 mention；满足授权后才进入自动路由。
- Draining：不接新任务，允许当前任务完成或超时停止。
- Archived：保留历史/记忆引用，不参与路由。
- Deleted：处理 team、catalog、snapshot、记忆中的引用后再删除。

当前 `is_active`、runtime readiness、managed instance 状态可承载其中大部分语义；新增状态前先确认是否确有第二个独立行为，避免重复枚举。

## 6. 安全默认

- 用户创建：`subscribe=mentions`、`respond_to=owner-only`、非 coordinator、最小工具集。
- `subscribe=all`、`respond_to=anyone`、协调能力、文件写入、终端和网络权限均需用户显式授权。
- Prompt 不能自封 coordinator、修改 capability、扩大权限或改变 catalog provenance。
- 密钥只进现有 keyring/secret 路径，不进 prompt、capability、catalog、snapshot 明文或任务记忆。
- 共享 catalog 是定义分发，不等于信任继承；复制后使用新的本地 ID，并保留 `CatalogSource`。
- 未识别的新 schema 版本 fail closed：可展示、可导出，不自动运行或路由。

## 7. 兼容策略

- 旧定义没有 capability：继续支持显式 mention；内置 Agent 可通过单点兼容映射获得默认 capability。
- 不在多个 prompt/UI 模块各写一份兼容映射；放在 Rust registry 的唯一边界。
- 新增字段使用 serde 默认值；升级时保留旧 snapshot/catalog 的读取能力。
- 破坏输入/输出契约时提升 capability major version；路由器不得把不兼容版本视为同一能力。
- 删除/重命名 capability 时提供迁移映射和截止版本，不静默改义。

## 8. 最小测试矩阵

| 场景 | 预期 |
|------|------|
| 旧定义无 capability | 可显式 mention，不因缺字段加载失败 |
| 两个 Agent 同显示名 | 通过 stable ID 正确区分 |
| 用户明确选择离线 Agent | 给出 readiness 错误，不偷偷换人 |
| 自动路由候选离线 | 选择下一个授权且兼容的候选 |
| capability 匹配但工具未授权 | 拒绝进入候选集 |
| schema 版本高于本机 | 可见但不自动运行 |
| Agent 被归档 | 不接新任务，历史记录仍可解析 |
| coordinator 候选超过一个 | 按房间策略只激活一个，避免路由环 |
