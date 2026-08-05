---
name: buzz-room-build-gate
description: >-
  Strictly gates Buzz Desktop / room stack compile and launch under bony-build.
  One monorepo Cargo workspace (root Cargo.toml), one root target/, Buzz is
  in-tree (not a submodule). Prefer scripts (build-desktop, start-desktop,
  build-tools) or cargo -p from repo root. Force shared target, no dual-target,
  no casual cargo clean. local-stt stays ON by default (sherpa shared DLLs).
  Use whenever building/running Buzz, Desktop, relay, or reclaiming target disk.
---

# Buzz Room 编译闸门（严格）

Agent 在 bony-build 内凡涉及 **Buzz / Desktop / 房间栈编译或启动**，**必须先读并遵守本 skill**。

## 项目形态（必须记住）

- **单一 Cargo workspace**：仓库根 `Cargo.toml`（Grok CLI + Buzz crates + `buzz-desktop`）。
- **单一 target**：仓库根 `target/`。
- Buzz 源码在 **`third_party/buzz/` 普通目录**（不是 git submodule）。
- 允许从根直接：`cargo build -p buzz-desktop`、`cargo build -p buzz-relay`。

## 硬性目标

1. **只编一次真正需要的**；有可用 `target/debug/buzz-desktop.exe` 就不重编。
2. **只有一棵** 仓库根 `target/`；禁止再生 `desktop/src-tauri/target` 或把主缓存拉回 `third_party/buzz/target`。
3. **默认保留 local-stt**（sherpa **shared** DLLs）；仅在用户明确要求时用 `-NoLocalStt`。
4. 路径只在 **本 monorepo**。
5. 磁盘：可用 `clean-target-bloat`；**禁止**无确认 `cargo clean` / `-Nuclear`。

---

## 允许的命令（白名单）

从 **仓库根** 执行。

| 意图 | 入口 | 说明 |
|------|------|------|
| 工具/sidecar | `scripts/buzz-room/build-tools.ps1` | |
| Relay + Grok | `scripts/buzz-room/start-room-stack.ps1 -SkipBuild` | 有二进制时带 `-SkipBuild` |
| **Desktop 编译** | `scripts/buzz-room/build-desktop.ps1` 或 `cargo build -p buzz-desktop` | 默认含 local-stt（shared） |
| **日常启动 Desktop** | `scripts/buzz-room/start-desktop.ps1` | 有 exe 则不准再全量 compile |
| 缩 target | `clean-target-bloat.ps1` | 须警告再 `-Nuclear` |
| 停 harness | `stop-room-stack.ps1` | |

Helpers：`_paths.ps1`、`_desktop-build.ps1`（`CARGO_TARGET_DIR` = 仓库根 `target`）。

Cargo 配置：仓库根 `.cargo/config.toml`（含 thin dev profile、`CMAKE_POLICY_VERSION_MINIMUM`、Windows **不设** `+crt-static`）。

文档：`docs/buzz-room-collab.md`。

---

## 禁止

| 禁止 | 原因 |
|------|------|
| 再建独立 `[workspace]` 在 `third_party/buzz` 或 Desktop 下 | 拆回多项目 |
| 未设共享 root `target` 的裸 `tauri dev` / 嵌套 target | 双 target |
| 无确认 `cargo clean` | 毁掉整仓缓存 |
| 改默认关掉 STT“只为图编译过” | 功能阉割；应继续用 shared sherpa |
| 固定污染性 `RUSTFLAGS` 导致全量失效 | 冷编译 |

---

## 决策树

```
用户要 Desktop / 房间？
  ├─ relay 没起来？ → start-room-stack -SkipBuild
  ├─ 只要开 UI 且已有 target/debug/buzz-desktop.exe？ → start-desktop.ps1
  └─ 缺 exe / 改了 Rust？ → cargo build -p buzz-desktop 或 build-desktop.ps1
```

链接失败若再出现 sherpa 静态库 `__std_find_end_*`：确认 Desktop 用的是 `sherpa-onnx` **shared** feature，不要退回 static。
