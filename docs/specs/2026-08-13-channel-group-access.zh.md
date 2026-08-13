# 渠道群聊访问策略（2026-08-13）

> 状态：实现契约。本文定义渠道机器人群聊准入的最小产品与安全边界；字段、迁移、前端和运行时实现应保持一致。

## 1. 目标与边界

目标是在不引入组织通讯录、角色系统或另一套 ACL 的前提下，让机器人负责人之外的群成员也能使用机器人：

1. 每个机器人独立选择“对群所有成员开放”“对群部分成员开放”或“禁止群聊”；
2. 继续复用现有 Pending / Authorised users，负责人不需要重新维护成员名单；
3. 群消息必须通过平台提供的结构化 `@机器人` 唤醒，避免机器人监听普通群聊；
4. 群聊准入不能隐式扩大私聊权限，群与私聊的会话记录必须隔离；
5. 无法可靠识别群聊或 mention 的适配器暂不展示设置，运行时按拒绝处理。

本次不建设企业目录同步、群管理员同步、群级角色、关键词唤醒或跨机器人共享授权。

## 2. 数据契约与三档模式

字段保存在单个 `channel_plugins` 机器人行上：

```text
group_access_mode: all_members | allowlist | disabled
```

| 用户选项 | Wire 值 | 准入规则 |
| -------- | ------- | -------- |
| 对群所有成员开放 | `all_members` | 任意有稳定 sender id 的群成员在结构化 `@` 当前机器人后可以使用，无需配对。 |
| 对群部分成员开放 | `allowlist` | 只有该机器人已有的 Authorised users 可以使用；其他成员的有效 `@` 创建或复用 Pending 请求。 |
| 禁止群聊 | `disabled` | 所有群消息均被忽略，负责人和已授权用户也不例外。 |

字段是 **per-bot**，不能按平台全局保存。相同平台上的两个机器人不得互相继承模式、Pending、Authorised users 或群会话。

默认与迁移规则：

- 新建伙伴机器人默认 `allowlist`；
- 已有非客服机器人回填 `allowlist`；
- 为保持陌生访客自动服务语义，新建客服机器人默认 `all_members`；迁移时已有的客服机器人也回填 `all_members`，负责人之后可修改；
- 非法枚举值必须在写入边界拒绝，不做宽松字符串推断。

## 3. 消息处理顺序

运行时必须按以下顺序处理，不能先创建配对或 session 再补做群聊检查：

```text
平台事件
  → 过滤机器人自身消息 / 重放事件
  → 解析 direct 或 group
      → direct：沿用既有私聊授权或客服语义
      → group：校验 sender id + 当前机器人的结构化 mention
          → 读取该 plugin_id 的 group_access_mode
          → disabled：忽略
          → allowlist：检查该机器人的 Authorised user；否则创建/复用 Pending
          → all_members：建立仅限该群的临时准入身份
          → 建立或复用该“机器人 + 群”的独立 session
          → 路由到所绑定的伙伴或客服
```

只有平台事件中的结构化 mention 才有效。正文中看似 `@bot` 的普通文本、引用消息里的名字、对其他机器人的 mention 和未 mention 消息都必须忽略，且不能产生 Pending、Authorised user、session 或 Agent 调用。

若 chat kind、sender id、bot id 或 mention 信息缺失 / 未知，群路径必须 fail closed。不得靠群名称、成员数量、chat id 前缀或自由文本猜测。

## 4. 身份、配对与私聊隔离

- `allowlist` 复用既有 per-bot Pending 和 Authorised users；不增加群成员目录。
- 未授权成员的有效群 `@` 只在负责人 UI 中产生待批准请求，不把 6 位配对码公开回发到群里。
- `all_members` 创建或复用 `authorization_kind=auto_group` 的隐藏 per-bot 访客身份，以保持群 session 稳定；客服私聊的自动接待访客也使用同一种“自动准入但未批准”身份。它不会出现在 Authorised users 中，也不能用于普通私聊或群 allowlist 授权。
- `all_members` 群成员之后私聊伙伴机器人时，仍按既有配对流程处理。
- 私聊授权用户被撤销后，其私聊 session 清理；其群聊行为重新由当前模式决定。
- `disabled` 不影响私聊授权、客服私聊或机器人连接状态。

对于绑定伙伴且使用 Nomi 的机器人，`auto_group` 是强制的 model-only 能力上限：只允许模型为当前群消息生成回复，不提供 cron、AutoWork、需求、记忆、跨会话、终端、浏览器、computer-use、自定义 MCP 或平台 Gateway 工具，也不加载私有知识挂载。通过 `allowlist` 明确批准的 `authorization_kind=approved` 身份保留既有的已授权渠道能力；看到已批准身份的开放群事件不得把它降级成 `auto_group`。

## 5. 会话隔离

伙伴机器人接受群消息后，session 至少按“机器人 + 群”隔离，并满足：

- 不进入负责人在桌面端或 IM 私聊中的 Conversation；
- 不进入任何群成员的私聊 Conversation；
- 不与同一机器人的另一个群共享上下文；
- 不与同一平台上的另一个机器人共享上下文；
- 切换绑定或影响路由的配置时，只清理受影响机器人对应的 session。

`all_members` 是入口准入，不代表负责人身份，也不能据此签发私聊或 owner 级权限。即使 Conversation 在物理上归本机 owner 所有，`channel_group_guest` 仍必须在 Agent factory 的统一权限上限处把 `auto_group` runtime 降为 model-only。

## 6. 客服兼容语义

群聊访问检查同样作用于绑定客服的机器人，但通过检查后的运行域不变：

- 入站消息整体交给客服域，不进入伙伴 Conversation 或平台 Gateway；
- 每条客服消息仍使用 disposable one-shot engine session；
- 工具表固定为三个只读工具：知识检索、知识阅读、客服笔记检索；
- 客服私聊继续沿用既有自动服务语义，不被伙伴配对规则替换；
- 群聊中的 `all_members` 身份不会成为伙伴或私聊授权。

## 7. 平台能力与 UI 显示

只有适配器能可靠提供 direct/group 分类、sender id、bot id 与结构化 mention 时，UI 才显示群聊访问设置。

| 适配器 | 群聊设置 | 依据 / 约束 |
| ------ | -------- | ----------- |
| Telegram | **暂不显示** | 当前适配器虽能识别 chat type，但尚未提供本契约要求的结构化 bot mention。 |
| 飞书（Lark，key=`lark`） | 显示 | `chat_type` 与 mentions 可结构化识别。 |
| 钉钉 | 显示 | `conversation_type` 与 at-list 可识别。 |
| 企业微信（WeCom） | 显示 | 长连接事件提供 single/group；群消息由平台 `@` 触发。 |
| QQ Bot | 显示 | C2C、group、guild、DM 事件类型可区分；群路径必须确认结构化 mention。 |
| Discord / Slack / Mattermost | 显示 | 适配器已有稳定的群 / 频道识别与 mention 信号。 |
| Matrix | **暂不显示** | 当前事件不能稳定判定 direct 与 group，不允许猜测。 |
| 微信 / Nostr | 不显示 | 当前实现按私聊使用。 |
| Twitch | 不显示 | 当前通道缺少本契约要求的可靠群类型与结构化 mention 组合。 |

隐藏设置不是放宽限制。任何意外进入群路径、但缺少可靠元数据的事件仍应拒绝。

## 8. 命名与兼容性

- 中文用户可见主称统一为**飞书（Lark）**；英文可写 **Feishu (Lark)**。
- 协议枚举、feature、API / 配置 key 和数据库 type 继续使用 `lark`，不做破坏性改名。
- 企业微信使用 `wecom`，QQ Bot 使用 `qqbot`；两者都是可运行的内置适配器，不应再标为 placeholder。
- `group_access_mode` 是稳定 wire 字段；负责人通过 `POST /api/channel/settings/group-access` 提交 `{ plugin_id, group_access_mode }`，接口不接收渠道凭据，也不重启连接。模式与清理在同一事务内更新：取消并删除该机器人的群聊与未知范围 session / 待处理队列，保留私聊 session。接口返回后，新消息不会再按旧策略准入；写锁取得前已经完成持久化准入的单次回复可以正常收尾。

## 9. 验收条件

1. 同一平台的两个机器人可以保存不同模式，互不影响。
2. 三种模式下，未结构化 mention 的群消息均不响应且不产生副作用。
3. `allowlist` 中未授权成员产生一条可批准的 per-bot Pending；批准后可用，撤销后立即失效。
4. `all_members` 中陌生成员可在群内获得 model-only 回复，随后私聊仍需配对，且其 `auto_group` runtime 不得出现 owner/channel 工具。
5. `disabled` 中负责人、已授权用户和陌生成员全部不能从群内使用，私聊不受影响。
6. 两个群、群与私聊、两个机器人之间的会话记录互不串联。
7. 客服群消息通过准入后仍只进入客服 one-shot 域；客服私聊行为保持兼容。
8. Matrix 等不可靠适配器不显示控件，缺失元数据的群事件 fail closed。
9. 升级后普通旧机器人为 `allowlist`，已有客服机器人为 `all_members`。
