# 文件

- [Tauri Desktop 启动、状态与 IPC](architecture.md) - `buzz-desktop` 将 React hash 路由与 Rust Tauri 本机能力组合，并以 `generate_handler!` 定义可调用 IPC 表。
- [Coding Workspace 与本地项目边界](coding-workspace.md) - Coding Workspace 是频道内覆盖式本地工程界面，安全读取项目/Git 快照并将任务交接给受管 ACP Agent。
- [Huddle 语音、Voice 模型与终端](huddle-terminal.md) - Desktop Huddle 管理 ephemeral 语音频道和浏览器音频，终端以 Rust PTY 与确认帧协议提供交互式 shell。
- [Desktop 身份、备份、归档与同步安全](local-state-security.md) - Desktop 将人类身份、受管 Agent、备份和离线同步保存在本机，并在所有 relay 出站边界禁止密钥材料泄露。
- [Desktop 受管 Agent 与 Local Room Seed](managed-agents.md) - Desktop 以持久化 Agent record 和按 agent/relay 隔离的运行时 pair 管理 ACP 子进程，并幂等 seed Local Room 的 ZeroClaw。
- [Desktop Mesh LLM 与 Mesh Compute](mesh-compute.md) - feature-gated Mesh LLM 在 Desktop 中管理本地模型节点、可信 Nostr 发现、Iroh 传输及 Agent provider 消费。
- [Persona 包、解析与受管 Agent 配置](personas.md) - `buzz-persona` 定义可移植 persona pack 的 manifest、合并、解析和验证，Desktop 将其快照为 Agent 的有效启动配置。
