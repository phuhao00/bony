---
name: buzz-room-build-gate
description: >-
  Strictly gates Bony desktop / room stack compilation and launch.
  One monorepo Cargo workspace (root Cargo.toml), one root target/, Buzz in-tree.
  Compile only with cargo -p from repo root. Start only via start-room-stack /
  start-desktop / stop-room-stack. No build/mint/register scripts. local-stt ON
  (sherpa shared DLLs). Use when building/running Buzz, Desktop, or relay.
---

# Bony 编译闸门（严格）

配合总规范：`docs/PROJECT_STANDARDS.md`、`.cursor/rules/*`。冲突时以 **PROJECT_STANDARDS + rules** 为准。

## 项目形态

- **单一 Cargo workspace**：仓库根 `Cargo.toml`。
- **单一 target**：仓库根 `target/`。
- Buzz 在 **`third_party/buzz/`**（普通目录，非 submodule）。

## 硬性目标

1. 有可用 `target/debug/buzz-desktop.exe` 则不重编。
2. 禁止第二棵 `target` / 嵌套 workspace。
3. 默认保留 local-stt（sherpa **shared**）。
4. **禁止**无确认 `cargo clean`。
5. **禁止**用非启动脚本来编译/注册/seed。

## 白名单命令

从 **仓库根**：

| 意图 | 入口 |
|------|------|
| 编译任意 crate | `cargo build -p <crate>`（如 `buzz-desktop`、`buzz-relay`、工具包） |
| 测试 | `cargo test -p <crate>` |
| 房间栈启动 | `scripts/buzz-room/start-room-stack.ps1 -SkipBuild` |
| Desktop 启动 | `scripts/buzz-room/start-desktop.ps1` |
| 停 harness | `scripts/buzz-room/stop-room-stack.ps1` |

Seed / 注册 agent / 密钥：**Rust（Tauri commands / room_seed / cargo 二进制）**，不跑 mint/register ps1。

## 禁止

| 禁止 | 原因 |
|------|------|
| `build-desktop.ps1` / `build-tools.ps1` 等当入口 | 规范：非启动脚本停用 |
| 双 target / 裸 `tauri dev` 另起缓存 | 缓存分裂 |
| 无确认 `cargo clean` | 毁缓存 |
| 默认关 STT「图快」 | 阉功能 |
| 新增自动化脚本实现业务 | 违反 scripts-start-only |

## 决策树

```
用户要 Desktop / 房间？
  ├─ relay 未起？ → start-room-stack.ps1 -SkipBuild
  ├─ 只要 UI 且已有 exe？ → start-desktop.ps1
  └─ 缺 exe / 改了 Rust？ → cargo build -p buzz-desktop，再 start-desktop
```
