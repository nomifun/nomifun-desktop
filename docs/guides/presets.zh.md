# Agent 工作台

从侧边栏进入 **Agent**（`/agent`），可直接编辑官方 Agent，或创建自己的 Agent。官方列表包含日常对话、通用、编程、伙伴、客服和创作六个入口。

## 直接编辑官方 Agent

1. 在官方列表选择要调整的 Agent，例如 **伙伴**。
2. 在能力页启用或停用能力，调整具体操作；在组件页选择合法的能力实现。
3. 在设置页调整名称、描述、人设与指令，在运行策略页调整默认行为。
4. 配置可用的对话模型，然后点击 **保存**。

保存修改的是当前用户的官方 Agent 配置，保留同一个 Agent 身份；不要求创建副本。其他用户的配置互不影响。后续从对应官方入口创建会话时使用这份已保存配置。官方模板更新不会覆盖当前数据集里已保存的能力、指令或组件选择。

官方伙伴保留“伙伴”和“伙伴记忆”两项核心能力，其余能力和操作可自由调整。

已有会话继续使用它们冻结的配置。对桌面伙伴，在伙伴概览或聊天窗口的 Agent 控件中选择 **应用已保存的更新**，查看能力变化并确认，即可在同一个空闲会话中应用已保存的伙伴配置。活动回合、暂停回合或未结算效果会阻止更新；伙伴模型及仍需使用的资源选择、参数保留，停用能力后移除的资源在预览中显示。增加能力所需的资源尚未配置时，先按提示补齐资源。预览后配置再次改变时，需要重新预览并确认。

需要独立的个人 Agent 时，可以点击 **创建副本**。此操作明确创建新身份，不改变原来的官方 Agent 配置。个人 Agent 可以删除，官方入口保留。

## 创建个人 Agent

点击 **新建** 创建空白 Agent，选择能力、组件实现、身份、指令与运行策略，然后保存。保存后可以通过 **使用 Agent** 开始会话，也可以在支持个人 Agent 的产品选择器中选择它。具体知识库、工作区或伙伴等资源由会话或对应产品入口配置，工作台不保存另一份资源绑定。

## 能力和扩展

工作台读取平台能力目录中已启用、完成物化且支持当前消费者的能力。未安装或不可用的能力不能通过编辑配置获得执行权限。操作授权、依赖闭包、冲突和组件兼容性由服务端统一校验。需要多个合法实现之一时，在组件页明确选择来源。

MCP 与 Skill 使用全局扩展目录及会话选择，不通过官方预设或个人 Agent 保存扩展总开关。安装、凭证、启停和服务生命周期由各自的设置与 Plugin Core 管理。

## 保存与会话边界

编辑内容先存在于草稿。保存由 canonical compiler 校验，并原子创建不可变 `AgentPresetRevision`，推进当前 Revision 指针。官方配置与个人配置共用这条保存链，不增加第二份配置或权限台账。

会话创建时冻结实际使用的能力、组件、模型、工具合同和资源到 Snapshot。保存 Agent 不改写已记录的 Revision、Snapshot、会话事件或正在执行的回合。已有会话的配置变化必须通过已有 canonical binding transition，在空闲、资源清理和效果结算边界显式提交。

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

当前运行边界见 [Agent Session 架构](../architecture/agent-session.zh.md)，Plugin 管理见 [插件平台架构](../architecture/plugin-platform.zh.md)。
