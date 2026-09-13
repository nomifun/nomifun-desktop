# 会话中的模型选择

普通聊天和基于 AgentPreset 创建的聊天都支持在会话页切换模型。
这项产品要求取代早期“Preset 会话隐藏或锁定模型选择器”的约定。

`Conversation.model` 是当前运行模型的唯一来源。Agent Snapshot 中的
`resolved_model` 记录创建时的初始模型；切换会话模型不会修改预设、指令、
能力白名单、资源权限或历史消息。

选择器通过现有会话 PATCH 一次性更新主模型与协作模型池。服务端等待旧运行时
退出后保存设置，下一条消息使用新模型并继续读取该会话的历史。
Guid 与会话页的桌面模型选择器使用同一条更新路径。

Agent 的资源控件仍依据 Snapshot 声明显示。不能把“具有 preset_id”重新解释为
“禁止用户切换模型”。执行任务的只读记录与创意工坊专属会话仍由各自边界管理。

## 会话内切换 Agent

普通 AgentSession 还允许在同一会话中切换官方或个人 Agent。服务端使用当前模型
解析目标 Agent 的稳定绑定，等待旧运行时退出后，原子替换会话的 Preset lineage、
Snapshot、工具白名单和内部 Session binding。会话 ID、标题、消息历史、Workspace、
当前模型和协作设置保持不变；下一条消息从完整历史中恢复并按新 Agent 执行。

官方模板在切换前只生成可复用的内部运行配置，不进入“我的 Agent”。Remote、
AgentExecution 只读记录及非 Nomi 会话不开放这项更新。
