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

协作控件与资源控件在已有会话中保留为只读摘要，并明确提示“新建会话后更改”。发送
Turn 时只提交消息、附件与明确允许的 Turn 输入，不接受 `preset_id`、Capability overlay
或 MCP overlay。

知识库遵循同一边界：Agent 工作台决定 `knowledge` Actions 的能力上限，Guid 在创建
会话前通过右上角「知识库」控件选择零到多个 `knowledge_base` 资源及回血策略，后端
校验归属与写权限，并把精确资源、`writeback` 与 `writeback_eagerness` 一起冻结进
`AgentBinding.typed_resource_bindings`。关闭回血时运行时按只读策略执行；手动型只响应
用户明确要求，自动型才允许按高标准自主沉淀。已有会话只读展示这组挂载；模型切换
必须原样保留它们。终端的 workpath 知识挂载属于独立产品契约，仍可随工作路径动态更新。

创意工坊任务只在已冻结为 Creative Studio Agent 的会话内继续复用该 Preset；从其他
会话选择图像、视频或音乐模式会进入 Guid 创建新的 Creative Studio Session。
