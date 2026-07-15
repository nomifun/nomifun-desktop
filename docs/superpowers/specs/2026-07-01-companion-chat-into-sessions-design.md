# 桌面伙伴聊天迁移进「会话」— 设计文档

日期：2026-07-01
状态：已与用户确认设计要点，进入实现

## 背景与问题

用户反馈：「桌面伙伴」的聊天页面藏在 `/nomi`（桌面伙伴配置中心）的一个"聊天"Tab 里，而不是在「会话」体系中，进入路径割裂。

希望：
1. 让伙伴的聊天也出现在「会话」中。
2. 在会话侧边栏出现一个专属的工作空间分组「桌面伙伴」，把所有创建的伙伴放置上去。
3. 该分组不需要终端会话类型，伙伴都是交互式会话。

## 关键现状（探索结论）

- **伙伴聊天本就是真实会话**：每个伙伴背后是一条真实的 `type:'nomi'` conversation，与普通会话同表、复用同一聊天引擎（`ChatLayout` + `NomiChat`）。它们只是被前端过滤器 `useConversationListSync.ts:123`（`isCompanionConversation`）**刻意隐藏**在会话列表之外。
- **一个伙伴 = 恰好一条会话**：通过 `ensureCompanionSession`（幂等，缺失则创建）解析，称"单会话契约"。
- **`/nomi` 已是侧边栏中标注「桌面伙伴」的入口**（i18n `nomi.siderTitle`），目前是配置中心 + 一个聊天 Tab。
- **会话列表已有"工作空间(workpath) → 类型(交互/终端)"分组**（`SessionList/`），新增一个专属「桌面伙伴」分组顺理成章。
- **会话视图的 nomi 分发点**在 `ChatConversation.tsx:357`：`type==='nomi'` → `NomiConversationPanel`（全功能面板）。
- **浮窗桌宠**（`/companion` 窗口）与伙伴共享同一会话与 `extra` 标记；其"去聊天"深链 `pages/companion/index.tsx:1317` 现指向 `/nomi?...&tab=chat`，聊天 Tab 移除后会失效，需要改向会话。

## 用户已确认的决策

1. **`/nomi` 保留为管理中心，移除聊天 Tab**。配置（形象/记忆/技能/知识/设置等）仍归宿于此。
2. **伙伴会话在「会话」视图打开时保留伙伴专属限制**：锁定模型（仅可在管理中心改）、隐藏高级控制、强制 yolo、固定工作区。行为与现在一致，只换入口。
3. **新建伙伴仍只在管理中心**；会话分组只展示已有伙伴（不提供"＋新建"）。

## 采用方案：花名册驱动分组 + 复用 `/conversation/:id`

伙伴会话本就是真实会话；把它们以**花名册驱动**的「桌面伙伴」分组呈现在会话侧边栏顶部（数据源 `useCompanions()`，因此能显示头像/等级/状态，并列出**全部**伙伴），点击在标准 `/conversation/:id` 视图打开——通过 `extra.companionSession` 分支保持伙伴受限聊天。**无后端改动。**

被否决的备选：
- *过滤器驱动*：放开过滤器并把伙伴会话塞进特殊 workpath 节点。否决——丢失伙伴元数据、可能泄漏进项目分组、只能显示已有会话的伙伴。
- *独立路由 `/companion-chat/:id`*：否决——不够"统一在会话里"，重复活动行逻辑。

## 设计细节

### 1. 会话侧边栏新增「桌面伙伴」分组（`pages/conversation/SessionList/`）
- 新增 `CompanionSessionGroup.tsx`，渲染在 `WorkpathSessionList` **顶部**、项目/workpath 树之上。数据源 `useCompanions()`，每个伙伴一行（头像 + 名字 + Lv/模型就绪点，复用 `CompanionSessionRail` 行视觉）。
- **仅交互式**：无 `SessionKindGroup`（无终端会话子组），无"＋新建"（创建留在 `/nomi`）。可有一个小"管理/齿轮"跳到 `/nomi`。
- 点击行 → `ensureCompanionSession({ companion_id })` → `navigate('/conversation/{conversation_id}')`。活动高亮：把 伙伴↔conversation_id（缓存）与路由 id 比对。
- `buildWorkpathTree` **保持不动**；伙伴会话仍被过滤出项目分组，避免重复列出。

### 2. 在会话视图打开伙伴聊天（`pages/conversation/components/ChatConversation.tsx`）
- 在第 357 行处分支：`extra.companionSession === true` → 渲染新的轻量 `CompanionChatPanel`（由 `extra.companionId` 推导 companionId，取 `useCompanion(companionId)`，再渲染**现有** `CompanionConversation` 及其当前受限 props + 当前 `ChatTab` 的"模型未配置"引导门禁）。否则 → 现有 `NomiConversationPanel`。
- 保留全部限制，仅把入口搬进会话。

### 3. `/nomi` 变为纯管理中心（`pages/nomi/index.tsx`、`tabs/ChatTab.tsx`）
- 从伙伴 Tab 列表移除 `'chat'`；默认 Tab → `overview`。退役/删除 `ChatTab`（其 ensure-session + 门禁逻辑迁入 `CompanionChatPanel`）。
- 提供"打开聊天"动作（overview 与创建后）：ensure 会话并导航至 `/conversation/:id`，使创建伙伴后仍能直接进入聊天。

### 4. 桌宠深链修正（`pages/companion/index.tsx:1317`）
- 把"去聊天"导航从 `/nomi?...&tab=chat` 改为 `/conversation/{conversation_id}`，使用桌宠已解析的 thread id。`memories`/`settings`/`suggestions` 深链仍留在 `/nomi`。

### 5. i18n
- 在 zh-CN 与 en-US 的 `sessionList.json` 增加 `sessionList.companionGroup`（"桌面伙伴" / "Desktop Companions"）。

### 6. 保持不动（不可破坏）
- `ensureCompanionSession`/`getCompanionSession`、`extra` 标记（companionSession/companionId/channelPlatform）、工作会话过滤器、`sync_companion_windows`、`conversation_id` 的 canonical `ConversationId` 边界校验、IM 渠道折叠入单会话。禁止 string/number 兼容强转。**无 Rust/后端改动。**

## 受影响文件（预估）

新增：
- `ui/src/renderer/pages/conversation/SessionList/CompanionSessionGroup.tsx`（+ 可能的 `CompanionSessionRow.tsx`）
- `ui/src/renderer/pages/conversation/components/CompanionChatPanel.tsx`（薄适配器）

修改：
- `ui/src/renderer/pages/conversation/SessionList/index.tsx`（渲染分组）
- `ui/src/renderer/pages/conversation/components/ChatConversation.tsx`（357 行分支）
- `ui/src/renderer/pages/nomi/index.tsx`（移除 chat tab、默认 overview、打开聊天动作、创建后导航）
- `ui/src/renderer/pages/nomi/tabs/ChatTab.tsx`（退役/删除）
- `ui/src/renderer/pages/companion/index.tsx`（1317 行深链）
- `ui/src/renderer/services/i18n/locales/{zh-CN,en-US}/sessionList.json`

## 验收标准

1. 会话侧边栏顶部出现「桌面伙伴」分组，列出全部已创建伙伴（头像/名字/状态），无终端子组、无新建入口。
2. 点击伙伴行在 `/conversation/:id` 打开其聊天，受限行为与旧聊天 Tab 一致（锁模型、隐藏高级控制、yolo、固定工作区、模型未配置引导）。
3. `/nomi` 不再有聊天 Tab，仍可管理形象/记忆/技能/知识/设置；创建伙伴后可一键进入聊天。
4. 桌宠"去聊天"打开会话视图而非失效的 `/nomi?tab=chat`。
5. 桌宠窗口、IM 渠道、单会话契约不受影响。
6. `bun run typecheck` 通过；i18n 校验通过。
