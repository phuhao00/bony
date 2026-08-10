# 10. 模块、Cargo Workspace、Feature 与平台编译

> **本章学到什么**：workspace 组织、成员继承（`edition.workspace`/`workspace = true`）、feature 与可选依赖（`dep:` 语法）、`cfg`/`cfg!` 条件编译、build.rs、模块可见性与 re-export facade。
>
> **真实入口**：根 `Cargo.toml`、`crates/codegen/xai-grok-voice/Cargo.toml`、`crates/codegen/xai-system-power/src/lib.rs`、`crates/codegen/xai-grok-shell/build.rs`。

## 1. Workspace：一个锁文件、一个 target

本仓库是单 workspace：根 `Cargo.toml` 列出约 130 个成员（`crates/codegen/*`、`crates/common/*`、`third_party/buzz/*` 等），共享：

- 一个 `Cargo.lock`（依赖版本全局一致）
- 一个根 `target/`（编译缓存共享；这也是项目规范「禁止第二套 target」的技术基础）

根 manifest 的两个共享段（`Cargo.toml:130-132, 488+`）：

```toml
[workspace.package]
edition = "2024"
license = "Apache-2.0"

[workspace.lints.clippy]
doc_lazy_continuation = "allow"
needless_lifetimes = "allow"
too_many_arguments = "allow"
uninlined_format_args = "allow"   # 注释解释了为什么暂时 allow
useless_format = "allow"          # fastrace 过程宏误报，上游修复后可回收
```

成员 crate 用继承写法，避免一百多份拷贝：

```toml
# 任一成员 Cargo.toml
edition.workspace = true

[dependencies]
tokio = { workspace = true, features = ["sync", "time"] }   # 版本由根 [workspace.dependencies] 定

[lints]
workspace = true
```

**注意**：本仓库根 `Cargo.toml` 头部写着 `# Auto-generated workspace root. Prefer editing per-crate Cargo.toml files.`——加成员/改依赖去改各 crate 自己的 manifest，再由工具同步根。

## 2. Feature：编译期的能力开关

`crates/codegen/xai-grok-voice/Cargo.toml`（节选）是教科书级例子：

```toml
[features]
# `audio` = "microphone capture is compiled in".
default = ["audio"]
# CI 沙箱没有麦克风，用无音频默认集
default-bazel = []
audio = ["dep:cpal"]

# cpal 只在非 Linux 用；Linux 走系统录音器子进程，静态 musl 不链 alsa
[target.'cfg(not(target_os = "linux"))'.dependencies.cpal]
version = "0.15"
optional = true

[[bin]]
name = "voice-probe"
path = "src/bin/voice_probe.rs"
required-features = ["audio"]
```

逐点学：

1. **`audio = ["dep:cpal"]`**：`dep:` 前缀语法——激活 feature 时启用可选依赖 `cpal`，且**不**隐式暴露同名 feature `cpal`（旧语法会把依赖名也变成 feature，污染命名空间）。
2. **平台门控依赖**：`[target.'cfg(not(target_os = "linux"))'.dependencies.cpal]`——Linux 构建根本不拉 cpal/alsa，全静态 musl 二进制才能成立。feature 与 target 两个维度组合。
3. **`required-features`**：`voice-probe` 这个 bin 只在 `audio` 开启时编译——关掉 feature 不是代码里塞 `#[cfg]` 空壳，而是目标直接不参与构建。
4. **default feature 不是神圣的**：CI 用 `default-bazel = []` 空集跑无麦克风构建。feature 是**构建配置**，不是代码分支的垃圾场（代码侧仍要用 `cfg` 正确门控，见下）。

代码侧对应（`xai-grok-voice/src/lib.rs` 节选）：

```rust
#[cfg(feature = "audio")]
pub mod audio;          // 模块级门控
// ...
#[cfg(feature = "audio")]
pub use probe::run_mic_only_probe;

/// Whether this build can capture microphone audio (the `audio` feature).
pub const AUDIO_SUPPORTED: bool = cfg!(feature = "audio");
```

- `#[cfg(...)]`：**编译期**移除条目（模块、函数、use）。
- `cfg!(...)`：**运行时**布尔表达式（编译期求值成 `true`/`false`）——适合放进常量或普通逻辑判断，不会造成死代码报错。

## 3. 平台多态：同名模块、不同文件

`crates/codegen/xai-system-power/src/lib.rs:116-153`：

```rust
#[cfg(target_os = "macos")]
#[path = "macos.rs"]
mod imp;

#[cfg(target_os = "windows")]
#[path = "windows.rs"]
mod imp;

#[cfg(target_os = "linux")]
#[path = "linux.rs"]
mod imp;

#[cfg(not(any(target_os = "macos", target_os = "windows", target_os = "linux")))]
mod imp {
    // 内联 no-op 实现：Listener::start() -> None 等
}
```

其余代码只写 `imp::Listener::start(callback)`，平台差异被收敛到模块选择这一处。第四个分支是**兜底 no-op**——冷门平台也能编译，只是没有电源感知。这是「平台多态编译」的标准形态（第 15 章进入 `windows.rs` 的 unsafe 内部）。

## 4. build.rs：编译前的脚本

`crates/codegen/xai-grok-shell/build.rs`（节选）负责在 release 构建时打包 ripgrep：

```rust
fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("cargo:rerun-if-env-changed=GROK_SHELL_BUNDLE_RG_PATH");
    // 声明自定义 cfg，让 cfg(bundle_rg) 被 lint 认识
    println!("cargo:rustc-check-cfg=cfg(bundle_rg)");

    // 只在 release 或显式指定路径时打包，debug cargo check 不碰文件系统
    let path_override = env::var("GROK_SHELL_BUNDLE_RG_PATH").ok();
    let is_release = env::var("PROFILE").as_deref() == Ok("release");
    if path_override.is_none() && !is_release {
        return Ok(());
    }
    // ... 下载/复制 rg，最后 println!("cargo:rustc-cfg=bundle_rg")
}
```

build script 与编译器的通信协议就是 stdout 指令：

| 指令 | 作用 |
|---|---|
| `cargo:rerun-if-env-changed=X` | 环境变量 X 变化才重跑（否则每次都可能重跑） |
| `cargo:rustc-cfg=NAME` | 给编译器注入 `#[cfg(NAME)]` |
| `cargo:rustc-check-cfg=cfg(NAME)` | 把自定义 cfg 登记为合法，避免 lint 报 unexpected_cfgs |

环境变量 `PROFILE`（debug/release）、`OUT_DIR`（脚本产物目录）、`CARGO_MANIFEST_DIR` 由 Cargo 注入。

## 5. 模块可见性与 facade

`crates/common/xai-tool-runtime/src/lib.rs`（节选）：

```rust
#![forbid(unsafe_code)]

pub mod context;
pub mod dispatch;
pub mod error;
// ...

pub use context::{ToolCallContext, SessionContext, /* ... */};
pub use dispatch::ToolDispatch;
pub use error::{ToolError, ToolErrorKind};
pub use tool::{ArcTool, Tool, ToolDyn, ToolStream, ToolStreamItem, terminal_only, /* ... */};

// 甚至可以把别的 crate 的类型也提升到本 facade
pub use xai_tool_protocol::{StreamingSpec, ToolCallId, ToolCapabilities, ToolId, ToolScope};
```

- `pub mod` 开放模块树，`pub use` 把分散在子模块里的类型**提升到 crate 根**：使用者 `use xai_tool_runtime::Tool` 即可，不必知道内部文件布局。重构内部目录时，只要 facade 不变，下游不受影响。
- `#![forbid(unsafe_code)]`：整个 crate 禁止 unsafe——这个契约层 crate 用属性把规范变成了编译器强制。

可见性层次复习：私有（默认）→ `pub(crate)`（本 crate）→ `pub(super)`/`pub(in path)`（局部）→ `pub`。写 API 时**从最小可见性开始**。

## 6. 动手练习

1. **画依赖账**：`cargo tree -p xai-grok-voice -f "{p} {f}"` 看 feature 如何影响依赖图；再跑一次带 `--no-default-features`，对比 cpal 是否消失。
2. **找门控**：`rg "cfg\(feature" crates/codegen/xai-grok-voice/src` 列出所有被 feature 门控的条目，检查「关掉 feature 后没有残留引用」。
3. **workspace 练习**：说出 `cargo build -p buzz-desktop` 与在 `third_party/buzz/desktop/src-tauri` 目录里构建的区别（提示：target 位置、锁文件）。
4. **思考题**：为什么 `[workspace.lints]` 的 `uninlined_format_args` 是 allow 而不是 deny？读那段注释，说出 merge queue 与 lint 策略的关系。

## 自检

- [ ] 能解释 workspace 共享 lock/target 的意义
- [ ] 会写 `dep:` feature 与平台门控依赖
- [ ] 区分 `#[cfg]` 与 `cfg!`
- [ ] 知道 build.rs 的 stdout 指令协议
- [ ] 会用 `pub use` 搭 facade

> 下一章：[11. 智能指针、内部可变性、Drop 与 Pin](11-smart-pointers-and-memory.md)
