---
type: Agent 协议实现
title: ACP Agent JSON-RPC 服务
description: "`buzz-agent` 是可由 harness 启动的 ACP agent，维护 session history、MCP registry、LLM provider、取消和 steer 状态。"
tags: [acp, agent, json-rpc, mcp]
---
# ACP Agent JSON-RPC 服务

`crates/buzz-agent/src/main.rs` 调用 `buzz_agent::run()`。默认路径启动 Tokio、读取 `Config::from_env()`、创建 `Llm` 与 `App`，然后经 stdin/stdout bounded JSON line transport 运行 JSON-RPC。`auth` 子命令单独执行 Databricks PKCE OAuth。

`App` 以 session id 保存 `Session`：MCP registry、skills、history、cancel watch、busy/active run、steer sender、handoff 和累积 token usage。请求处理包括 `initialize`、`session/new`、`session/prompt`、`session/set_model`、`session/cancel` 与 Goose-compatible `_goose/unstable/session/steer`。prompt 新建 task，cancel/steer 只对 in-flight session 生效。

## Provider 与 MCP

`Provider`、`ModelEntry`、`discover_databricks_models` 属于 public surface。MCP child registry 和 built-in tool 与 session 绑定，不能跨 session 搬运 history 或 cancel token。Windows shell resolver 仅转发 `WINDOWS_SHELL_RESOLUTION_ENV` 中明确列出的变量到清空后的 MCP child。

测试：`databricks_oauth.rs`、`golden_transcripts.rs`、`hints_integration.rs`、`openai_auto_upgrade.rs`、`regressions.rs`。运行 `cargo test -p buzz-agent`。房间级运行策略见[ACP Harness](acp-harness.md)。