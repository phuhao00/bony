---
type: 桌面架构
title: Tauri Desktop 启动、状态与 IPC
description: "`buzz-desktop` 将 React hash 路由与 Rust Tauri 本机能力组合，并以 `generate_handler!` 定义可调用 IPC 表。"
tags: [desktop, tauri, react, ipc]
---
# Tauri Desktop 启动、状态与 IPC

`desktop/src-tauri/src/main.rs` 在 Linux 先应用 WebKit rendering 环境，再调用 `buzz_lib::run()`。`lib.rs::run()` 注册 single-instance、deep-link、notification、window state、native websocket、dialog/process 与非测试 PTT shortcut，`manage` `AppState`、clipboard、pairing 和 `TerminalSessions`，最后 setup 后进入 event loop。

setup 的顺序是安全契约：boot reset sentinel → migration → identity resolution → persona backfill/config warmup → local media proxy/nest → 标记 managed-agent restore pending。agent 不在 setup 立即恢复，而在前端 `apply_workspace` 选定 relay 后恢复，避免在 fallback workspace 上启动。

前端 `desktop/src/main.tsx` 在渲染前配置 dev/E2E bridge、quota recovery 和 legacy community migration；`App.tsx` 完成 identity/community/onboarding gate；`routes.ts` 定义 hash 路由。频道 `ChannelScreen` 是 stream/forum/DM、Agent、Huddle 和 Coding Workspace overlay 的容器。

`commands/mod.rs` 仅组织 module；真实 public IPC 权威点是 `lib.rs` 的 `invoke_handler(tauri::generate_handler![...])`。新增 API 必须经过 command 定义、module、handler 注册、`shared/api/tauri.ts` wrapper/type 和 React consumer。最小 `cargo check -p buzz-desktop`。身份/同步见[本地状态安全](local-state-security.md)。

`apply_workspace` 先规范化候选 relay、设置 relay override 和可选 workspace nsec，故无效 repos directory 不会阻断 relay/key 应用。保存前 `validate_repos_dir` 是 UI 可复用校验；apply 中 `effective_repos_dir` 负责 runtime 降级。无效或后来失效路径不持久化，返回前端诊断；有效路径先持久化，再尝试更新 `REPOS` symlink，symlink 失败仅记录而不令 command 失败。legacy retention 必须在当前 workspace 生效后迁移，event sync 以当前 relay/owner scope 启动，随后才消费 restore-pending。

退出由 `shutdown.rs::shut_down_app` 的 `shutdown_started`/`shutdown_done` 保证正常 exit 与 Unix signal 路径只做一次清理：释放防休眠、关闭 terminal、停止 local managed-agent pairs，再有界停止可选 mesh runtime。只有 local backend agent 进入本地 child shutdown；同一 agent 的所有 relay runtime pair 都必须处理。Unix 按进程组 SIGTERM、最多等两秒、仍存活则 SIGKILL/reap，并持续 drain PTY reader；cooperative child 在 TERM 退出，忽略 TERM 的 child 在 grace 后被 kill。