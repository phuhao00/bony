# Buzz Room 多智能体协作 + 记忆规划

> 目标：人在房间里提一个问题/需求 → 相关 agent 先短促讨论「怎么分工、谁来做、什么顺序」→ 按顺序把每一步交给**最擅长**的那个 agent 执行 → 做完之后把结果**总结写回记忆** → 下一次遇到类似请求时，把这段记忆「卷」进新一轮的讨论/分工里，让房间越用越懂这个用户。

这份文档是所有内置与用户自建 Agent 的**架构基线**，不是固定角色名单或一次性脚本。Grok / ZeroClaw / DocSmith / Unity / OpenMontage 只是当前默认实现；后续新增 Agent 应通过能力声明与 registry 自动适配，不要求不断扩写名称分支。

## 1. 现状（已实现，Phase 0）

- **路由**：Grok（`subscribe=all`，房间主脑）用一行 `@Agent` 做单跳指派；specialists（`subscribe=mentions`）只在被 @ 时动。规则见 `scripts/buzz-room/prompts/*.md`。
- **串行两跳**：需要先检索再出文档的请求，固定 `Grok → @ZeroClaw（web_search）→ ZeroClaw → @DocSmith（带完整正文）→ pdf_create`。一条消息只带一个 `@Agent`，避免双 p-tag 同时唤醒两个 agent 抢答。
- **防串戏**：`buzz-acp`（`third_party/buzz/crates/buzz-acp/src/pool.rs::should_suppress_meta_channel_post`）在发布前过滤掉「自我介绍/等待/角色说明」类空话，只保留真正的交付内容。
- **已知回归**（记录以防再犯）：Grok 曾经绕过 ZeroClaw/DocSmith，自己 `read_file`/`list_dir`/`run_terminal_command`（甚至调 `pandoc`）去拼文档，或者把 `docs/` 里昨天的旧 PDF 当成「今天」的答案。修复方式是在 `grok-coordinator.md` 里显式列出禁止清单（工具名 + 场景），而不是只讲原则。**结论：对这类模型，规则要具体到「禁止调用哪个工具、在哪种请求下」，抽象的「别越权」不够用。**

限制：目前**没有**讨论阶段、**没有**持久记忆，路由也主要依赖 prompt 里的静态角色表。现有 Desktop 已支持自定义 persona、managed agent、custom harness、team、catalog 和 snapshot，但尚未形成统一 capability registry；这三点是后续演进重点。

## 2. 目标架构（Phase 1+）

```
人提问
  │
  ▼
⓪ Registry 快照 ── active/readiness/权限/capability/版本/用户显式选择
  │
  ▼
① 记忆检索（卷积核）── 从 memory/task-log 里捞和本次主题相关的过去条目
  │                     （偏好、上次谁做得好、踩过的坑）
  ▼
② 分工讨论（仅复杂/跨域请求触发）
  │   Grok 提议一个「顺序 + 负责人」清单，最多 2~3 行；
  │   涉及的 specialist 可以用一行确认/纠正（"我来做 X，但 Y 部分该给 Z"）；
  │   简单请求（天气、单文件转格式）跳过讨论，直接进④
  ▼
③ 指派（一次一跳，仍然遵守"最多一个 @Agent"规则）
  │   按 capability、授权、健康、质量和负载选择具体 Agent；逐跳 handoff
  ▼
④ 执行
  │   ZeroClaw 检索 / DocSmith 出文档 / Unity 建场景 / OpenMontage 出视频 /
  │   Coding Workspace 中显式选择的 Agent 在所选工程 cwd 内写代码；不再外开独立编码 UI
  ▼
⑤ 记忆写回
  │   任务链最后一个 agent（或 Grok）写一条结构化总结到 memory/task-log.jsonl：
  │   谁参与、顺序、产出路径、用户当场的反馈（喜欢/不喜欢排版等）
  ▼
下一次同类请求 → 回到①，把⑤沉淀的经验"卷"进新一轮讨论
```

「像卷积函数」的落地方式：**不是**训练模型权重，而是每次任务结束都往一个轻量记忆库追加一条摘要；下次讨论阶段先检索相关摘要，当作这次讨论的「先验/kernel」叠加进当前请求的上下文——层层任务的摘要滑动叠加，房间对同一个用户的偏好会越来越准，但实现始终是"读文件 + 拼 prompt"，没有黑箱。

## 3. Capability 路由与策略 Pin

| 任务类型 | Capability / 策略 | 当前默认实现 |
|---|---|---|
| 天气 / 实时信息 | `research.web` / `weather.lookup` | ZeroClaw |
| 正文 → PDF/PPT/Word | `document.*.create` | DocSmith |
| 3D/场景 | `unity.scene.*` | Unity |
| 视频生成/剪辑 | `media.video.*` | OpenMontage |
| 代码分析/改动 | `code.repo.read` / `code.rust.change` | Buzz Coding Workspace 中显式选择的已授权 Agent |
| 跨域组合 | `coordination.route` 先规划，再逐 capability 指派 | 当前 Grok |

“实时检索→文档”仍是安全 policy pin：先 `research.web`，再把完整正文交给 `document.*.create`。当前实例是 ZeroClaw→DocSmith，但兼容的授权 Agent 可以替换具体实现；每帖仍只出现一个 `@Agent`。

路由优先级：用户显式选择 → 安全 policy pin → capability/版本/输入匹配 → 权限/readiness → 历史质量与偏好 → 负载/延迟 → stable ID 确定性 tie-break。

## 4. 动态 Agent 与用户创建

现有权威数据面：

- `AgentDefinition`：定义级身份、prompt、runtime、模型、行为默认值与来源。
- `ManagedAgentRecord`：实例身份、运行配置、状态和错误。
- custom harness / runtime catalog：可执行运行时与 readiness。
- `CatalogSource` / team / snapshot：共享来源、组合与可移植性。

目标扩展是在 `AgentDefinition` 上增加可选、版本化 capability profile；不另建一套 manifest。旧定义没有 capability 时仍可被用户显式 mention，但不自动推断高权限能力。

用户创建生命周期：`Draft → Validate → Ready → Active → Draining → Archived → Deleted`。默认 `subscribe=mentions`、`respond_to=owner-only`、非 coordinator、最小工具权限；`subscribe=all`、`respond_to=anyone`、终端/写文件/网络等均需显式授权。

Prompt 不能自封 coordinator 或扩大权限。Capability 只描述“能做什么”，有效权限始终取用户授权、ACP allow/deny、运行时能力和房间策略的交集。详细契约见 `.cursor/skills/buzz-agent-contracts/references/dynamic-agent-registry.md`。

## 5. 分工讨论阶段（Phase 2，待实现）

**触发条件**（避免把简单请求也拖进讨论，制造话痨）：请求同时满足以下 ≥2 条才触发讨论，否则直接单跳/两跳指派：

- 涉及 ≥2 个不同 specialist 的产出（不算固定的 ZeroClaw→DocSmith 两跳）；
- 请求里包含"先…再…""顺便""同时"等多步骤连接词；
- 用户明确说"你们讨论一下"/"商量一下怎么做"。

**讨论格式**（写入 Grok 与各 specialist 的 prompt）：

```
Grok: 分工提议 — 1) @ZeroClaw 检索X  2) @Unity 建场景  3) @DocSmith 整理成PDF。顺序对吗？
（涉及的 specialist 各回一行确认或改动，不解释身份，不写小作文）
Grok: 确认，开始执行 → @ZeroClaw ...
```

讨论阶段本身也要走 `should_suppress_meta_channel_post` 过滤，防止讨论退化成互相寒暄。

## 6. 记忆存储（Phase 1，先实现这个，价值最高）

新增 `scripts/buzz-room/memory/task-log.jsonl`（append-only，人类可读，不进 keyring/不进 git 敏感区）：

```json
{"ts":"2026-08-07T09:31:00Z","topic":"今天AI资讯PDF","agents":["ZeroClaw","DocSmith"],"outputs":["docs/ai-news-2026-08-07.pdf"],"feedback":"排版满意，字体OK","notes":"用户之前抱怨过留白多/不整齐，已在 pdf_create 里改过版式，本次没有再抱怨"}
```

字段：`ts` / `topic`（自由文本，方便后续检索）/ `agents`（按执行顺序）/ `outputs`（产物路径或链接）/ `feedback`（用户当场反馈，没有就留空）/ `notes`（agent 自己写的一句经验总结，给下次用）。

**写入者**：任务链最后一跳的 agent 在交付消息之后追加一次工具调用式的记忆写入（后续可以做成 `bony-room-tools-mcp` 里的一个 `memory_append` 工具，现在没有就先靠 DocSmith/ZeroClaw 在完成后额外写一行 JSON 到该文件，或由 Grok 统一代写）。

**读取者**：Grok 在①记忆检索阶段，对本次 `topic` 做一次简单的关键词匹配（不需要向量检索，几十条数据用 grep 级别的匹配就够），把匹配到的 1~3 条 `notes` 摘要塞进分工提议里的第一行,例如：`（参考记忆：上次PDF排版反馈是"满意"，按同样版式做）`。

## 7. 分阶段路线图

| Phase | 内容 | 状态 |
|---|---|---|
| 0 | 单跳/两跳路由 + meta 过滤 + Grok DIY 工具禁令 | ✅ 已实现 |
| 1 | `memory/task-log.jsonl` 读写 + Grok 分工提议里带"参考记忆"一行 | 待做 |
| 2 | 触发条件明确的"分工讨论"短对话（≥2 specialist 才触发） | 待做 |
| 3 | 把 `memory_append` / `memory_search` 做成真正的 MCP 工具（而不是靠 prompt 让 agent 手写 JSON），减少格式错误 | 待做 |
| 4 | 定期（比如每周）人工或 Grok 扫一遍 `task-log.jsonl`，把重复出现的偏好固化进对应 specialist 的 prompt，形成"从记忆到规则"的闭环 | 待做 |

扩展性并行路线：

| Track | 内容 | 状态 |
|---|---|---|
| D0 | persona / managed agent / custom harness / team / catalog / snapshot | ✅ 已有基础 |
| D1 | `AgentDefinition` 可选 capability profile + 旧定义兼容映射 | 待做 |
| D2 | Rust registry 统一 readiness、权限、版本和 route eligibility | 待做 |
| D3 | Coordinator 通过 registry 动态选 Agent，支持用户显式 pin 与确定性 fallback | 待做 |
| D4 | 质量/偏好记忆只参与候选调序，不绕过权限 | 待做 |

## 8. 硬约束（贯穿所有 Phase，不因为加了讨论/记忆就放松）

- 一条消息最多一个 `@Agent`；讨论阶段的确认回复也一样。
- 一个房间只允许一个 active `subscribe=all` coordinator；用户 Agent 默认 mention-only。
- 路由按 stable capability/ID，不把 display name、prompt 自述或未知 schema 当权限证据。
- 禁用/归档 Agent 不接新任务；旧定义无 capability 时保持显式 mention 可用但不自动提权。
- Grok 没有文档/搜索工具，永远不能自己 `read_file`/`list_dir`/`run_terminal_command` 去代替 ZeroClaw/DocSmith 完成任务。
- 不解释身份、不写流程小作文、不甩选项菜单——`should_suppress_meta_channel_post` 兜底，但 prompt 层要先自律。
- 记忆文件只存"事实性总结"，不存 `nsec`/密钥/大段原文。
