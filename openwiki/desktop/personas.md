---
type: 配置契约设计
title: Persona 包、解析与受管 Agent 配置
description: "`buzz-persona` 定义可移植 persona pack 的 manifest、合并、解析和验证，Desktop 将其快照为 Agent 的有效启动配置。"
tags: [persona, agents, configuration]
---
# Persona 包、解析与受管 Agent 配置

`buzz-persona` 的公开模块为 `manifest`、`merge`、`pack`、`persona`、`resolve`、`validate`。它是可移植 persona 定义格式，不等于 `managed-agents.json`：pack/manifest 定义输入，merge 处理叠加，resolve 选择有效结果，validate 拒绝不完整/冲突内容。

Desktop 的 `managed_agents/personas.rs`、effective-config、`spawn_snapshot.rs` 将 persona、global config、agent override 和 runtime/harness 解析为启动快照。修改 persona 后必须考虑已创建 record：reconcile/retention/snapshot 决定何时收敛和是否要 idle restart；只更新新建分支会造成旧 record 漂移。

验证 `cargo test -p buzz-persona` 和 Desktop persona/effective-config 测试。运行时私钥/子进程边界见[受管 Agent](managed-agents.md)，本地 retention sync 见[本地状态安全](local-state-security.md)。