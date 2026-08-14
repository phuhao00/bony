# Bony 架构

Bony 是本地优先的桌面 AI 编程 + 多 Agent 房间。产品在仓库根：`crates/`、`desktop/`。单一 Cargo workspace、单一根 `target/`。

Coding Workspace 通过 PATH 上的 ACP 进程接入编码 Agent。

## 平面

| 平面 | 位置 | 职责 |
|------|------|------|
| 桌面壳 | `desktop/`（crate `buzz-desktop`） | Tauri 2 + React；频道、Coding Workspace、seed、本机能力 |
| 房间栈 | `crates/buzz-relay` 等 | WebSocket / Nostr 事件、SQLite、进程内 pubsub、搜索 |
| ACP 桥 | `crates/buzz-acp` | 房间事件 → ACP 会话池；启停 managed agent |
| 默认房间座席 | `desktop/src-tauri` `room_seed` | ZeroClaw（`research.web`）；规格外的旧座席 reconcile 时剥离 |

## 构建与启动

```powershell
cargo build -p buzz-desktop
cargo build -p buzz-relay
powershell -File .\scripts\buzz-room\start-room-stack.ps1 -SkipBuild
powershell -File .\scripts\buzz-room\start-desktop.ps1
```

只允许白名单启动脚本；编译一律 `cargo -p`。

## 协作硬拦

`buzz-acp` 对 `session/request_permission` 命中 `BUZZ_ACP_DENY_TOOLS` 时 `reject_once`。一条房间消息最多一个 `@Agent`。检索默认 `@ZeroClaw`。编码走 Coding Workspace + ACP catalog，不把「今天资讯」做成本地拼文件旁路。
