---
type: 本地工程工作流
title: Coding Workspace 与本地项目边界
description: Coding Workspace 是频道内覆盖式本地工程界面，安全读取项目/Git 快照并将任务交接给受管 ACP Agent。
tags: [coding-workspace, git, desktop, agents]
---
# Coding Workspace 与本地项目边界

Coding Workspace 不是独立 route：`ChannelScreen` 在 stream channel 上切换 overlay，`CodingWorkspaceScreen` 选择项目和 managed agent。发送 prompt 时会先启动 inactive agent，再以选中 agent mention 和 project context 走频道消息路径；真正执行仍由 harness/runtime 完成。

后端 `commands/coding_workspace.rs` 维护 app-data `coding-workspaces.json`，最多 12 个 recent projects，采用 atomic write。`canonical_project_path` 要求 absolute、无 control char、canonical 后为目录；diff 的 relative path 禁止 absolute、空、control char 与非普通组件。untracked file 会重新 canonicalize 并验证仍在 root 内。

`build_workspace_snapshot` 对 plain folder 排除 `.git`、`target`、`node_modules`，对 Git 调用 ls-files/status/log；上限为 500 files、500 changes、30 commits。file diff 限 2,000 行，untracked 读取限 512 KiB。E2E `coding-workspace.spec.ts` 覆盖 overlay inert 开关；Rust 单测覆盖 plain/Git 描述。运行 `cargo test -p buzz-desktop`。