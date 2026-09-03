# Browser Platform 历史审计记录

状态：`HISTORICAL AUDIT / NOT A CURRENT ENTRY`

历史记录日期：2026-07-27
归档修订日期：2026-09-03

本文只保留 Browser Platform 在产品方向调整期间形成的有效结论和审计摘要。
它不是当前开发指南、实施台账、发布门禁或验证结果来源。

## 保留的产品结论

- Browser 是 Agent-only 的受管 Chromium 能力。普通 Agent Browser Use 使用
  `--headless=new` 的 Primary，不创建窗口、不抢焦点。
- `/browser` 只负责 Lane/Host 的状态、容量、身份和生命周期管理，不承载页面
  内容、页面输入或用户接管。
- 安装 owner 可以显式请求 running Primary 前台打开；系统会替换 Host，递增
  browser epoch，并使旧 target/frame/ref 失效。调用方随后必须重新观察。
- Primary 使用 NomiFun 管理的应用隔离 profile；Crawl、Anonymous、Authenticated
  Replica 和 Isolated Host 按各自策略保持隔离。
- `BrowserSessionHub` 负责 Host、Lane、资源、身份、owner lease、清理和关闭；
  同一 Lane 串行，不同 Lane 进行有界并发。
- 高风险或不可逆 Browser action 继续遵守现有审批和 fail-closed 边界。
- 早期嵌入式 Viewer、screencast、专用 Viewer 通道和用户接管能力已退出产品，
  不应从历史材料中恢复。

## 历史审计摘要

当时的审计覆盖了 Browser Hub/Lane 生命周期、资源调度、身份隔离、状态管理
页面、实时库存、进程清理和目标平台 smoke。涉及已退出产品方向的 Viewer
测试记录只用于追溯，不能作为当前能力或发布通过证据。

当前实现事实和 Browser 架构请以
[`../architecture/browser-platform.zh.md`](../architecture/browser-platform.zh.md)
及其对应的源码和行为测试为准。

## 当前实施入口

Agent Capability Platform v2 的当前文档入口是
[`../specs/2026-08-28-agent-capability-platform-v2/README.zh.md`](../specs/2026-08-28-agent-capability-platform-v2/README.zh.md)，
当前任务状态以
[`../specs/2026-08-28-agent-capability-platform-v2/GLOBAL-CLOSURE-TODO.zh.md`](../specs/2026-08-28-agent-capability-platform-v2/GLOBAL-CLOSURE-TODO.zh.md)
为准。
