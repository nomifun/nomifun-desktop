# Agent 工作台

> 用户可见的产品名称是 **Agent 工作台**，
> 公共 UI 路由是 `/agent`；`AgentPreset` 只是后端 authoring aggregate 的内部名称。
> 应用 MCP 工具通过现役会话选择接入；程序启停归应用托管 owner。

从侧边栏进入 **Agent**（`/agent`），可直接编辑官方 Agent，或创建自己的 Agent。普通官方列表包含日常对话、通用、编程、伙伴、客服和创作六个入口。第七个官方模板 `app.developer`（内置应用）用于应用工作区，不作为普通会话的通用入口。

## 直接编辑官方 Agent

1. 在官方列表选择要调整的 Agent，例如 **伙伴**。
2. 在能力页启用或停用能力，调整具体操作；在组件页选择合法的能力实现。
3. 在设置页调整名称、描述、人设与指令，在运行策略页调整默认行为。
4. 配置可用的对话模型，然后点击 **保存**。

保存修改的是当前用户的官方 Agent 配置，保留同一个 Agent 身份；不要求创建副本。其他用户的配置互不影响。后续从对应官方入口创建会话时使用这份已保存配置。官方模板更新不会覆盖当前数据集里已保存的能力、指令或组件选择。

官方伙伴保留“伙伴”和“伙伴记忆”两项核心能力，其余能力和操作可自由调整。

已有会话继续使用它们冻结的配置。普通会话中选择另一个 Agent 会直接应用，通过原 Session owner 的闲置、资源和绑定版本检查，从下一回合生效，不增加能力审阅或交接确认弹窗。

保存伙伴 Agent 配置也不会暗改已经打开的伙伴会话。需要应用已保存配置时，在伙伴概览或聊天窗口的 Agent 控件中明确选择 **应用已保存的更新**；该操作直接提交现役配置应用命令，并保留同一个 Session、当前模型、仍适用的资源与历史。活动回合、暂停回合或未结算效果会阻止更新，界面给出具体原因。可选资源缺失只使相应操作不可用，不补造连接或权限；服务端仍校验 owner、目标配置和 binding version。

| 对象 | 所属方 | Agent 工作台能做什么 |
| --- | --- | --- |
| Package / Agent Module | Agent 平台内部 | 只读取已正式物化的内置来源和 provenance |
| 应用 MCP 工具 | 应用托管与现役 MCP owner | 发现应用来源并显式选择给 Agent/会话，不自动启动程序 |
| Capability Catalog | 平台能力目录域 | 查询能力、来源、合同、支持的消费者和 availability |
| Skill Catalog | 平台技能目录域 | 选择 instruction/workflow；Skill 本身不是执行器 |
| AgentPreset | Agent authoring 域 | 保存用户意图和当前 Revision 引用 |
| AgentPresetRevision | Agent authoring 域 | 保存不可变 payload、ContributionLock 和 revision digest |
| Agent Session | Agent 运行域 | 消费冻结的 Snapshot，不反向改写 Revision |

应用的当前目录、命令、进程和 MCP 地址归应用托管 owner。Agent 仅通过现役 MCP 链
发现并调用明确选择的工具，不复制可写来源配置，也不持有另一份进程。

需要独立的个人 Agent 时，可以点击 **创建副本**。此操作明确创建新身份，不改变原来的官方 Agent 配置。个人 Agent 可以删除，官方入口保留。

## 创建个人 Agent

点击 **新建** 创建空白 Agent，选择能力、组件实现、身份、指令与运行策略，然后保存。保存后可以通过 **使用 Agent** 开始会话，也可以在支持个人 Agent 的产品选择器中选择它。具体知识库、工作区或伙伴等资源由会话或对应产品入口配置，工作台不保存另一份资源绑定。

## 能力和扩展

| 种子 | 初始内容 | 不会自动做什么 |
| --- | --- | --- |
| 轻量 | 身份、指令和模型路由 | 不加入工具、Workspace、MCP 或应用工具 |
| 通用 | 官方常用 Capability/Skill | 不吸收用户安装的全部扩展 |
| 全面 | 官方 Coding 基线 | 不安装或纳入全部应用工具 |
| 自定义 | 空白或可选种子 | 不允许填写内部 ID、Digest 或 Runtime 参数 |

工作台读取平台能力目录中已启用、完成物化且支持当前消费者的能力。未安装或不可用的能力不能通过编辑配置获得执行权限。操作授权、依赖闭包、冲突和组件兼容性由服务端统一校验。需要多个合法实现之一时，在组件页明确选择来源。

MCP 与 Skill 使用全局扩展目录及会话选择，不通过官方预设或个人 Agent 保存扩展总开关。安装、凭证、启停和服务生命周期由各自的设置与现役领域 owner 管理。

## 保存与会话边界

编辑内容先存在于草稿。保存由 canonical compiler 校验，并原子创建不可变 `AgentPresetRevision`，推进当前 Revision 指针。官方配置与个人配置共用这条保存链，不增加第二份配置或权限台账。

会话创建时冻结实际使用的能力、组件、模型、工具合同和资源到 Snapshot。保存 Agent 不改写已记录的 Revision、Snapshot、会话事件或正在执行的回合。已有会话的配置变化必须通过已有 canonical binding transition，在空闲、资源清理和效果结算边界显式提交。

工作台从平台 Capability Catalog 查询能力。Catalog 条目至少包含稳定能力 ID、合同
digest、owner、Package 或应用 MCP 来源、支持的消费者、typed contribution、所需资源和
按消费者区分的 availability。

Capability 没有独立版本号。Preset、依赖和冲突只引用稳定能力 ID；内置实现由 Package
版本、合同 digest、ContributionLock 与 Snapshot digest 冻结，因此更新实现不需要同步改写工作台引用。

- 只有已启用并完成 materialization 的内置贡献，以及明确选择的应用 MCP 工具，才能进入正式发现面；
- 托管应用的连接凭据和私有业务数据不进入 Agent Snapshot；
- 标记为 non-Agent-only 的能力不会出现在 Agent picker；
- 能力不可用、合同不匹配或资源缺失时显示可解释状态，并 fail closed；
- Skill 只提供 instruction/workflow 和资源说明，不自动扩张 Snapshot。

如果同一 canonical 能力存在多个合法实现，用户只能在能力详情中显式选择来源。
选择结果由服务端生成 ContributionLock；前端不提交内部 Artifact digest
来驱动执行。

## Revision 与 Snapshot

用户编辑内容先存在于不可执行 Draft。Save 成功后，服务端原子创建不可变
`AgentPresetRevision`，并推进当前 Revision 指针。Preview、Save 和 Test 使用同一个
canonical Compiler；Test 创建普通 AgentSession，不创建 test-only Session。

Revision digest 覆盖规范化的：

```text
payload + contribution_locks
```

Snapshot 进一步冻结本次实际执行所需的 Capability、Provider/Model route、Tool schema、
typed resource binding、Runtime feature、initial/on-demand 分组和 Snapshot digest。
Session Open 读取已保存 Snapshot，不在每个 Turn 中重新选择 latest 来源，也不静默
切换 Provider 或 fallback。

存储与切换合同以 [Agent Session 架构](../architecture/agent-session.zh.md) 和当前 canonical baseline 为准；不从退役的 Conversation 或旧预设字段恢复数据。

## Canonical API（机器资源名）

工作台使用以下 canonical API；API 的稳定机器名称不代表 UI 必须显示“Preset”：

官方模板是用于初始化和显式复制的发布基线。模板更新只影响尚未保存的默认配置；已保存的个人官方配置不会静默漂移。

## API 与实现入口

| 用途 | Endpoint |
| --- | --- |
| 模板及当前用户的官方、个人配置目录 | `GET /api/agent-preset-templates` |
| 创建个人 Agent | `POST /api/agent-presets` |
| 从官方初始模板显式创建个人配置 | `POST /api/agent-presets/from-template/{template_id}`，`reuse_existing=false` |
| 为官方产品入口准备已保存配置 | 同上，`reuse_existing=true` |
| 读取统一编辑器 | `GET /api/agent-presets/{preset_id}/editor` |
| 保存不可变 Revision | `POST /api/agent-presets/{preset_id}/revisions` |
| 能力和组件目录 | `GET /api/agent-catalog` |
| 会话及显式 Agent 更新 | `/api/agent-sessions/*` |

目录中的 `official_templates` 是初始模板；`official_presets` 按模板 key 返回当前用户的稳定可编辑身份；`user_presets` 只包含个人 Agent。客户端不能把模板、来源标记或内部 ID 当作授权，owner、能力与 Snapshot 校验由服务端负责。

## 实现边界

Agent revision、Snapshot、Session 与 Module 授权的当前实现见
[Agent Session 架构](../architecture/agent-session.zh.md)。

应用的导入、预览、当前设置和启停请见
[应用托管架构](../architecture/app-hosting.zh.md)。

当前运行边界见 [Agent Session 架构](../architecture/agent-session.zh.md)，Plugin 管理见 [插件平台架构](../architecture/plugin-platform.zh.md)。
