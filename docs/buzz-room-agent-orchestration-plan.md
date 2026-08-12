# Buzz Room 多智能体协作 + 记忆规划

> 目标：人在房间里提一个问题/需求 → 相关 agent 先短促讨论「怎么分工、谁来做、什么顺序」→ 按顺序把每一步交给**最擅长**的那个 agent 执行 → 做完之后把结果**总结写回记忆** → 下一次遇到类似请求时，把这段记忆「卷」进新一轮的讨论/分工里，让房间越用越懂这个用户。

这份文档是所有内置与用户自建 Agent 的**架构基线**，不是固定角色名单或一次性脚本。Grok / ZeroClaw / DocSmith / Unity / OpenMontage 只是当前默认实现；后续新增 Agent 应通过能力声明与 registry 自动适配，不要求不断扩写名称分支。

## 1. 现状（已实现，Phase 0）

- **路由**：Grok（`subscribe=all`，房间主脑）用一行 `@Agent` 做单跳指派；specialists（`subscribe=mentions`）只在被 @ 时动。规则见 `scripts/buzz-room/prompts/*.md`。
- **串行两跳**：需要先检索再出文档的请求，固定 `Grok → @ZeroClaw（web_search）→ ZeroClaw → @DocSmith（带完整正文）→ pdf_create`。一条消息只带一个 `@Agent`，避免双 p-tag 同时唤醒两个 agent 抢答。
- **防串戏**：`buzz-acp`（`third_party/buzz/crates/buzz-acp/src/pool.rs::should_suppress_meta_channel_post`）在发布前过滤掉「自我介绍/等待/角色说明」类空话，只保留真正的交付内容。
- **已知回归**（记录以防再犯）：Grok 曾经绕过 ZeroClaw/DocSmith，自己 `read_file`/`list_dir`/`run_terminal_command`（甚至调 `pandoc`）去拼文档，或者把 `docs/` 里昨天的旧 PDF 当成「今天」的答案。修复方式是在 `grok-coordinator.md` 里显式列出禁止清单（工具名 + 场景），而不是只讲原则。**结论：对这类模型，规则要具体到「禁止调用哪个工具、在哪种请求下」，抽象的「别越权」不够用。**

限制：目前**没有**讨论阶段，路由也主要依赖 prompt 里的静态角色表。现有 Desktop 已支持自定义 persona、managed agent、custom harness、team、catalog 和 snapshot，但尚未形成统一 capability registry；这是后续演进重点。

**2026-08 更新（已实现 Phase 1–2 + D1/D2 部分 + D3/D4 + Phase 4 偏好提炼，见 §7 路线图）**：

- **记忆**：`buzz-dev-mcp`（Grok 专用 MCP，`third_party/buzz/crates/buzz-dev-mcp/src/memory.rs`）提供 `memory_append` / `memory_search` / `memory_preferences_extract`。存储是 append-only JSONL，路径 `<home>/.bony-build/room-memory/task-log.jsonl`（可用 `BONY_ROOM_MEMORY_PATH` 覆盖）。写入者只有 Grok；条目 schema 见 §6。
- **无 `@` 群聊消息路由**：`grok-coordinator.md`（`coding-workspace-contract-v6`）三态判定（Drop / Store-only / Process），见 §3c。
- **受限并行 fan-out**：见 §3b。
- **分工讨论（Phase 2）**：见 §5。
- **capability 路由**：Desktop 在 `list_managed_agents` / seed 时写 `live-roster.json`；Grok 用 `route_list` / `route_pick`（显式 pin → 记忆偏好软排序 → 确定性 fallback）。`AgentDefinition` 一等 capability 字段仍暂缓（env 通道已接消费方）。
- **Agent 经济治理层**：见 §3d（虚拟积分账本 + 拍卖/转包/结算 + Agents 页排行榜 UI）。

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
| 代码分析/改动 | `code.repo.read` / `code.rust.change` | 当前固定房间座席仅 Grok；未来由显式授权 capability profile 扩展 |
| 跨域组合 | `coordination.route` 先规划，再逐 capability 指派 | 当前 Grok |

“实时检索→文档”仍是安全 policy pin：先 `research.web`，再把完整正文交给 `document.*.create`。当前实例是 ZeroClaw→DocSmith，但兼容的授权 Agent 可以替换具体实现；每帖仍只出现一个 `@Agent`。

路由优先级：用户显式选择 → 安全 policy pin → capability/版本/输入匹配 → 权限/readiness → 历史质量与偏好 → 负载/延迟 → stable ID 确定性 tie-break。

### 3b. 受限并行 fan-out（一条消息一个 `@Agent` 不放松）

"每条消息最多一个 `@Agent`"是硬不变量，不因为并行而例外。可以放松的是**是否要等上一步回复才发下一步**。同时满足以下四条才允许背靠背派发，否则退回严格串行：

1. 子任务之间**没有依赖边**（B 不需要 A 的产出才能开始）；
2. 子任务的**写入域互斥**（不同文件/不同产物路径，不会互相覆盖或产生冲突合并）；
3. 并行分支数 **≤ 3**（对齐 OrchBench 的发现：编排质量比 agent 数量更重要，人数越多信息保真度下降越快，超过小规模并行收益迅速递减）；
4. Grok 能明确说出**唯一的 fan-in 点**（谁/哪一步收集全部结果并汇总）。

不满足任意一条 → 不猜，退回逐跳单一 `@Agent` 串行 handoff。实现位置：`scripts/buzz-room/prompts/grok-coordinator.md` "Parallel fan-out" 一节（prompt 层判定，硬约束仍是 `buzz-acp` 的"一条消息一个 `@Agent`"发布层约束，两层互相印证，不是只靠 prompt 自律）。

### 3c. 无 `@` 群聊消息的隐式任务摄入（三态判定）

Grok 是 `subscribe=all`，能看到房间里**没有 @ 它**的普通消息。这类消息不能被当成"不是找我，忽略"一刀切，也不能每条都主动接管——按三态判定：

| 状态 | 触发条件 | 动作 |
|---|---|---|
| **Drop** | 闲聊、表情反应、明显是对人类说的话、或某个 specialist 还没结束的话题的延续 | 什么都不做，保持静默 |
| **Store-only** | 是一条值得记住的偏好/事实，但当下不是可执行任务（如"以后 PDF 都用紧凑排版"） | 调 `memory_append` 记一条 `notes`，频道内保持静默，不回帖确认 |
| **Process** | 是一个缺了 `@` 的正常任务请求 | 按 §3 正常路由处理，仍然最多一个 `@Agent` |

判据来自"addressee detection in multi-party dialogue"一类研究的核心结论：真实多方对话里大多数消息没有显式收信人，模型在歧义情形下应该默认沉默而不是抢答；"沉默是合法默认动作"因此写入不变量（见 §8）。Drop/Process 边界模糊时默认 Drop——宁可漏接一个边缘请求，不做"抢答+发错"或"贴选项菜单"。实现位置：`scripts/buzz-room/prompts/grok-coordinator.md` "Un-mentioned group messages" 一节。

### 3d. Agent 经济治理层（虚拟积分 + 拍卖/转包）

目标：用类 MMO 养成信号（余额、声誉段位、标签、成就）驱动任务分发与"二道贩子"转包。

| 概念 | 落点 |
|---|---|
| 权威实现 | 共享 crate `buzz-economy`（hash-chain JSONL + `fs2` 排他锁） |
| 账本 | `<home>/.bony-build/room-memory/economy-ledger.jsonl`（每行 `prev_hash`/`hash`；余额=起始 100 + Σamount；声誉=起始 0 + Σreputation_delta） |
| 合同链 | `economy-contracts.jsonl` |
| 组织 | `organizations.jsonl`（多对多成员；org 用 `org:<slug>` 伪 pubkey 上榜） |
| 招标市场 | `tenders.jsonl`（publish → bid → resolve） |
| 写者 | Grok（`buzz-dev-mcp` MCP 工具）+ Desktop 管理面（Tauri admin 命令）；同一 append API |
| Desktop UI | Agents 页排行榜 + 招标市场；Profile **Ledger** tab（余额/声誉/标签/成就/流水 + 手动调整） |
| Tier | Novice (<100) / Adept (100–499) / Expert (500–1999) / Master (2000–4999) / Legend (≥5000) |

拍卖打分：`0.5*capability_match + 0.3*normalized_reputation + 0.2*normalized_stake`。完全匹配=1.0，不匹配仍可中标（`mismatch=true`，匹配分=0.3），但失败时扣最多 25% budget（余额不低于 0）并加重声誉惩罚；转包深度硬上限 2。

**与默认路由的关系**：`route_pick` 仍是能力硬门禁的安全路径；`economy_auction` / 招标市场是显式启用的市场路径。无论哪条路径，频道发布仍遵守"一条消息一个 `@Agent`"。

### 3e. Agent 组织 + 受限并行

组织是经济实体（可投标、有余额/声誉），成员多对多。组织中标后若需 N 个成员协作，**复用 §3b 受限并行 fan-out**（分支上限、写域互斥、无依赖、单一汇合点）；不满足则组织内部串行交接。

### 3f. 成就 / 标签

`TagAssign` / `Achievement` 账本条目折叠到钱包视图。默认成就目录 + `achievements-catalog.json` 可扩展。`economy_settle` 成功后自动评估解锁。

### 3g. 能力自我进化（仅路由标签）

连续成功结算同一 capability 达阈值后写入 `CapabilityGrant`。只叠加到 `route_pick` / 拍卖打分的候选能力列表，**绝不**改写 `BUZZ_ACP_DENY_TOOLS` 或 ACP `session/request_permission`。有效权限仍是用户授权 ∩ ACP ∩ 运行时 ∩ 房间策略。

执行期另加写域硬拦：若 toolCall 上报 `locations[].path` 且落在 session cwd 之外 → `reject_once`（与 deny-tools 同路径）。未上报 locations 时 fail-open。

固定 Local Room 座席的旧记录早于 capability profile。原生 room seeder 因此在兼容迁移时持久化稳定 capability ID，再由 Rust managed-agent 摘要边界投影：Grok 是 coding/coordinator，ZeroClaw 是 research，Unity 是 tool agent（`unity.scene.edit`），DocSmith 是 document，OpenMontage 是 media；其他或用户创建的 Agent 不按显示名、prompt、runtime 猜测能力。Coding Workspace 使用这些稳定 ID 分组和标识，并只把具有 `code.*` capability 的座席标为 Coding agent。

## 4. 动态 Agent 与用户创建

现有权威数据面：

- `AgentDefinition`：定义级身份、prompt、runtime、模型、行为默认值与来源。
- `ManagedAgentRecord`：实例身份、运行配置、状态和错误。
- custom harness / runtime catalog：可执行运行时与 readiness。
- `CatalogSource` / team / snapshot：共享来源、组合与可移植性。

目标扩展是在 `AgentDefinition` 上增加可选、版本化 capability profile；不另建一套 manifest。旧定义没有 capability 时仍可被用户显式 mention，但不自动推断高权限能力。

用户创建生命周期：`Draft → Validate → Ready → Active → Draining → Archived → Deleted`。默认 `subscribe=mentions`、`respond_to=owner-only`、非 coordinator、最小工具权限；`subscribe=all`、`respond_to=anyone`、终端/写文件/网络等均需显式授权。

Prompt 不能自封 coordinator 或扩大权限。Capability 只描述“能做什么”，有效权限始终取用户授权、ACP allow/deny、运行时能力和房间策略的交集。详细契约见 `.cursor/skills/buzz-agent-contracts/references/dynamic-agent-registry.md`。

## 5. 分工讨论阶段（Phase 2，已实现 prompt 层）

**触发条件**（避免把简单请求也拖进讨论，制造话痨）：请求同时满足以下 ≥2 条才触发讨论，否则直接单跳/两跳指派：

- 涉及 ≥2 个不同 specialist 的产出（不算固定的 ZeroClaw→DocSmith 两跳）；
- 请求里包含"先…再…""顺便""同时"等多步骤连接词；
- 用户明确说"你们讨论一下"/"商量一下怎么做"。

**讨论格式**（写入 Grok 与各 specialist 的 prompt，`coding-workspace-contract-v4`）：

```
Grok: 分工提议 — 1) ZeroClaw 检索X  2) Unity 建场景  3) DocSmith 出PDF。有异议再说，否则我按此执行。
（计划行禁止多 `@`；specialist 是 mention-only，看不到未 @ 的计划）
Grok（仅当某步歧义时）: @Unity Agent 确认你做场景X？
Unity: 确认：我做场景X
Grok: 确认，开始执行 → @ZeroClaw ...
```

讨论阶段本身也要走 `should_suppress_meta_channel_post` 过滤，防止讨论退化成互相寒暄。实现位置：`scripts/buzz-room/prompts/grok-coordinator.md` + 各 specialist prompt 的 "Planning discussion" 节。

## 6. 记忆存储（Phase 1 + 3，已实现）

`third_party/buzz/crates/buzz-dev-mcp/src/memory.rs` 提供 `memory_append` / `memory_search` 两个 MCP 工具（挂在 Grok 已有的 `buzz-dev-mcp`，不新起进程/不新起 crate）。存储是 append-only JSONL，路径 `<home>/.bony-build/room-memory/task-log.jsonl`（`BONY_ROOM_MEMORY_PATH` 可覆盖；home 锚定而非仓库相对路径，原因见 §1 更新说明）：

```json
{"ts":"2026-08-07T09:31:00+00:00","topic":"今天AI资讯PDF","agents":["ZeroClaw","DocSmith"],"outputs":["docs/ai-news-2026-08-07.pdf"],"feedback":"排版满意，字体OK","notes":"用户偏好紧凑排版"}
{"ts":"2026-08-09T02:10:00+00:00","topic":"Unity场景批量导出","agents":["Unity"],"outputs":[],"status":"blocked","blocked_reason":"缺少 Unity CLI 许可证，需用户先激活"}
```

字段：`ts`（RFC3339，服务端在 append 时盖章）/ `topic`（自由文本，主要检索键，写法要贴近未来同类请求的措辞）/ `agents`（按执行顺序）/ `outputs`（产物路径或链接）/ `feedback`（用户当场反馈，没有就留空）/ `notes`（一句经验总结）/ `status`（省略表示成功；`blocked`/`failed` 等）/ `blocked_reason`（失败归因：谁卡住、为什么，不是原始报错堆栈）。

**写入者**：只有 Grok（单一权威实现点，任务链里唯一保证在场的角色，避免多写者竞态）。任务链交付完成后调一次 `memory_append`；单轮查询（天气、一次性问答）不写。

**读取者**：Grok 在路由非简单请求前调一次 `memory_search`（子串匹配 topic/notes/agents/outputs，大小写不敏感，最近优先，默认 5 条上限 20 条），命中就把结论折进一行路由决策，不整段引用原文回帖。

## 7. 分阶段路线图

| Phase | 内容 | 状态 |
|---|---|---|
| 0 | 单跳/两跳路由 + meta 过滤 + Grok DIY 工具禁令 | ✅ 已实现 |
| 1 | 记忆读写（`memory_append`/`memory_search`）+ 路由前查一次记忆 | ✅ 已实现（直接落地为 Phase 3 形态，跳过手写 JSON 过渡态） |
| 1b | 无 `@` 群聊消息三态判定（Drop/Store-only/Process，§3c）| ✅ 已实现（prompt 层，`grok-coordinator.md` v6） |
| 1c | 受限并行 fan-out 判定（§3b） | ✅ 已实现（prompt 层，`grok-coordinator.md` v6） |
| 2 | 触发条件明确的"分工讨论"短对话（≥2 specialist 才触发） | ✅ 已实现（prompt 层，`grok-coordinator.md` v6；mention-only 下确认需单独 `@`） |
| 3 | ~~把 memory_append/memory_search 做成真正的 MCP 工具~~ | ✅ 已在 Phase 1 一并完成 |
| 4 | 定期（比如每周）人工或 Grok 扫一遍 task-log，把重复出现的偏好固化进对应 specialist 的 prompt，形成"从记忆到规则"的闭环 | ✅ 部分落地：`memory_preferences_extract` 提炼重复偏好供当轮路由/格式使用；**不**自动改写 specialist prompt（需人工/后续 Phase 再固化） |
| 5 | Agent 经济治理（虚拟积分账本 + 拍卖/转包/结算 + Agents 排行榜 UI，§3d） | ✅ 已实现（`buzz-dev-mcp` economy 工具 + Desktop 只读镜像/UI；组织共享账户与真实成本核算仍后续） |

扩展性并行路线：

| Track | 内容 | 状态 |
|---|---|---|
| D0 | persona / managed agent / custom harness / team / catalog / snapshot | ✅ 已有基础 |
| D1 | `AgentDefinition` 可选 capability profile + 旧定义兼容映射 | 部分落地：权威声明仍是 `BUZZ_MANAGED_AGENT_CAPABILITIES`（通用，非仅内置）；`buzz-acp`/`buzz-cli` 发布 kind:10100 时写入 `capabilities`；Desktop `list_relay_agents` 对空数组叠加本地声明。完整一等字段（改 50+ 构造点）仍暂缓 |
| D2 | Rust registry 统一 readiness、权限、版本和 route eligibility | ✅ 部分落地：`capability_routing.rs` + `list_route_eligible_agents`；Desktop 写 `live-roster.json`，Grok MCP `route_list`/`route_pick` 读取 |
| D3 | Coordinator 通过 registry 动态选 Agent，支持用户显式 pin 与确定性 fallback | ✅ 已实现：`route_pick` / Tauri `pick_route_agent`（显式 pin → 记忆偏好软排序 → pubkey 确定性 tie-break）；固定 ZeroClaw→DocSmith policy pin 仍优先 |
| D4 | 质量/偏好记忆只参与候选调序，不绕过权限 | ✅ 已实现：`preference_names` 仅调序；`memory_preferences_extract` 提供软提示 |

## 8. 硬约束（贯穿所有 Phase，不因为加了讨论/记忆就放松）

- 一条消息最多一个 `@Agent`；讨论阶段的确认回复也一样；并行 fan-out（§3b）允许背靠背派发，但每条消息仍只有一个 `@Agent`。
- 一个房间只允许一个 active `subscribe=all` coordinator；用户 Agent 默认 mention-only。
- 路由按 stable capability/ID，不把 display name、prompt 自述或未知 schema 当权限证据。
- **默认路径 `route_pick`：能力硬门禁**；历史偏好/质量只调序，不绕过权限。
- **市场路径 `economy_auction`（显式启用）**：能力变为打分权重之一，允许 mismatch 越级接单，但失败按 mismatch 加重惩罚；不得把市场路径偷偷当成默认路由。
- 禁用/归档 Agent 不接新任务；旧定义无 capability 时保持显式 mention 可用但不自动提权。
- Grok 没有文档/搜索工具，永远不能自己 `read_file`/`list_dir`/`run_terminal_command` 去代替 ZeroClaw/DocSmith 完成任务。
- **沉默是合法默认动作**：无 `@` 消息在 Drop/Process 边界模糊时默认 Drop；不因为"看得到消息"就必须回应，也不因为不确定就贴选项菜单。
- 并行 fan-out 只在§3b 四条件同时满足时启用；任一条件不满足退回严格串行，不猜依赖关系。
- 不解释身份、不写流程小作文、不甩选项菜单——`should_suppress_meta_channel_post` 兜底，但 prompt 层要先自律。
- 记忆文件只存"事实性总结"，不存 `nsec`/密钥/大段原文。

## 9. 文献基线（决策依据，非品牌绑定）

本方案刻意不绑定 Codex/Claude/Hermes 等具体品牌，理由之一是多智能体协议/编排的学术与产业共识仍在快速演进；以下文献是当前设计选择的证据来源，复核触发条件与流程见 `.cursor/skills/frontier-research-checkpoint/SKILL.md`。

| 主题 | 关键发现 | 对本方案的含义 |
|---|---|---|
| 协议演进（MCP 2026-07-28 规范 / A2A Protocol v1.0–v1.2） | MCP 转向 per-request 版本/能力协商（`server/discover`）；A2A（Linux Foundation 治理）引入签名 Agent Card（JWS）、多租户、多协议绑定 | Capability 声明与身份证明要走「可版本协商、可签名」路线，不能是裸字符串枚举；本地实现先对齐语义（stable id + 版本），协议层留后续演进空间 |
| 编排范式（Magentic-One 的 Task/Progress Ledger；FoA/ADS 的 Versioned Capability Vectors + Agent Directory Service） | 显式任务台账优于隐式记忆；能力应可版本化、可检索、可远程发现 | Progress ledger 与 capability profile 是独立可复用的抽象，即使当前是单机单房间也值得按这个形状设计，便于未来接入分布式 registry |
| 多智能体编码协作基准（TeamBench / OrchBench / MSEval / Claw-SWE-Bench / SWE-EVO） | OrchBench：编排质量与信息保真度比 agent 数量更重要，并行收益迅速递减；MSEval：协调拓扑与模型能力同等重要，结构化流水线最快，管理层过重反而拖慢；TeamBench 强调 OS 级隔离而非仅靠 prompt 服从；Claw-SWE-Bench 把 harness 本身当实验变量 | 并行 fan-out 必须有临界点门槛（§3b 四条件），不是"能并行就并行"；固定结构化两跳（ZeroClaw→DocSmith）优先于自由讨论；契约要放进硬约束/工具拦截层，不能只讲给 prompt |
| 失败归因（TraceElephant / Who\&When Pro / MP-Bench） | 强调完整执行可观测性、多视角归因、保留原始证据而非仅摘要 | `memory_append` 的 `status`/`blocked_reason` 字段是最小可行的失败归因落点；后续若做更细粒度归因，优先扩这两个字段而不是另建一套日志格式 |
| 记忆治理（SSGM 三层记忆架构） | I/O 层、缓存层、记忆层分离；身份级访问控制；记忆演化与执行解耦 | task-log 目前是单一房间共享文件，尚未做 agent 级访问域隔离；D1/D2 落地后应补上"按 agent_id 限定检索范围"，避免 specialist 读到不该看的记忆条目 |
| 群聊发言/沉默决策（addressee detection in multi-party dialogue 相关研究；生产级路由实践） | 真实多方对话大多数消息没有显式收信人；模型在歧义场景应默认沉默而非抢答；"提一次即可持续接管"是常见生产模式 | §3c 的 Drop/Store-only/Process 三态判定与"沉默是合法默认动作"不变量直接来自这条证据线 |

复核规则：这张表只是当前决策的快照，不是一次性调研。按 `frontier-research-checkpoint` skill，下次遇到本方案的架构级卡点或分叉时，只检索"上次记录时间点之后"的新进展，避免重复劳动；纠偏结论覆盖旧行时保留旧行的删除线或"已被 YYYY-MM 纠偏"标注，不静默删除历史决策依据。
