# Agent 引擎

Agent 引擎位于 [`crates/agent/`](../../crates/agent/)，后端主要通过
[`nomifun-ai-agent`](../../crates/backend/nomifun-ai-agent/) 消费它。本页是
当前 workspace 的实现地图，不再是抽离独立仓库的计划。

## Crate 地图

| Crate | 职责 |
| --- | --- |
| `nomi-types` | Provider 无关的消息、工具类型、压缩类型、文件状态、skill 类型，以及本地/持久协作共用的 Agent task、tool policy 与一次调用原语。 |
| `nomi-protocol` | Host/agent 命令与事件协议，以及工具审批状态。 |
| `nomi-compact` | 上下文压缩与消息窗口整理。 |
| `nomi-config` | 运行时、provider、profile、auth 配置。 |
| `nomi-providers` | Anthropic、OpenAI-compatible、Bedrock、Vertex，以及共享的流式、重试、provider 逻辑。 |
| `nomi-tools` | 内置工具与工具注册表原语。 |
| `nomi-mcp` | MCP client、manager、transports 与工具代理。 |
| `nomi-skills` | Skill 发现、frontmatter、加载与 skill-index 支持。 |
| `nomi-memory` | 记忆存储与检索原语。 |
| `nomi-agent` | 核心 engine loop、session、压缩粘合、confirmations、output sinks、skill tool、requirement tools，以及 crate-private 的 embedded AgentExecution 投影。 |
| `nomi-cli` | 使用同一引擎的独立 `nomi` CLI。 |
| `nomi-computer` | 桌面 computer-use 工具实现。 |
| `nomi-a11y` | computer-use 流程使用的 accessibility helper。 |
| `nomi-browser-engine` | 自托管 browser/CDP 自动化引擎。 |
| `nomi-browser` | Browser-use 工具 facade。 |

`nomi_delegate` 在 `nomi-types` 中只有一套请求和回执契约：
`ParallelDelegationRequest`、`AgentExecutionReceipt` 与 `AgentExecutionStatus`。
平台部署持久化聚合，scheduler 异步继续运行时可以返回活动状态；embedded CLI 部署在
当前 Turn 内执行相同的 Agent 调用，并返回 `completed`、
`completed_with_failures` 或 `failed` 的同步终态投影及强类型结果。部署选择是私有的
host composition，不是用户设置、模型参数、产品 mode 或第二套状态机。fork-mode Skill
继续复用同一个 `AgentInvocationRunner` 一次调用原语。

embedded 多 Agent 工作由宿主维护私有 progress ledger，并通过 `ContextContributor`
只注入有长度上限、JSON 编码的兄弟任务分配与状态快照。该区块明确标记为不可信数据，
不能授予权限；模型看不到额外 task-board 工具。workspace 位置由继承后的最终工具权限
以及 registry 共用的读/写 effect catalog 决定：零个或一个可写兄弟继续直接写入，两个
及以上时只有 writer 使用同一个稳定、自包含源码快照派生的私有 worktree，reader 继续
共享源 workspace；非 Git 降级会写进每个受影响结果。child 不再继承 parent raw-shell
hook，因为它会绕过 read-only 与 synthesis 的权限边界；未来若恢复，必须经过相同的进程
capability 与 effect 判定。

Agent crates 不依赖 `nomifun-*` 后端 crate。常规的后端到 agent 集成通过
`nomifun-ai-agent` 进入；`nomifun-app` 与 `nomifun-gateway` 中 feature-gated
的桥接面会直接依赖 browser/computer-use crate，以便把这些能力暴露为 stdio
或公开工具。

## 唯一的运行时

NomiFun 只有一个会话引擎：来自 `nomi-agent` 的内置 **`nomi`** 引擎，带
provider、内置工具、skills、MCP、memory、browser 与 computer-use 支持，在
进程内运行。不存在第二套适配器栈，不需要与外部 agent 协商协议，也没有可以
把会话移交出去的子 agent CLI。

Factory 行为的源码真相来源：

- `crates/backend/nomifun-ai-agent/src/factory/nomi.rs`

两类容易被误认为“会话引擎”、但并不是的东西：

- **终端 session。** 第三方 agent CLI（Claude Code、Codex、Gemini CLI）作为
  普通子进程运行在 `nomifun-terminal` 的 PTY session 里。后端不解析它们的
  协议、也不持有它们的回合状态，只持有伪终端。见
  [`../guides/terminal.zh.md`](../guides/terminal.zh.md)。
- **公开能力入口。** 外部 agent 与脚本经 companion-token 认证的 `/mcp`、
  `/mcp-agent` 或 `/v1` 调用**进入** NomiFun。那是入站方向：NomiFun 是工具
  提供方，不是引擎宿主。

## MCP 与工具注入

MCP / tool 可用性按 session 组装，不是一张全局扁平列表。

常见来源包括：

- 来自 `nomifun-mcp` 的用户配置 MCP server 行；
- AutoWork 需要时注入的 requirement declaration tools；
- session 绑定知识库时注入的 scoped knowledge search；
- Agent factory 根据实例所有者边界派生的平台 Gateway tools；
- Windows/open helper bridge；
- feature-gated computer-use 与 browser-use stdio bridges；
- 引擎自有 `Skill` tool 路径解析出的 skills；
- Nomi 原生工具注册表。

平台 Gateway 是内部能力传输，不是 Conversation 设置或持久化授权。服务端从
已认证主体派生权限；Agent 在子进程运行时，父进程只向 stdio bridge 签发
带作用域和有效期的 access 声明，以及绑定同一份不可变授权的 renewal proof。
续期由进程内可撤销 lease 支撑，因此长时运行或休眠恢复后的 child 可以刷新 access，
却拿不到签名根、也不能扩大 scope。签名根和 lease registry 始终留在进程内，
不写入 build-extra、Conversation 或数据库；runtime teardown 和主进程重启会撤销它们。
公开主体和非实例所有者默认拒绝，不能获得宿主能力。

记录工具可用性时应引用上面的 factory 文件，不要假设每个 session 都拿到同一组
injected servers —— 这组集合仍会随会话配置、挂载的知识库和派生出的权限而变化。

## Skills

Skills 是 instruction/tool bundle。`nomi` 引擎在引擎内有真实的 `Skill` tool
路径，因此一个 skill 会被直接解析并调用，而不是被压平成 prompt 文本。

相关源码：

- `crates/backend/nomifun-extension/src/skill_service.rs`
- `crates/agent/nomi-agent/src/skill_tool.rs`

## Session Flow

```text
UI request
  -> nomifun-conversation route/service
  -> nomifun-ai-agent AgentService / AgentRuntimeRegistry
  -> nomi runtime factory
  -> Nomi engine turn (in-process)
  -> AgentStreamEvent
  -> nomifun-realtime /ws
  -> renderer stream handlers
```

每个会话回合都在进程内运行。公开 remote capability 调用通过 `nomifun-public`
与平台 Gateway registry 进入，而不是通过 conversation HTTP route。

## Design Notes

旧 specs 会把 agent 层描述为“可机械抽离”并只列 11 个 crates。那些文件属于
历史资料。当前代码仍保持强边界，但 browser/computer bridge 与 public gateway
surfaces 意味着真实规则是“主接缝 + 明确记录的 feature-gated exceptions”。

旧 specs 与带日期的 handoff 还会描述并存的多个运行时家族（ACP、OpenClaw
Gateway、Nanobot、Remote Agent）以及在它们之间做选择的 factory。那些引擎已被
移除，只剩 `nomi`。请把那些文档当作“接缝为何长成这样”的记录，而不是当前分发
路径的描述。
