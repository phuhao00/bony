---
name: lancedb-best-practices
description: >-
  LanceDB embedded vector store best practices for bony-build, grounded in the
  existing buzz-search::vector::VectorSearchService implementation. Covers
  cold/hot table topology (community-wide knowledge tables vs per-agent/session
  memory tables), a Rust producer-consumer batch write architecture
  (tokio::sync::mpsc) to avoid concurrent merge_insert version conflicts,
  Table::optimize maintenance scheduling (compaction/prune/index-optimize)
  triggered by room-idle conditions instead of "midnight windows", and
  nprobes/refine_factor query tuning defaults sized for a handful to dozens of
  local room agents rather than internet-scale traffic. Use when creating,
  reviewing, or extending any LanceDB-backed vector search or agent-memory
  table in this repo, or when touching buzz-search/vector.rs,
  VectorRow/VectorSearchService, IVF_PQ indexing, merge_insert upserts, or
  Buzz Room agent memory persistence.
---

# LanceDB 向量存储最佳实践（bony-build / buzz-search）

本 skill 把一套通用 LanceDB 最佳实践素材，翻译并核实为**本仓库实际会用的 Rust API**。唯一权威参照实现是
`third_party/buzz/crates/buzz-search/src/vector.rs`（`VectorSearchService`）。任何新 LanceDB 用例，优先复用或扩展
这个模块里已有的类型/写法，而不是平行发明一套新接口（见 `.cursor/rules/modularity-dry.mdc`）。

## 硬约束：只用 Rust

- 本仓库 `.cursor/rules/rust-only-stack.mdc` 禁止新增 Python/Node/Go 业务逻辑；LanceDB 官方最佳实践素材里的示例代码
  全是 Python（`queue`/`threading`/`pandas`/`duckdb`），**在本仓库落地时必须是 Rust 等价物**，不能照抄。
- 下文每一条都标注了「已核实」或「未核实/需验证」——已核实的都是对着本机 `~/.cargo/registry` 里实际下载的
  `lancedb-0.31.0` 源码 + docs.rs 逐一核对过方法签名的，不是凭印象编的。

## 版本基线（已核实）

| 项 | 值 | 来源 |
|---|---|---|
| `lancedb` | `0.31.0` | 根 `Cargo.toml:408` |
| `arrow-array` / `arrow-schema` | `58.0.0`（锁定） | 根 `Cargo.toml:409-410` |
| lancedb crate MSRV | `1.91.0` | `lancedb-0.31.0/Cargo.toml` |
| lancedb 内部依赖 `datafusion` | `53.1.0`（透传，不是直接依赖） | `Cargo.lock` |

---

## 一、冷热数据拓扑

原始素材的核心思想（全局知识库离线批量导入+建索引+只读高并发；Agent 独占记忆库小表不建索引）直接适用，落到本仓库两类真实表：

### 1. 全局知识库 = 社区共享检索表（冷/大，对应现有 `buzz_search_vectors`）

`VectorSearchService`（`vector.rs`）当前打开的 `buzz_search_vectors` 表就是这一类：所有社区消息的语义索引，持续增长，
被高频查询。设计要点：

- **表命名**：固定单表 `buzz_search_vectors`（已是现状，不要为每个社区单开一个 `.lance` 目录——`community_id`/
  `channel_id` 列过滤已经做了多租户隔离，见 `vector.rs` 的 `search()` 里 `only_if(filter)`）。
- **要不要建索引**：`vector.rs` 现状是**故意不建索引**（模块注释：flat search 在几十万向量以内都够快）。当某个
  社区/全库的行数逼近这个量级、且查询延迟明显变差时，才调用：

  ```rust
  use lancedb::index::{Index, vector::IvfPqIndexBuilder};

  table
      .create_index(&["embedding"], Index::IvfPq(IvfPqIndexBuilder::default()))
      .execute()
      .await?;
  ```

  `IvfPqIndexBuilder::default()` 的 `num_partitions` 默认是 `None`（LanceDB 自动按行数估算分区数），**本仓库房间
  规模下不需要手动指定 `num_partitions`**——手动调参是数据量到百万级才有意义的优化，个位数到几十个 agent 的房间用
  默认值即可。
- 建完索引后，新写入的行**不会自动进索引**（LanceDB 文档明确写的），会被 flat search 兜底但会变慢——这正是第三节
  「维护管线」`OptimizeAction::Index` 要解决的问题。

### 2. Agent 独占记忆库 = per-agent/per-session 小表（热，本仓库尚无实现，是新增建议）

对应素材里的「Agent 独占记忆库」。本仓库定位是本地房间、个位数到几十个 agent（不是互联网规模），单个 agent/session
的记忆量通常是几百到几万条，因此：

- **不要**给每个 agent 开一个独立的 LanceDB 目录（`connect()` 一次开销不小，且会碎片化磁盘）。复用同一个
  `Connection`（同一个 `.lance` 目录），每个 agent/session 一张表，表名建议 `agent_memory_<agent_id>` 或
  `session_memory_<session_id>`（`agent_id`/`session_id` 用现有 Buzz 类型的 `to_string()`，参照 `vector.rs` 里
  `community_id.to_string()` 的写法，不要引入新的 ID 格式）。
- 表 schema 复用 `table_schema()` 的模式（`Arc<Schema>` 构造函数），但按记忆场景精简字段（不需要 `community_id`/
  `channel_id` 多租户列，因为表本身已经是单 agent 隔离）。
- **不建索引**——素材原话「几百到几万条时 flat search 也是毫秒级」在这个数量级下必然成立，`create_index` 反而是
  纯浪费 CPU（且素材已指出「避免频繁重建索引开销」）。
- session 结束后如果记忆不需要保留，直接 `connection.drop_table(&name)`（`lancedb::Connection` 已有此方法），
  不需要维护管线介入。

---

## 二、多 Agent 异步批量写入架构

### 为什么需要（已核实的技术前提）

LanceDB 的提交协议是基于 manifest 的乐观并发控制：多个写者对同一张表**并发**调用 `add`/`merge_insert` 时，后提交
的一方会因为版本冲突失败而需要重试。房间场景下可能有多个 agent 同时产出要写入语义索引的内容，直接让每个 agent
各自调用 `VectorSearchService::upsert`/`upsert_many` 会互相触发这种冲突重试。素材里「生产者-消费者」模型的意图是
**把写入收敛到唯一一条路径**，这个意图完全适用，只是 Python 的 `queue.Queue` + `threading.Thread` 必须换成 Rust
生态的 `tokio::sync::mpsc` + 一个专属写入 `tokio::task`。

### Rust 骨架（伪代码/骨架，非可编译完整代码——接入时参照 `vector.rs` 现有的 `VectorRow`/`upsert_many` 签名）

```rust
// 骨架：所有 agent 通过 tx 提交待写入的 VectorRow，唯一一个后台 task 攒批后统一调用
// VectorSearchService::upsert_many。真正接入时：
// - WriteRequest 里塞 vector.rs 已有的 VectorRow（不要重新发明一套行类型）
// - 错误处理要接到 VectorSearchError，不要 unwrap
use tokio::sync::mpsc;
use tokio::time::{interval, Duration};

const BATCH_MAX_ROWS: usize = 200;      // 数量触发阈值（房间规模，非互联网规模的量级）
const BATCH_MAX_WAIT: Duration = Duration::from_millis(500); // 超时触发阈值

struct WriteRequest {
    row: buzz_search::vector::VectorRow,
}

fn spawn_batch_writer(
    service: std::sync::Arc<buzz_search::vector::VectorSearchService>,
) -> mpsc::Sender<WriteRequest> {
    let (tx, mut rx) = mpsc::channel::<WriteRequest>(1024);

    tokio::spawn(async move {
        let mut pending = Vec::with_capacity(BATCH_MAX_ROWS);
        let mut ticker = interval(BATCH_MAX_WAIT);

        loop {
            tokio::select! {
                maybe_req = rx.recv() => {
                    match maybe_req {
                        Some(req) => {
                            pending.push(req.row);
                            if pending.len() >= BATCH_MAX_ROWS {
                                flush(&service, &mut pending).await;
                            }
                        }
                        None => { // 所有 Sender 都被 drop，收尾后退出
                            flush(&service, &mut pending).await;
                            break;
                        }
                    }
                }
                _ = ticker.tick() => {
                    if !pending.is_empty() {
                        flush(&service, &mut pending).await;
                    }
                }
            }
        }
    });

    tx
}

async fn flush(
    service: &buzz_search::vector::VectorSearchService,
    pending: &mut Vec<buzz_search::vector::VectorRow>,
) {
    if pending.is_empty() {
        return;
    }
    let batch = std::mem::take(pending);
    if let Err(err) = service.upsert_many(batch).await {
        // 骨架：真正接入时接到本模块/调用方已有的 tracing/日志基础设施，
        // 且要考虑失败批次是否需要重新入队重试。
        tracing::warn!(?err, "lancedb batch upsert failed");
    }
}
```

要点：

- `tokio::select!` 同时监听 channel 收数据和 `interval` 心跳，实现「数量 OR 超时」双触发——这是对素材「攒批（满足
  数量或时间间隔）」要求的直译。
- 多个 agent 各自持有 `mpsc::Sender` 的 clone，往同一个 channel 塞数据；只有 `spawn_batch_writer` 起的这一个
  task 真正调用 `service.upsert_many`，天然避免了并发提交冲突。
- `upsert_many` 内部已经是一次 `merge_insert` 调用（见 `vector.rs`），不需要在骨架里重复实现批处理逻辑，批处理
  的职责边界就是「攒够一批 `VectorRow`，调一次现有的 `upsert_many`」。

---

## 三、维护管线：`Table::optimize`

### Python → Rust 的真实差异（已核实，不是同名方法）

Python LanceDB 把维护拆成三个独立方法：`compact_files()` / `cleanup_old_versions()` / `create_index(force=True)`
增量重建。**Rust 0.31 把压缩+清理旧版本+索引优化统一成一个方法**：

```rust
use lancedb::table::OptimizeAction;

// 全量维护（压缩碎片文件 + 清理旧 manifest 版本 + 优化索引），默认参数：
table.optimize(OptimizeAction::All).await?;
```

`OptimizeAction` 是一个枚举，也可以只做其中一项（已核实字段签名对着 `lancedb-0.31.0/src/table/optimize.rs`）：

| 意图 | 变体 | 对应 Python 方法 |
|---|---|---|
| 压缩小文件 / 清掉删除标记的死数据 | `OptimizeAction::Compact { options: CompactionOptions::default(), remap_options: None }` | `compact_files()` |
| 清理历史 manifest 版本 | `OptimizeAction::Prune { older_than: Some(chrono::Duration::hours(1)), delete_unverified: None, error_if_tagged_old_versions: None }` | `cleanup_old_versions()` |
| 让未索引数据并入现有索引（不是重新训练） | `OptimizeAction::Index(lance_index::optimize::OptimizeOptions::default())` | 近似 `create_index(force=True)` 的「增量」效果 |

**差异需要如实指出**：Python 的 `create_index(force=True)` 语义更接近「强制整表重训索引」；Rust 的
`OptimizeAction::Index(OptimizeOptions { retrain: true, .. })`（`OptimizeOptions::retrain()` 构造器）才是等价的
「重训」，而 `OptimizeOptions::default()`/`OptimizeOptions::merge(n)` 是更轻量的「把新数据并入已有聚类，不挪动
聚类中心」。两者取舍见 `lancedb-0.31.0` 文档注释：合并快但不调整模型，重训慢但能纠正数据分布漂移——本仓库房间
规模下，数据分布漂移不会很剧烈，日常维护用默认合并即可，只有长期运行后检索质量明显下降才考虑 `retrain()`。

单机没必要保留很久的 Time Travel 版本这一条原始判断是对的——`older_than` 可以给一个很短的窗口（几小时到 1 天），
不需要照抄互联网服务常见的 7 天默认值。

### 触发条件：房间空闲检测，不是「午夜维护窗口」

素材原话的「低峰期」在服务器场景下通常是「凌晨」，这在单机桌面 + 房间协作场景下没有意义（没有固定用户时区/负载
曲线）。本仓库更贴切的触发条件是**房间空闲**：

- 判断依据（骨架级建议，本仓库目前没有现成的「房间空闲检测」API，需要按下面思路新增，不要假设已存在）：以「最近一次
  agent 写入/查询发生的时间」做一个 `Arc<AtomicI64>`（存 unix 秒），每次 `upsert_many`/`search` 调用后更新；一个
  低频轮询（如 `tokio::time::interval(Duration::from_secs(60))`）检查「距离上次活动是否已经超过 N 分钟」，超过则
  触发一次 `optimize(OptimizeAction::All)`。
- 频率建议：不要用「每 N 次写入」这种固定计数（房间里写入频率本身波动很大），优先用「空闲 M 分钟」；作为兜底，也可以
  叠加一个「距离上次维护超过 T 小时，即使一直不空闲也强制跑一次」的上限，避免长时间高活跃房间的碎片文件无限累积。
  具体 M/T 数值取决于房间实际写入速率，现状（几个 agent、消息级写入）下每小时甚至每几小时跑一次全量 optimize 的
  开销都可以忽略，不需要复刻 lancedb 官方「改动/删除 100000 行或 20 次写操作再 optimize」这个经验值——那是为更大规模
  数据量设计的阈值，本仓库场景下时间驱动比计数驱动更贴合「agent 房间空闲」这个语义。

---

## 四、查询调参：`nprobes` / `refine_factor`

已核实：两者都是 `VectorQuery`（`table.query().nearest_to(...)?` 返回的类型）上的链式方法，和 `vector.rs`
`search()` 里已经在链的 `.distance_type(...)`、`.limit(...)` 是同一个 builder，可以直接续接：

```rust
let stream = self
    .table
    .query()
    .only_if(filter)
    .nearest_to(query.vector.as_slice())?
    .distance_type(DistanceType::Cosine)
    .nprobes(32)        // 新增
    .refine_factor(1)   // 新增，仅在建了 IVF_PQ 索引且需要重排时才有意义
    .limit(top_k)
    .execute()
    .await?;
```

**没有建索引（当前 `buzz_search_vectors` 现状）时，`nprobes`/`refine_factor` 会被 LanceDB 静默忽略**（已核实：
两个方法的文档都写明「仅在该向量列有 IVF PQ 索引时生效，否则该值被忽略」）——所以先看第一节「要不要建索引」，没建
索引就不需要调这两个参数。

### 本仓库房间规模下的默认值建议（区别于互联网规模素材原文的数字）

| 参数 | 素材原文建议 | 本仓库建议 | 调整依据 |
|---|---|---|---|
| `nprobes` | 普通知识库 20~50 | 建索引后从 **20** 起（LanceDB 0.31 也支持完全不设、走自动调优，见下） | 房间规模下表更小、分区数天然更少，20 通常够用；先别手动设，观察召回再调 |
| `refine_factor` | 精度极敏感场景 10 | 默认**不设**；精度不够时先试 **3~5**，极端敏感再到 10 | `refine_factor` 每次查询都要多一轮拉取未压缩向量，房间场景延迟预算小，没必要一开始就上限 |

调整判断依据（直译自素材、保留原意）：

- **召回不够（找不到该有的相关结果）** → 调大 `nprobes`；副作用是磁盘 I/O 和延迟上升。
- **I/O / 延迟已经顶到上限** → 调小 `nprobes`。
- **对排序精度极度敏感的场景**（比如去重、找最相似的那一条而不是「差不多相关」的一批）→ 加大 `refine_factor`。

另外已核实：LanceDB 0.31 的 `VectorQuery` 还提供 `minimum_nprobes`/`maximum_nprobes`（自适应区间，不设定值时由
LanceDB 按过滤后候选数动态调整）——**调用 `.nprobes(n)` 等价于把 min/max 都锁定成 n，会关掉这个自适应行为**。
房间规模、agent 数量不大时，官方建议是「默认不手动设 `nprobes`，先用自动调优」，只有观察到召回不够或延迟不达标
才手动固定值——本表格给的区间是「手动介入时」的起点，不是「必须设置」的强制值。

---

## 五、原生 Arrow / DataFusion 零拷贝分析（已核实存在，接入前需复核版本）

素材第四条要求「深度分析/复杂 SQL 过滤走 Arrow 零拷贝工具链，不要用低效工具」。Rust 生态里**确认存在**对应能力：
`lancedb` crate 自带一个 `lancedb::table::datafusion::BaseTableAdapter`，把 `Table` 包装成 DataFusion
的 `TableProvider`，注册进 `datafusion::prelude::SessionContext` 后可以对表跑任意 SQL，全程 Arrow `RecordBatch`
零拷贝——这不是凭空猜的，是对着本机 `~/.cargo/registry/.../lancedb-0.31.0/src/table/datafusion.rs` 源码核实过
`BaseTableAdapter::try_new` 签名和 `lancedb-0.31.0` 自带的测试用例（`table/datafusion/insert.rs` 里的用法）确认的：

```rust
use lancedb::table::datafusion::BaseTableAdapter;
use datafusion::prelude::SessionContext;
use std::sync::Arc;

let provider = BaseTableAdapter::try_new(table.base_table().clone()).await?;
let ctx = SessionContext::new();
ctx.register_table("buzz_search_vectors", Arc::new(provider))?;
let df = ctx.sql("SELECT community_id, count(*) FROM buzz_search_vectors GROUP BY community_id").await?;
let batches = df.collect().await?;
```

**如实标注未核实的部分**：`datafusion` 目前只是通过 `lancedb`/`lance` 传递进来的依赖（`Cargo.lock` 里能看到
`datafusion 53.1.0`），根 workspace **没有**把 `datafusion` 列为直接依赖（`Cargo.toml` 里搜不到）。要在
`buzz-search` 之外的 crate 里用上面这段代码，需要先给对应 crate 显式加 `datafusion = { workspace = true }`
风格的依赖（并锁定和 `lancedb 0.31.0` 兼容的 `datafusion` 版本——不做这层版本对齐，直接引入可能和 lancedb 内部
用的 `datafusion` 版本冲突，编译期会报类型不匹配），这一步引入新依赖的具体版本号需要在接入时用
`cargo tree -p lancedb -i datafusion` 之类的命令重新确认一次，不要直接抄本文档里的 `53.1.0` 硬编码到
`Cargo.toml`（lockfile 里的传递版本会随 `lancedb` 升级变化）。

---

## 与原始素材的差异小结（Python → Rust 翻译对照）

| 原始素材（Python 生态） | 本仓库 Rust 落地 | 差异说明 |
|---|---|---|
| `queue.Queue` + `threading.Thread` 生产者-消费者 | `tokio::sync::mpsc::channel` + 专属 `tokio::spawn` task | 同步队列/线程 → 异步 channel/task，语义一致 |
| `compact_files()` / `cleanup_old_versions()` / `create_index(force=True)` 三个独立方法 | 统一成 `Table::optimize(OptimizeAction::{Compact,Prune,Index})` 一个方法 + 枚举变体 | **不是**改名对齐，是 Rust API 真的把三件事合并了 |
| pandas 做表格分析 | `arrow-array`/`arrow-schema`（已是本仓库依赖）直接操作 `RecordBatch` | pandas 的拷贝语义 → Arrow 列式零拷贝 |
| duckdb 对接 Arrow 做复杂 SQL | `lancedb::table::datafusion::BaseTableAdapter` + `datafusion::SessionContext` | 见第五节；已核实存在，但引入需补依赖并核对版本 |
| "午夜维护窗口" | "房间空闲检测"（骨架，需新增） | 单机桌面场景没有服务器式负载曲线，用空闲时长替代固定时钟 |
| 数字均按互联网规模标注（百万级并发、100000 行阈值等） | 全部按"个位数到几十个 agent"的房间规模重新给区间 | 见第二、三、四节各自的表格 |

## 相关文件

- `third_party/buzz/crates/buzz-search/src/vector.rs` —— 唯一权威参照实现，任何新 LanceDB 用例先看这个文件的
  `VectorRow`/`VectorSearchService`/`table_schema` 写法。
- `third_party/buzz/crates/buzz-search/Cargo.toml`、根 `Cargo.toml` —— 版本锁定的位置，升级 `lancedb` 前先看
  `Cargo.lock` 里 `datafusion`/`arrow-*` 的传递版本是否跟着变。
