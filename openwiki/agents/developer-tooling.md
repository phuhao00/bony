---
type: 开发者工具设计
title: MCP、CLI 与 Sprig 分发
description: "`buzz-dev-mcp` 为 agent 提供开发工具，`buzz-cli` 提供 Buzz 自动化 CLI，`sprig` 以 multicall 组合分发人格。"
tags: [mcp, cli, tooling, agents]
---
# MCP、CLI 与 Sprig 分发

`buzz-dev-mcp` 的 `run()` 经 rmcp stdio 提供 `read_file`、`rg`、`tree`、`shell`、`str_replace`、`todo`、`view_image`、`memory`、`economy` 等工具；`route`/`shim` 负责 multicall 分派。shell 有 Unix process-group 和 Windows Job Object kill 语义，改动时必须保护超时/取消后子树不会遗留。

`buzz-cli` binary `buzz` 的库入口是 `run_from_args()`；它以 NIP-98/NIP-OA、SDK builders 和 WS client 操作 messages、channels、agents、workflow、repos 等公共面。`buzz-sdk` 仅构造 typed event，不持 key/network；`buzz-ws-client` 提供 `NostrWsConnection`、publish、auth event 与 relay parser。

`s‍prig` 根据 `argv[0]` 分发：`buzz-acp`→harness，`buzz-agent`→agent，其余 `buzz-dev-mcp`/`rg`/`tree`/`buzz`/Git helper aliases→developer MCP。新增 alias 要同时验证分发、目标 CLI contract 与包装环境。

运行 `cargo test -p buzz-dev-mcp`、`cargo test -p buzz-cli`；端到端 relay 行为使用[开发与验证](../operations/development-testing.md)。