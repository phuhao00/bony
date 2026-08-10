# 09. Trait、泛型、高阶约束与高级类型

> **本章学到什么**：trait bound 与关联类型的取舍、静态/动态分发、`impl Trait`（参数位与返回位）、HRTB、`Send`/`Sync` 自动 trait、newtype 模式、`From`/`TryFrom`/`AsRef` 转换体系。
>
> **真实入口**：`crates/common/xai-tool-runtime/src/tool.rs`、`crates/codegen/xai-grok-workspace-types/src/identity.rs`。第 03 章从「流契约」看过 `Tool` trait，本章从**类型系统设计**角度重看它。

## 1. 泛型参数 vs 关联类型

```rust
pub trait Tool: Send + Sync {
    type Args: for<'de> Deserialize<'de> + JsonSchema + Send + 'static;
    type Output: Serialize + ToolOutput + Send + 'static;
    // ...
}
```

为什么是 `type Args`（关联类型）而不是 `trait Tool<Args, Output>`（泛型参数）？

| 维度 | 关联类型 | 泛型参数 |
|---|---|---|
| 一个类型实现几次 | 每种 Tool **一次** | 可对不同参数实现多次 |
| 使用方写法 | `impl Tool for MyTool` | `impl Tool<MyArgs, MyOut> for MyTool` |
| 作为类型提及 | `T::Args` 唯一确定 | `Tool<A, B>` 必须带全参数 |

工具场景：一个具体工具（如 `read_file`）只有一组参数/输出类型，关联类型让「`T::Args`」成为**唯一确定的投影**，调用方不用携带额外类型参数。口诀：**「每个实现只有一种选择」用关联类型，「同类型可多次实现」用泛型参数**（如 `From<T>` 可以对无数 T 实现）。

## 2. HRTB：`for<'de> Deserialize<'de>`

`Args: for<'de> Deserialize<'de>` 读作「**对任意**生命周期 `'de` 都实现 `Deserialize<'de>`」。

为什么需要：serde 反序列化可以**借用**输入缓冲（零拷贝），此时借用带某个具体生命周期。如果 bound 写成 `Deserialize<'a>`（固定 `'a`），就只接受特定借用长度。`for<'de>` 表达「无论输入活多久都能反序列化」，涵盖拥有式（`'static`）和借用式两种情况。

仓库里也有函数级用法，`crates/codegen/xai-grok-shell/src/extensions/search.rs:39`：

```rust
fn parse<T: for<'de> Deserialize<'de>>(s: &str) -> Result<T, acp::Error> {
```

HRTB 几乎只和 serde/借用型 trait 一起出现，看到 `for<'...>` 想到「对任意短生命周期成立」即可。

## 3. `impl Trait`：返回位置的类型隐藏

`Tool::execute` 的签名：

```rust
fn execute(
    &self,
    ctx: ToolCallContext,
    args: Self::Args,
) -> impl Future<Output = ToolStream<Self::Output>> + Send {
    async move {
        let result = self.run(ctx, args).await;
        terminal_only(result)
    }
}
```

返回位 `impl Trait`（RPIT；在 trait 方法里叫 RPITIT）：调用方只知道「这是一个 `Future<Output=...> + Send`」，具体是哪个不透明 future 被隐藏。收益：

- 实现者可以自由更换内部 future 组合方式而不破坏 API。
- 不需要 `Box<dyn Future>` 的堆分配。

非 async 的常见形态——返回迭代器，`crates/codegen/xai-codebase-graph/src/scope_graph/graph.rs:844`：

```rust
pub fn file_paths_with_meta(&self) -> impl Iterator<Item = (&str, &FileMeta)> {
```

参数位 `impl Trait` 是泛型语法糖：`fn walk(value: &mut Value, f: &mut impl FnMut(&mut String))` ≈ `fn walk<F: FnMut(&mut String)>(value: &mut Value, f: &mut F)`。

## 4. 静态分发 vs 动态分发（复习 + 决策）

第 03 章看过完整案例：`Tool`（静态、关联类型、RPITIT → 不能 dyn）与 `ToolDyn`（object-safe、JSON 进出 → 装进 `Arc<dyn ToolDyn>` 异构集合）。

决策表：

| 场景 | 选择 |
|---|---|
| 热路径、类型已知、需要内联 | 泛型静态分发 |
| 异构集合、插件式注册、跨 ABI 边界 | `dyn Trait` 动态分发 |
| trait 有泛型方法/关联返回类型又需要 dyn | 再造一个 object-safe 镜像 trait + blanket impl |

## 5. `Send` / `Sync`：编译器推导的并发安全标记

- `Send`：类型的所有权可以**移动到**另一个线程。
- `Sync`：`&T` 可以**同时出现在**多个线程（等价于 `&T: Send`）。

绝大多数类型自动满足；裸指针不满足（第 15 章手动 `unsafe impl`）。`Tool: Send + Sync`、`ToolStream` 里的 `+ Send`、`spawn_blocking(move || ...)` 要求闭包 `Send`——都是同一套标记在把关。写异步代码遇到 `future cannot be sent between threads` 报错时，先找哪个字段不是 `Send`（常见：`Rc`、裸指针、`RefCell` 守卫跨了 `.await`）。

## 6. newtype：用类型系统防止张冠李戴

`crates/codegen/xai-grok-workspace-types/src/identity.rs:17-50`（节选）：

```rust
/// Unique session identifier. Used to scope per-session operations.
#[derive(Debug, Clone, Default, Hash, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct SessionId(pub(crate) String);

impl SessionId {
    pub fn new(id: impl Into<String>) -> Self {
        Self(id.into())
    }
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl From<String> for SessionId {
    fn from(value: String) -> Self { Self(value) }
}
impl From<&str> for SessionId {
    fn from(value: &str) -> Self { Self(value.to_owned()) }
}
```

逐点学：

1. **newtype 模式**：`SessionId(String)` 防止把 session id 误传给期望 tool id / request id 的函数——三个都是 `String`，编译器原本无法区分；包一层就都能区分了，运行时零开销。
2. **`pub(crate) String`**：内部字段只对本 crate 可见，外部只能通过 `new`/`as_str` 操作——**不变量由构造入口守护**。
3. **`#[serde(transparent)]`**：序列化时和裸 `String` 完全一样，线上协议无感知。
4. **`From` 家族**：`From<String>`、`From<&str>` 让构造点写 `.into()`；`impl Into<String>` 入参与之配合。
5. derive 里的 `Hash, Eq, Ord`：因为它要当 HashMap 的键、要排序。

转换 trait 习惯：

| trait | 用途 |
|---|---|
| `From`/`Into` | 不会失败的转换（成对实现，写 `From` 送 `Into`） |
| `TryFrom`/`TryInto` | 可能失败（第 01 章 `FromStr` 是同类思想的字符串专用版） |
| `AsRef`/`AsMut` | 廉价借用视图（`fn f(path: impl AsRef<Path>)` 同时收 `String`/`&str`/`PathBuf`） |

## 7. 其它高级类型（知道即可，用到再深挖）

- **DST（动态大小类型）**：`str`、`[T]`、`dyn Trait` 大小编译期未知，必须躲在 `&`/`Box`/`Arc` 后面。
- **GAT（泛型关联类型）**：关联类型自己带泛型参数（如 `trait Iterable { type Iter<'a>: Iterator where Self: 'a; }`）。本仓库的异步流处理用 RPITIT 回避了大部分 GAT 需求。
- **const generics**：`[u8; 32]` 这类以常量为参数的类型（第 12 章去重表用 `[u8; 32]` 当事件 ID）。

## 8. 动手练习

1. **关联类型实验**：给第 03 章练习里的小 Tool 写一个泛型函数 `fn describe<T: Tool>(t: &T) -> String`，用 `T::Args` 做点事。观察编译器如何单态化。
2. **newtype 练习**：为 `ToolCallId` 设计 newtype（如果已有，读一遍），列出它应该 derive 哪些 trait、为什么。
3. **找 dyn 边界**：`rg "Arc<dyn" crates/ --count`，挑三处说出为什么那里必须动态分发。
4. **思考题**：`SessionId` 为什么把字段设为 `pub(crate)` 而不是 `pub`？如果完全公开，哪些不变量会失守？

## 自检

- [ ] 能说出关联类型与泛型参数的选择判据
- [ ] 能解释 `for<'de>` 的含义
- [ ] 理解返回位 `impl Trait` 隐藏了什么
- [ ] 会用 newtype 防止 ID 混用
- [ ] 知道 `Send`/`Sync` 由编译器自动推导、何时需要手动关注

> 下一章：[10. 模块、Cargo Workspace、Feature 与平台编译](10-modules-cargo-and-platform.md)
