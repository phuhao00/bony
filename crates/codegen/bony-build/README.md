# bony-build

**Bony Build** 桌面客户端 crate（eframe/egui + ACP stdio）。

产品说明、截图与完整上手步骤见仓库根目录 [`README.md`](../../../README.md)（[English](../../../README.en.md)），含预编译包、会话级插件、Unity、任务 / worktree、监控与上游同步说明。

## 运行

```powershell
# 仓库根目录
powershell -ExecutionPolicy Bypass -File .\scripts\run-desktop.ps1
# 干净重启（结束旧进程 + release）
powershell -ExecutionPolicy Bypass -File .\scripts\run-bony-build.ps1
# 或
cargo run -p bony-build
```

```text
--cwd <path>           会话工作目录
--grok-bin <path>      grok 可执行文件
--ask-permissions      工具需手动批准
```

## 结构

```text
Bony Build (egui)
    │  ACP JSON-RPC over stdio
    ▼
grok agent stdio  →  MvpAgent / SessionActor
```

本 crate 不嵌入完整 agent 运行时，只作为桌面壳驱动 `grok` 子进程。

## Unity 控制

入口有两处：

1. **侧栏「插件 → Unity 控制」** — 引导安装 CLI / Pipeline、选工程、分组操作  
2. **聊天输入框旁的 `+` → Unity 控制** — 为本会话挂上可关闭的 Unity 芯片；也可直接发送「保存场景」「进入 Play」或 `/unity`

对话控制走本地 Unity CLI，**不经 Agent**，避免 agent 在 worktree 里挂死 `unity pipeline install`。

### 侧栏分组

| 分组 | 能力 |
|------|------|
| 创作 | 一键搭场景骨架；创建玩家/NPC/标记点；**给 NPC 接入 AI**（写入脚本 + 挂载）；可单步重跑类型布局 |
| 快捷操作 | 保存/刷新/重编译/清控制台、撤销重做、Play 控制、聚焦视图、复制/删除选中 |
| 场景 / 对象 | 列构建场景、当前场景、新建/加载首场景、根物体、创建平面/平行光 |
| 资源 / 包 | 保存资源、搜索资源、控制台错误、缺失脚本、列包/装包（eval 框可写过滤器或包名） |
| 工程连接 | 状态、Hub 列表/信息/注册/收藏、打开工程、补齐编辑器、Pipeline 安装/升级、LTS、日志、缓存 |
| 测试 / 构建 / 闭环 | EditMode / PlayMode 测试、Win64 构建（`Builds/Win64/Player.exe`）、观察/修复碰撞体、完整闭环（可改闭环对象名） |

聊天芯片或自然语言可触发：

| 说法 | Action | 保存场景 |
|------|--------|----------|
| `搭小游戏雏形` / `/unity scaffold` | `ScaffoldMiniGame` | `Assets/Scenes/BonyPlayground.unity` |
| `搭 RPG` / `做一个rpg` | `ScaffoldRpg` | `Assets/Scenes/BonyRpgTown.unity` |
| `搭 MMO 大厅` / `搭mmo大厅` | `ScaffoldMmo` | `Assets/Scenes/BonyMmoHub.unity` |
| `搭肉鸽局` / `创建肉鸽关卡` / `roguelike雏形` | `ScaffoldRoguelike` | `Assets/Scenes/BonyRoguelikeRun.unity` |

也可说「换个蓝天」「晚霞天空」。单独摆件：`创建npc`、`创建商人`、`创建任务npc`、`创建出生点`、`创建传送门`、`创建敌人点`；也支持 `创建3个npc`。

### NPC AI（Play 对话）

1. 先创建或搭好带 `NPC_*` 的场景（如 `搭 RPG`）
2. 聊天说 `给npc接入ai` 或点芯片 / 侧栏「给 NPC 接入 AI」
3. 流水线会写入 `Assets/Bony/NpcAi/`（`BonyNpcBrain` + `BonyNpcDialogue`）→ 重编译 → 挂到所有 `NPC_*`
4. 进 Play，走近 NPC 按 **E** 打开对话窗

配置 API Key（任选其一）：

- 环境变量 `XAI_API_KEY`（推荐，编辑器进程能读到）
- 或运行时 `PlayerPrefs` 键 `BonyXaiApiKey`

未配置 Key 时仍可对话，但会走**离线占位回复**。Inspector 里可改每人设 `persona` / `role` / `model`。

**边界：** 这些流水线只搭**可进 Play 的场景雏形**（命名标记点、分区、UI 占位、Spawn），**不是**完整 RPG/MMO/肉鸽玩法——不含任务系统、战斗数值、Mirror/Netcode 联机，也不含完整程序化肉鸽地图算法。NPC AI 是对话层，不是任务/经济系统。

诊断类（`/unity doctor`、`/unity env`、`/unity license`）仅 slash，不占主按钮。

搜索资源默认 `t:Prefab`；安装包默认 `com.unity.ugui`。先在 eval 输入框填写再点对应按钮即可覆盖默认值。

### 引导步骤

1. 安装 Unity CLI（复制安装命令）
2. 重新检测
3. 确认 Unity 项目目录
4. 安装 Pipeline（`unity pipeline install`）
5. 探测编辑器（需编辑器已打开项目）
6. 跑完整闭环

Windows 默认安装路径：`%LOCALAPPDATA%\Unity\bin\unity.exe`。

安装 CLI：

```powershell
$env:UNITY_CLI_CHANNEL='beta'; irm https://public-cdn.cloud.unity3d.com/hub/prod/cli/install.ps1 | iex
```

聊天里发送 `/unity` 可查看全部快捷指令列表。