# Agent 引擎

NomiFun 只安装一个官方 `nomifun.nomi` Runtime factory。Chat、Coding、规划、工具调用、压缩与暂停恢复共享 [nomifun-agent-runtime](../../crates/backend/nomifun-agent-runtime/src/lib.rs) 的执行循环。

会话与内容合同见 [Agent Session 当前架构](agent-session.zh.md)。本页记录当前实现接缝，不保留旧引擎、外部 Wrapper 或退役 CLI 的施工方案。

## 执行链

```text
UI 或领域命令
  → CanonicalSessionOwner 与 AgentSessionStore
  → canonical Turn admission
  → official_runtime 与 EngineSessionHost
  → nomifun-agent-runtime
  → engine_journal
  → canonical events 与 Message projection
  → realtime 与 renderer
```

[official_runtime.rs](../../crates/backend/nomifun-app/src/router/official_runtime.rs) 安装唯一 factory；[unified_runtime_host.rs](../../crates/backend/nomifun-app/src/router/unified_runtime_host.rs) 组合模型、工具、资源、journal 和取消端口；[nomifun-ai-agent](../../crates/backend/nomifun-ai-agent/src/lib.rs) 管理产品运行句柄和流式事件。

## 引擎原语与平台所有权

`crates/agent/` 提供 provider、模型消息、压缩、MCP、Skill、Browser 与 Computer 等共享原语。平台领域通过 Module 和 Action 接口执行资源操作，由各自 owner 解释结果。Runtime 不拥有第二套 Session、效果台账或产品权限。

Agent revision 与 Snapshot 冻结选中的执行闭包。可见工具描述可以按需展开；授权仍由冻结 binding 和 Kernel 校验。Skill、MCP 和 Provider 不因为它们参与模型上下文就合并成新的 Agent 身份。

## 历史与恢复

模型上下文从 canonical 事件和内容构建，不读取 UI 投影补齐缺失 Runtime 日志。闭合回合的只读重建与可执行 checkpoint 恢复具有不同条件，详见 [Session 架构](agent-session.zh.md)。

Terminal 中的第三方 CLI 是 PTY 子进程；Remote 的 `/mcp` 和 `/api/remote/*` 是 canonical Session 入口。它们都不注册另一套产品 Runtime。
