# 会话中的 Agent 与模型选择

普通本地 AgentSession 在创建时冻结 AgentPreset Revision、Capability、Skill、Resource
与协作配置，但允许用户在会话页切换当前 Chat 模型。模型切换不创建新会话，也不清空
消息历史、Workspace 或任务状态；下一轮任务继续读取同一会话的历史，并使用新模型。

模型选择仍只有一个运行时事实源。后端从当前 Session 的精确 Revision/Snapshot 出发，
只替换其中的 Chat Route，生成一个内部、不可见的模型变体，并以 binding version 的
compare-and-swap 原子替换 Session binding。新 binding 必须保留原有指令、Capability、
Skill、已解析实现以及 `typed_resource_bindings`；不得从 Agent 当前稳定版本重新解析，
也不得借模型切换升级工具权限。`Conversation.model` 投影这个新 binding 的当前模型。

已有 Turn 进行中时不能切换模型：该 Turn 继续使用接纳时的精确路由，选择器提示用户
等待任务完成。切换成功后，进程内旧 Runtime 会在下一次获取时按模型配置差异安全回收，
再从同一会话历史构建新 Runtime。无效、禁用或不具备 Chat capability 的模型由后端
拒绝，不能静默回退。

Remote Session 的路由由 Remote binding 冻结；AgentExecution Attempt 是只读审计记录；
没有 Chat Route 的专属创作 Session 也不开放此更新。这些边界不能通过模型接口绕过。

## 更换 Agent 或资源

用户在已有会话中选择另一个官方或个人 Agent 时，产品进入 Guid 的新会话流程；原会话
保持不变。Guid 负责重新选择资源、AutoWork 或 Agent 集群配置，并创建新的冻结 Session。
若原会话绑定了 Workspace，入口会把该路径作为新会话的显式用户意图带入，但仍由新
Agent 的 Resource admission 重新校验。

协作控件与多数资源控件在已有会话中保留为只读摘要，并明确提示“新建会话后更改”。
发送 Turn 时只提交消息、附件与明确允许的 Turn 输入，不接受 `preset_id`、Capability
overlay 或 MCP overlay。知识库是例外：它是会话拥有的一组可热替换数据资源，不是
Capability overlay。

知识库遵循同一边界：Agent 工作台决定 `knowledge` Actions 的能力上限，Guid 在创建
会话前通过右上角「知识库」控件选择零到多个 `knowledge_base` 资源及回血策略，后端
校验归属与写权限，并把精确资源、`enabled`、`writeback` 与
`writeback_eagerness` 写入 `AgentBinding.typed_resource_bindings`。已有会话可在回合之间
通过专用 AgentSession Knowledge 命令原子替换这组资源；该命令只能在 Agent 工作台已经
授予的 `knowledge` Actions 范围内收窄权限，不能改变 Preset Revision/Snapshot 或扩大
能力。更新时旧运行时必须先完成退出，下一个回合再按新绑定重建，避免 UI、提示词与工具面
各自持有不同状态。

Resource 只会收窄能力：即使通用 Agent 具备 `knowledge/write`，只读知识库仍可挂载并提供
search/read；只要选择中含只读库，当前会话就不能开启回血。知识库删除在 SQLite 写事务内
检查 `agent_session_resources`，仍被任一存活会话选中（包括暂时关闭但保留选择）时必须先解绑，
从而避免产生悬空资源。

启用挂载后，Host 在每个用户回合入口对精确挂载库执行一次有界检索，并将命中文档作为
带来源路径的数据上下文交给模型；模型仍可调用 `knowledge/search` / `knowledge/read` 做
二次检索。关闭回血时 `knowledge/write` / `knowledge/autogen` 不进入模型工具面；手动型
只响应用户明确要求；自动型在最终回答完成、Turn 尚未结束前调用正式的 turn write-back
执行器，高标准筛选并直接写入可编辑知识库。终端的 workpath 知识挂载继续属于独立产品契约。

创意工坊任务只在已冻结为 Creative Studio Agent 的会话内继续复用该 Preset；从其他
会话选择图像、视频或音乐模式会进入 Guid 创建新的 Creative Studio Session。
