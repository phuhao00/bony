---
type: 本机交互运行时
title: Huddle 语音、Voice 模型与终端
description: Desktop Huddle 管理 ephemeral 语音频道和浏览器音频，终端以 Rust PTY 与确认帧协议提供交互式 shell。
tags: [huddle, voice, terminal, desktop]
---
# Huddle 语音、Voice 模型与终端

Huddle 的 `HuddleState` 位于 `AppState` 单一 mutex；不得跨 await 持 outer lock。`start_huddle` 走 Idle→Creating→Connected，创建 private ephemeral stream、写 guidelines、加 bot、发 started event、启动 pipeline/audio relay；失败必须 rollback。`confirm_huddle_active` 在浏览器完成 getUserMedia/AudioWorklet 后推进 Active。leave/end 先取 handles、释放锁、再 teardown。

`buzz-voice` 提供 April Pocket TTS 的 pinned artifact、SHA-256/size/quantization、`PocketTts`、voice style、默认 voice 和 sample rate。Huddle pipeline 管理 TTS/STT 模型下载、hot start/retry；远程语音中断、取消消费与播放生命周期由 `huddle/tts_tests.rs` 覆盖。PTT global shortcut 用 generation 防止旧 delayed release 覆盖新 press。

`tts_startup.rs::await_worker_startup` 只在 worker 发 readiness 成功时交还可用 handle；显式 startup error 或 readiness channel 断开都会 join handle 后返回错误。断开仅说明 worker 未报告 ready，不能臆测底层退出原因；helper 不启动 worker，也不取消一个仍运行但未发消息的 worker。

终端由 `terminal_runtime.rs` 拥有 PTY，最多 20 live sessions。attach 创建/重连 session，输入最多 1 MiB；detach 只卸载 renderer subscription，close 才 kill/reap child。前端必须 ack frame；sequence、full/incremental snapshot、viewport 和 scrollback 防止 backpressure/resize 使用旧画面。测试含 runtime reattach/parser 和 `terminal-wheel.spec.ts`。运行 `cargo test -p buzz-desktop`。