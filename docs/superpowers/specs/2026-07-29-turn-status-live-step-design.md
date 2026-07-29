# 会话交互页任务执行状态与实时步骤设计

日期：2026-07-29
状态：已定稿（依据用户提供的详细需求推导；实现前欢迎复核）

## 背景与目标

当前会话页在任务执行期间，回合头部显示「处理中 37秒」，完成后变为「已处理 1分 20秒」；执行步骤（思考/工具/权限行）与回复内容混杂，用户不易判断任务是否仍在执行、正在做什么、是否已完成。

目标（参考 Codex 会话页的信息层级与克制感，但保持 NomiFun 品牌色、布局与组件体系）：

1. 回合头部状态统一为「已处理 {耗时}」，不再出现「处理中」；
2. 在最新 AI 回复内容底部（输入框上方）新增实时「处理步骤」区，作为"仍在执行"的主要视觉信号，完成后自动消失；
3. 失败：头部仍为「已处理 {耗时}」，底部步骤区消失，回复区错误卡片提供简短说明与重试入口；
4. 用户主动停止：头部显示「你在 {已耗时} 后停止了」，底部步骤区消失。

## 现状要点（探索结论）

- 头部组件 `TurnProcessDisclosure.tsx`：`labelKeyByState` 将 `running → messages.turnProcessing`（"处理中 {{duration}}"）、`waiting → turnWaiting`、`canceled → turnCanceled`（"已取消 {{duration}}"）；`failed` 已被两层机制强制归并为 completed（模型层 `turnDisclosureModel.ts:242-248`、组件层 `:199`）。运行中有 1s ticker 实时刷新耗时。
- 回合状态机 `turnDisclosureModel.ts`：`buildTurnDisclosureItems` 按 turnId 分段；活跃回合 `running/waiting`，关闭回合按末项 `canceled`/否则 `completed`。`startAt` 取用户请求时间，`endAt` 取最终回复时间。
- 「当前步骤」逻辑已存在但只在展开明细里：`TurnProcessDisclosure.currentItemKey`（findLast running/waiting）；步骤文案由 MessageList 私有函数 `buildProcessReceiptSummary`（映射到 `messages.processReceipt.*`：正在读取/正在编辑/正在运行/正在准备下一步操作等）产出，视觉由 `TurnProcessReceipt`（running → Arco Spin 12px）承载。
- 停止：前端 stop（`NomiSendBox.handleStop` → `useNomiMessage.resetState`）不写任何取消标记，「已取消」只在后端最后一个工具 status=Canceled 时出现；停止时刻的耗时未被记录，停止后头部耗时会回跳变小。
- 失败：错误以 `tips(type=error)` 渲染为 `.message-error-note` 卡片（标题/说明/技术详情/反馈按钮），**无重试入口**。共享 SendBox 已支持 `sendbox.edit` 事件（回填输入框进入编辑模式，提交即截断重跑，仅 Nomi 平台提供 `onEditResubmit`）。
- 约束：仅 en-US/zh-CN 两个 locale，新 key 需双语言 + `bun run gen:i18n`；结构测试（`turnProcessLayout.structure.test.ts` 等）钉死大量源码字符串，需同步更新；UI 测试为 bun test（无 DOM，纯函数 + renderToStaticMarkup + 源码结构断言）。

## 设计

### 1. 头部统一「已处理 {耗时}」

改 `TurnProcessDisclosure.labelKeyByState`：`running → 'messages.turnProcessed'`（其余不变）；`defaultLabelByState.running → 'Processed {{duration}}'`。运行中沿用现有 1s ticker，即显示「已处理 37秒」并持续增长；完成后停在最终总耗时（现有 `endAt` 行为）。

- `waiting`（等待权限确认）保留「等待确认 {{duration}}」：它不是"处理中"，且是需要用户行动的强信号；需求仅禁止「处理中」。
- `messages.turnProcessing` key 从两个 locale 中删除并重新生成 i18n 类型（无其他引用）。
- 状态样式类（`--running/--waiting/--canceled`）保持不变。

### 2. 底部实时「处理步骤」区

**位置**：消息列表滚动区内容末尾（最新 AI 回复内容之后），即输入框上方。流式期间自动跟随滚动保持可见（`useAutoScroll` 已有 pinned-to-bottom 行为）；这与 Codex 的内联位置一致。选择内联而非悬浮停靠，是因为 PinnedPlan 已占用 SendBox 上方的停靠锚点，且内联可完全复用消息列表已有数据流。

**出现条件**：`conversationContext.isProcessing === true` 且列表末段回合的 disclosure `running === true`。完成/失败/停止后随 `running=false` 自动消失。

**内容选择**（新纯函数模块 `turnLiveStepModel.ts`，可单测）：按优先级取当前步骤：
1. findLast 处于 `waiting` 的过程项 → 该项 receipt 文案（如「等待确认 xxx」，警示色）；
2. findLast 处于 `running` 的过程项 → thinking 项用 `messages.processReceipt.thinkingRunning`（正在思考），其余用 `buildProcessReceiptSummary` 的 running 文案（正在读取/正在编辑/正在运行/正在搜索…，带目标名）；
3. 无运行项但最终回复文本正在流式输出 → 新 key `messages.turnLiveStep.composing`（正在整理回复）;
4. 回合刚开始、尚无过程项 → 新 key `messages.turnLiveStep.analyzing`（正在分析需求）;
5. 其余间隙 → 复用 `messages.processReceipt.preparingAction`（正在准备下一步操作）。

**渲染**：MessageList displayList 末尾追加 `ITurnLiveStepVO`（`type:'turn_live_step'`，id=`turn-live-step-${turnId}`，label/state/icon），渲染时复用现有 `TurnProcessReceipt`（`hasDetail:false` 静态行：running → Spin + 文案），外层 `.turn-live-step` 容器加：
- 文案轻呼吸动效（opacity 1↔0.55，~1.8s，新 keyframes 定义在 messages.css）；
- `@media (prefers-reduced-motion: reduce)` 关闭动效（页面惯例）；
- waiting 态警示色（`var(--color-warning-6)`）、不加呼吸；
- 标签单行省略号，窄屏（≥360px 最小宽度）自适应，不需额外断点。

不引入新图标、新色值（全部走既有 token 与 `TurnProcessReceipt` 图标映射），`check:theme`/`check:icons` 天然通过。

### 3. 停止态「你在 {已耗时} 后停止了」

**文案**：`messages.turnCanceled` 改为 zh「你在 {{duration}} 后停止了」/ en「You stopped after {{duration}}」（保留 `{{duration}}`，结构测试兼容）。后端 Canceled 工具导致的取消也用此文案（实际来源即用户停止）。

**可靠触发 + 准确耗时**（现状两个缺口：停止时若无 Canceled 工具则头部显示已处理；停止时刻耗时未记录会回跳）：
- `useNomiMessage` 新增 `stopNotice: { stoppedAt: number } | null` 状态：`resetState()`（用户点停止，乐观路径）置 `{ stoppedAt: Date.now() }`；`restoreRunningAfterStopFailure()` 清空；新回合开始（`setWaitingResponse(true)`、`acceptStart()`）与会话切换时清空；随 runtime 返回。
- `NomiChat` 将其放入 `ConversationContext`（`ConversationContextValue` 新增可选字段 `stopNotice`）。
- `buildTurnDisclosureItems` 新增可选 option `stopNotice?: { stoppedAt: number }`：对输出中最后一个 turn_disclosure，若已关闭（running=false）且 `stoppedAt >= startAt`，覆写 `state='canceled'`、`endAt=stoppedAt`（已是 canceled 的也覆写 endAt，保证「已耗时」为停止时刻）。模型层实现，纯函数可单测。
- 会话重载后 stopNotice 不持久化：有后端 Canceled 标记的仍显示停止文案，否则回落为「已处理」。可接受（本需求聚焦实时交互）；如需持久化属后端改动，超出本次范围。
- 仅 Nomi 平台接入 stopNotice（本需求场景）；其余平台（acp/openclaw/nanobot/remote）保持现状，仅受益于文案变化。

**底部步骤区**：停止的乐观路径立即 `running=false` → 步骤区消失。✓

### 4. 失败态重试入口

- 头部已处理 ✓（failed 归并 completed 为既有行为，保持）；底部步骤区随 running=false 消失 ✓；错误说明卡片已有 ✓。
- 在 `MessageTips` 错误分支（结构化卡片 `message-error-note__actions` 与普通错误分支）新增「重试」按钮（`common.retry`）。显示条件：`conversationContext.type === 'nomi'`、非 readOnly、`isProcessing !== true`、且该错误属于最新一次用户请求（通过 `useMessageList()` 找到最后一条 right 文本，其 created_at 早于本 tips）。
- 点击行为：`emitter.emit('sendbox.edit', { msgId, createdAt, content })`（最后一条用户消息）——复用共享 SendBox 既有「编辑重发」通道：原文回填输入框并聚焦，提交即截断重跑。不新增直发通道（避免误触重复扣费与五个平台 SendBox 的无关改动）。
- 附带修复：补齐被引用但缺失的 `conversation.stop.failed` locale key（zh/en）。

### 数据流小结

```
useNomiMessage(stopNotice, running) → NomiChat(resolvedIsProcessing, stopNotice)
  → ConversationContext → MessageList
      → buildTurnDisclosureItems(…, {tailClosed, activeTurnId, stopNotice})
          → turn_disclosure VO(state/startAt/endAt) → TurnProcessDisclosure（头部「已处理/你在 X 后停止了」）
      → planTurnLiveStep(尾段 disclosure + 流式回复判定) → ITurnLiveStepVO
          → TurnProcessReceipt（底部实时步骤行）
MessageTips(错误卡片) --sendbox.edit--> SendBox（重试回填）
```

## 测试与验证

- 新增：`turnLiveStepModel.test.ts`（步骤选择优先级、消失条件）；`turnDisclosureModel.test.ts` 增补 stopNotice 覆写用例（含 stale stoppedAt 防护）；TurnLiveStep 渲染结构断言（renderToStaticMarkup 或源码结构测试，含 reduced-motion 与呼吸动效类名）；MessageTips 重试按钮结构断言。
- 更新：`turnProcessLayout.structure.test.ts`（running→turnProcessed 映射、turnProcessing 移除）；涉及 locale 的断言。
- 门禁：`bun test ui/src/renderer/pages/conversation/Messages`（及新测试路径）、`bun run gen:i18n`、`bun run check`（typecheck + i18n + theme + icons 等）。
- 运行时验证限制：本机（Windows）无 Mac 后端运行时，无法端到端驱动真实会话；以单测 + 结构测试 + 类型/契约检查作为门禁，并在交付说明中如实标注。

## 非目标

- 不改动过程明细行、思考块、交付物卡片等既有视觉；
- 不为 acp/openclaw/nanobot/remote 平台接入 stopNotice；
- 不持久化 stopNotice（重载后回落）；
- 不改变完成后 disclosure 的展开/收起策略。
