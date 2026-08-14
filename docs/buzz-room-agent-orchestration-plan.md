# 房间协作基线

人在 Local Room 里提问 → 用一条消息、一个 `@Agent` 交给最合适的座席 → 交付写在频道正文（禁止沙盒附件死链）。

当前默认 seed 只有 **ZeroClaw**（检索）。编码走 **Coding Workspace + ACP catalog**。硬拦在 `buzz-acp`（`BUZZ_ACP_DENY_TOOLS`、meta 过滤），不靠 prompt 自觉。

细节与启动：[`docs/buzz-room-collab.md`](./buzz-room-collab.md)、[`docs/PROJECT_STANDARDS.md`](./PROJECT_STANDARDS.md)。
