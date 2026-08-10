# 15. Unsafe Rust、FFI、ABI 与安全封装

> **本章学到什么**：`unsafe` 的五种能力与责任、raw pointer 所有权转移（`into_raw`/`from_raw`）、`extern "system"` ABI、`#[repr(C)]`、`unsafe impl Send/Sync`、SAFETY 注释纪律、把 unsafe 封进安全 API。
>
> **真实入口**：`crates/codegen/xai-system-power/src/windows.rs`、`crates/codegen/xai-tty-utils/src/process_resources.rs`。
>
> 项目立场：unsafe 极少且集中（`xai-tool-runtime` 甚至 `#![forbid(unsafe_code)]`）。用到的地方每一处都有 SAFETY 论证。

## 1. `unsafe` 到底解锁了什么

只有五件事：解引用裸指针、调用 unsafe 函数（含 FFI）、访问可变静态、实现 unsafe trait、union 字段。它**不**解除借用检查之外的任何正确性义务——越界、悬垂、数据竞争照样是 UB（未定义行为）。口诀：**unsafe 是「编译器无法验证，由我担保」的签名**。

## 2. 完整案例：Windows 电源事件回调

需求：监听系统休眠/唤醒（agent 要暂停心跳、重连模型）。Windows 给你一个注册函数和一个约定：**系统会在任意线程用 `extern "system"` ABI 回调你，并原样传回你注册时给的指针**。

### 2.1 注册：把所有权交给 OS

`windows.rs:44-68`：

```rust
impl Listener {
    pub(crate) fn start(callback: PowerCallback) -> Option<Self> {
        let ctx = Box::into_raw(Box::new(Context { callback }));

        let mut params = DEVICE_NOTIFY_SUBSCRIBE_PARAMETERS {
            Callback: Some(power_callback),
            Context: ctx as *mut c_void,
        };

        let mut handle: *mut c_void = std::ptr::null_mut();
        let status = unsafe {
            PowerRegisterSuspendResumeNotification(
                DEVICE_NOTIFY_CALLBACK,
                &mut params as *mut DEVICE_NOTIFY_SUBSCRIBE_PARAMETERS as HANDLE,
                &mut handle,
            )
        };

        if status != ERROR_SUCCESS || handle.is_null() {
            unsafe { drop(Box::from_raw(ctx)) };   // 注册失败：立刻取回所有权
            return None;
        }

        Some(Self { handle, ctx })
    }
}
```

`Box::into_raw`：消费 `Box`，返回裸指针，**关闭自动释放**——此刻起这块内存归 OS 的指针「保管」。这是 Rust 与外部世界交接所有权的标准动作。

### 2.2 回调：OS 手里拿回借用

`windows.rs:80-97`：

```rust
unsafe extern "system" fn power_callback(
    context: *const c_void,
    event_type: u32,
    _setting: *const c_void,
) -> u32 {
    // Safe: `context` is the live `Context` we registered with.
    let ctx = unsafe { &*(context as *const Context) };
    match event_type {
        PBT_APMSUSPEND => (ctx.callback)(PowerEvent::WillSleep),
        // A single resume can deliver both PBT_APMRESUMEAUTOMATIC and
        // PBT_APMRESUMESUSPEND, so `DidWake` may fire twice per wake. That is
        // fine and intentional: lowering the sleep gate is idempotent...
        PBT_APMRESUMEAUTOMATIC | PBT_APMRESUMESUSPEND => (ctx.callback)(PowerEvent::DidWake),
        _ => {}
    }
    ERROR_SUCCESS
}
```

- `extern "system"` = Windows 回调约定（x86 上是 stdcall 家族）。**签名必须与 OS 期望逐字节一致**，否则栈都退不平衡。
- 回调里只做**借用**（`&*`），不取回所有权——因为 OS 之后还会用它。
- 注释说明「唤醒事件会来两次，但我们的处理是幂等的，不要『修复』它」——跨语言/跨系统边界的真实怪癖，写下来就是知识。

### 2.3 注销：对称地取回所有权

`windows.rs:71-78`：

```rust
impl Drop for Listener {
    fn drop(&mut self) {
        unsafe {
            PowerUnregisterSuspendResumeNotification(self.handle as HPOWERNOTIFY);
            drop(Box::from_raw(self.ctx));
        }
    }
}
```

顺序严格：**先注销**（OS 承诺不再回调）**再释放**。反过来就是 use-after-free。`Box::from_raw` 重新激活 Drop——`into_raw`/`from_raw` 必须严格配对，且一个 Box 只能 `from_raw` 一次。

### 2.4 手动 Send/Sync + 不变量注释

`windows.rs:31-42`：

```rust
pub(crate) struct Listener {
    // Registration handle from `PowerRegisterSuspendResumeNotification`
    handle: *mut c_void,
    // Kept alive (and freed in `Drop`) because the OS holds a raw pointer to it.
    ctx: *mut Context,
}

// The OS invokes the callback on an arbitrary thread; the handle is only used
// to unregister. `PowerCallback` is `Send + Sync`.
unsafe impl Send for Listener {}
unsafe impl Sync for Listener {}
```

裸指针默认不是 `Send`/`Sync`，所以含裸指针的类型不能跨线程——但本例可以，理由写在注释里：handle 只用于注销、回调闭包本身 `Send + Sync`。**`unsafe impl` 必须携带这段论证**，这是项目的隐性规范（也是 review 要点）。

## 3. `#[repr(C)]`：与 OS 共享内存布局

Rust 默认布局（`repr(Rust)`）允许编译器重排字段；跨 ABI 必须锁定为 C 布局。`crates/codegen/xai-tty-utils/src/process_resources.rs:29-58`（节选）读取 macOS 内核的进程内存信息：

```rust
// Hand-rolled `task_vm_info` prefix through `phys_footprint` (the kernel
// accepts any count ≤ the current struct revision; passing the prefix
// count returns exactly these fields). Layout per XNU osfmk/mach/task_info.h.
#[repr(C)]
#[derive(Default)]
struct TaskVmInfoPrefix {
    virtual_size: u64,
    region_count: i32,
    page_size: i32,
    resident_size: u64,
    // ... 字段顺序与 XNU 头文件逐一对应 ...
    phys_footprint: u64,
}
```

配套的 SAFETY 论证（同文件 74-87）：

```rust
pub(super) fn sample() -> ProcessResources {
    let mut info = TaskVmInfoPrefix::default();
    let mut count = PREFIX_COUNT;
    // SAFETY: `info` is a properly sized/aligned out-buffer and `count`
    // tells the kernel its length in natural_t units; TASK_VM_INFO on
    // the caller's own task port cannot fault.
    let kr = unsafe {
        task_info(
            mach_task_self_,
            TASK_VM_INFO,
            &mut info as *mut _ as *mut u8,
            &mut count,
        )
    };
    // ...
}
```

**每处 unsafe 块前都有一条 `// SAFETY:` 注释**，列出前置条件为什么成立。这条纪律值得在你自己的项目里复制。

再看一个短例子——fork 后 exec 前的钩子（`xai-tty-utils/src/lib.rs:104-110`）：

```rust
// SAFETY: detach_from_tty only calls setsid/setpgid, both POSIX
// async-signal-safe. Satisfies the pre_exec contract.
unsafe {
    cmd.pre_exec(detach_from_tty);
}
```

`pre_exec` 运行在 fork 之后、exec 之前，只能调**异步信号安全**函数——SAFETY 注释点明了这一点。unsafe 的难点从来不是语法，而是这些**外部契约知识**。

## 4. 安全封装的边界

注意这三个 crate 的形状：`unsafe` 集中在 `windows.rs` / `macos.rs` / `imp` 模块内部；对外暴露的是 `Listener::start(callback) -> Option<Self>`、`sample() -> ProcessResources` 这样的**安全 API**。调用方（第 10 章的平台多态 `imp` 模块）完全不知道 unsafe 的存在。

**封装原则**：unsafe 的「爆炸半径」必须是一个可以人工审计的小模块；模块边界外，编译器重新接管一切。

## 5. 动手练习

1. **审计练习**：通读 `xai-system-power/src/windows.rs` 全文（很短），列出所有 unsafe 块，逐条写下它依赖的不变量。
2. **找 SAFETY 注释**：`rg "SAFETY:" crates/ -l`，挑两处读论证；再 `rg "unsafe \{" crates/codegen/xai-tool-runtime`（应该没有——`forbid`）。
3. **思考题**：如果 `Listener` 忘记实现 `Drop`，会发生什么？（两块泄漏：句柄 + Context。）如果 `Drop` 里先 `from_raw` 后注销呢？
4. **思考题**：为什么回调里用 `&*`（临时借用）而不是 `Box::from_raw`？什么情况下才能取回所有权？

## 自检

- [ ] 能背出 unsafe 的五种能力
- [ ] 理解 `into_raw`/`from_raw` 的所有权交接与配对要求
- [ ] 知道 `#[repr(C)]` 为什么是 FFI 必需
- [ ] 会写 SAFETY 注释（前置条件 + 为什么成立）
- [ ] 理解「unsafe 封进安全模块」的封装原则

> 下一章：[16. 测试、质量闸门、Benchmark 与性能分析](16-testing-quality-performance.md)
