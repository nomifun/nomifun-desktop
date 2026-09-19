# 会话中的 Agent 与模型选择

AgentSession 在创建时冻结 AgentPreset Revision、Resolved Snapshot、Chat Route、模型、
Capability、Resource 与协作配置。`Conversation.model` 是这份冻结事实的 UI 投影，
不是会话内可变的第二套路由来源；Snapshot 中的 `resolved_model` 记录同一创建边界。

因此，会话页只读展示当前模型与协作配置。发送 Turn 时只提交消息、附件与明确允许的
Turn 输入，不接受 `preset_id`、Capability overlay 或 MCP overlay。后端不提供
`/preset`、`/capability-selection`、`/mcp-selection` 等原地覆盖路由，避免历史消息、
工具权限与 Runtime 实际绑定互相矛盾。

## 更换 Agent 或模型

用户在已有会话中选择另一个官方或个人 Agent 时，产品会进入 Guid 的新会话流程；
原会话保持不变。Guid 负责重新选择模型、资源、AutoWork 或 Agent 集群配置，并创建
新的冻结 Session。若原会话绑定了 Workspace，入口会把该路径作为新会话的显式用户
意图带入，但仍由新 Agent 的 Resource admission 重新校验。

模型与协作控件在已有会话中保留为只读摘要，并明确提示“新建会话后更改”。执行
Attempt 的只读记录继续由 AgentExecution 控制，不能成为绕过冻结 Session 的聊天入口。

创意工坊任务只在已冻结为 Creative Studio Agent 的会话内继续复用该 Preset；从其他
会话选择图像、视频或音乐模式会进入 Guid 创建新的 Creative Studio Session。
