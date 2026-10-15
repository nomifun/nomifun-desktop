# 消息渠道与群聊准入

渠道把外部消息接入现役伙伴或客服 owner，不创建第二套 Agent Runtime 或会话权威。用户操作见 [渠道指南](../guides/channels.zh.md)；本页保留跨适配器合同，实际平台能力由代码声明。

## 每机器人策略

`channel_plugins.group_access_mode` 独立保存于每个机器人：

| 值 | 群聊准入 |
| --- | --- |
| `all_members` | 有稳定 sender ID 且结构化 @ 当前机器人的成员可以进入群路径 |
| `allowlist` | 使用该机器人的已批准身份；其他有效 @ 创建或复用 Pending |
| `disabled` | 忽略全部群消息，包括负责人和已批准成员 |

伙伴机器人默认 `allowlist`，客服默认 `all_members`。非法值在写边界拒绝。相同平台的机器人不共享策略、Pending、已批准身份或会话。群策略不扩大私聊授权，`disabled` 不关闭私聊或连接。

处理先过滤机器人自身及重放消息，再判断 direct/group。群路径必须具有可靠 sender、bot、chat kind 和结构化 mention，之后才检查当前机器人策略、身份及 canonical 会话准入。正文中的 `@bot`、引用或其他机器人 mention 不生效；未知元数据拒绝处理，不能先创建 Pending、Session 或 Agent 调用再补校验。适配器不可靠时不显示群设置，也不放宽后端检查。

## 身份与会话

`allowlist` 复用 per-bot 配对与已批准用户；待批准群请求只显示给负责人，不把配对码发送到群。开放群使用隐藏的 `authorization_kind=auto_group` 身份，它不进入已批准列表，也不授予普通私聊或 allowlist 权限。已批准身份不能因开放群事件被降级。

绑定伙伴的 `auto_group` 在统一 Agent factory 处具有 model-only 上限：只回复当前群消息，不开放 owner 工具、私有知识、记忆、定时任务、跨会话、终端、浏览器、computer-use 或自定义 MCP。入口准入不代表安装 owner 权限。

群会话按机器人与群隔离，不与负责人桌面、成员私聊、其他群或机器人共享上下文。渠道通过当前 owner 与 typed binding 找到 Session；存在会话而缺少 authority binding 时报告冲突，不选择旧记录自动回绑。新建、重置与取消调用 canonical owner。

客服消息通过群检查后仍进入客服域；其会话与工具边界由当前客服 owner 和冻结 Agent 合同决定，不能把旧 one-shot 实施说明当作现役运行要求。

## 策略变更

`POST /api/channel/settings/group-access` 只接收机器人 ID 与策略，不接收凭据或重启连接。更新持有 per-bot 写围栏，在同一事务变更策略并清理该机器人的群及未知范围会话、待处理队列，保留私聊。入站与队列投递持有读围栏直到持久化 Turn handoff；更新返回后不能继续按旧策略准入，已持久化准入的工作按原 canonical 回执结算。

## 实现与验证

| 边界 | 源码 |
| --- | --- |
| 适配器与类型 | [plugin.rs](../../crates/backend/nomifun-channel/src/plugin.rs)、[types.rs](../../crates/backend/nomifun-channel/src/types.rs) |
| 入站、身份与策略 | [message_loop.rs](../../crates/backend/nomifun-channel/src/message_loop.rs)、[pairing.rs](../../crates/backend/nomifun-channel/src/pairing.rs)、[channel_settings.rs](../../crates/backend/nomifun-channel/src/channel_settings.rs) |
| 线性化围栏 | [group_policy.rs](../../crates/backend/nomifun-channel/src/group_policy.rs) |
| Session 接缝 | [session_port.rs](../../crates/backend/nomifun-channel/src/session_port.rs)、[app channel](../../crates/backend/nomifun-app/src/channel.rs) |

修改时定向覆盖三种策略、结构化 mention、缺失元数据、per-bot 隔离、开放群能力上限、私聊不受影响和更新/投递竞争。平台能力矩阵以 adapter 实现及其测试为准，不保留带日期的第二份列表。
