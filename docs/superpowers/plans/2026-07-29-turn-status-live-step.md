# 会话回合状态与实时步骤区实现计划

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 回合头部统一显示「已处理 {耗时}」（停止时「你在 {已耗时} 后停止了」），在最新 AI 回复底部新增实时「处理步骤」行，失败卡片增加重试入口。

**Architecture:** 数据模型层（`turnDisclosureModel`/新 `turnLiveStepModel`）保持纯函数并单测；`useNomiMessage` 记录用户停止时刻（stopNotice）经 `ConversationContext` 流入 `MessageList`；渲染完全复用既有 `TurnProcessDisclosure`/`TurnProcessReceipt` 组件与 token。

**Tech Stack:** React + TypeScript（ui/src/renderer）、bun test（无 DOM：纯函数 + 源码结构断言）、i18n JSON（en-US/zh-CN 双语言 + 生成类型）。

**Spec:** `docs/superpowers/specs/2026-07-29-turn-status-live-step-design.md`

## Global Constraints

- 新 locale key 必须同时加到 `ui/src/renderer/services/i18n/locales/zh-CN/messages.json` 与 `en-US/messages.json`，然后运行 `bun run gen:i18n` 重新生成 `i18n-keys.d.ts`（否则 typecheck / `bun run check:i18n` 失败）。
- 不新增颜色 hex 与 @icon-park 图标（复用既有 token 与 `TurnProcessReceipt` 图标），保证 `check:theme`/`check:icons` 通过。
- 结构测试钉死源码字符串。以下字符串**不得改动**：MessageList.tsx 的 `activeTurnId: conversationContext?.activeTurnId`、`tailClosed: conversationContext?.isProcessing !== true`、`running: entry.running`、`processItemStates: entry.processItemStates`、`getDisclosureProcessItemState`；TurnProcessDisclosure.tsx 的 `failed: 'messages.turnProcessed'`、`if (!item.running) return;`、`const durationEndAt = item.running ? now : item.endAt;`、`shouldResetTurnProcessDisclosureExpansion` 及 header/actions/body 的 className 顺序。
- 新建源文件带 Apache-2.0 头（格式与邻近文件一致）：
  ```
  /**
   * @license
   * Copyright 2025-2026 NomiFun (nomifun.com)
   * SPDX-License-Identifier: Apache-2.0
   */
  ```
- 测试从仓库根目录跑：`bun test <文件或目录路径>`（根 `bun run test` 是 cargo，别用）。
- 提交信息用仓库惯例（`feat(ui): …`/`fix(ui): …`/`test: …`），结尾加 `Co-Authored-By: Claude Fable 5 <noreply@anthropic.com>`。

---

### Task 1: 回合头部统一「已处理 {耗时}」+ 停止文案

**Files:**
- Modify: `ui/src/renderer/pages/conversation/Messages/components/TurnProcessDisclosure.tsx:43-57`
- Modify: `ui/src/renderer/services/i18n/locales/zh-CN/messages.json:55-58`
- Modify: `ui/src/renderer/services/i18n/locales/en-US/messages.json:55-58`
- Test: `ui/src/renderer/pages/conversation/Messages/turnProcessLayout.structure.test.ts`

**Interfaces:**
- Consumes: 现有 `labelKeyByState`/`defaultLabelByState`（TurnProcessDisclosure.tsx 43-57）。
- Produces: locale 键 `messages.turnProcessing` 被删除；`messages.turnCanceled` 新文案（Task 2 的停止态直接复用）。

- [ ] **Step 1: 在 turnProcessLayout.structure.test.ts 新增失败测试**

在 `describe('turn process disclosure content layout', …)` 内追加（放在 `'never exposes a failed or success outcome in the turn header'` 测试之后）：

```ts
  test('shows processed instead of processing while the turn is running', () => {
    expect(disclosureSource.includes("running: 'messages.turnProcessed'")).toBe(true);
    expect(disclosureSource.includes('messages.turnProcessing')).toBe(false);
    expect(zhMessages.turnProcessing).toBeUndefined();
    expect(enMessages.turnProcessing).toBeUndefined();
  });

  test('labels a stopped turn with the stop moment copy', () => {
    expect(zhMessages.turnCanceled).toBe('你在 {{duration}} 后停止了');
    expect(enMessages.turnCanceled).toBe('You stopped after {{duration}}');
  });
```

- [ ] **Step 2: 运行确认失败**

Run: `bun test ui/src/renderer/pages/conversation/Messages/turnProcessLayout.structure.test.ts`
Expected: 上述两个新测试 FAIL（其余通过）。

- [ ] **Step 3: 改 TurnProcessDisclosure 映射**

`TurnProcessDisclosure.tsx` 43-57 行改为：

```ts
const labelKeyByState: Record<TurnDisclosureProcessState, string> = {
  completed: 'messages.turnProcessed',
  running: 'messages.turnProcessed',
  waiting: 'messages.turnWaiting',
  failed: 'messages.turnProcessed',
  canceled: 'messages.turnCanceled',
};

const defaultLabelByState: Record<TurnDisclosureProcessState, string> = {
  completed: 'Processed {{duration}}',
  running: 'Processed {{duration}}',
  waiting: 'Waiting for confirmation {{duration}}',
  failed: 'Processed {{duration}}',
  canceled: 'You stopped after {{duration}}',
};
```

（`waiting` 保留「等待确认」是规格决策：它是需要用户行动的状态，规格只禁止「处理中」。）

- [ ] **Step 4: 改两个 locale**

`zh-CN/messages.json` 55-58 行（删除 `turnProcessing`）：

```json
  "turnProcessed": "已处理 {{duration}}",
  "turnWaiting": "等待确认 {{duration}}",
  "turnCanceled": "你在 {{duration}} 后停止了",
```

`en-US/messages.json` 55-58 行同样删除 `turnProcessing`：

```json
  "turnProcessed": "Processed {{duration}}",
  "turnWaiting": "Waiting for confirmation {{duration}}",
  "turnCanceled": "You stopped after {{duration}}",
```

- [ ] **Step 5: 重新生成 i18n 类型并确认没有残留引用**

Run: `bun run gen:i18n`
Run: `grep -rn "turnProcessing" ui/src --include="*.ts" --include="*.tsx" --include="*.json"`
Expected: 无结果（i18n-keys.d.ts 已再生成，代码无引用）。

- [ ] **Step 6: 跑测试确认通过**

Run: `bun test ui/src/renderer/pages/conversation/Messages/turnProcessLayout.structure.test.ts`
Expected: 全部 PASS。

- [ ] **Step 7: Commit**

```bash
git add ui/src/renderer/pages/conversation/Messages/components/TurnProcessDisclosure.tsx ui/src/renderer/services/i18n/locales/zh-CN/messages.json ui/src/renderer/services/i18n/locales/en-US/messages.json ui/src/renderer/services/i18n/i18n-keys.d.ts ui/src/renderer/pages/conversation/Messages/turnProcessLayout.structure.test.ts
git commit -m "feat(ui): unify turn header to processed copy and stop-moment cancel copy"
```

---

### Task 2: 用户停止 → stopNotice → 尾部回合置为停止态

**Files:**
- Modify: `ui/src/renderer/pages/conversation/Messages/turnDisclosureModel.ts`（`BuildTurnDisclosureOptions`、`buildTurnDisclosureItems` 末尾）
- Modify: `ui/src/renderer/pages/conversation/platforms/nomi/useNomiMessage.ts`（新增 stopNotice 状态；`resetState`/`restoreRunningAfterStopFailure`/`setWaitingResponse`/`acceptStart`/会话切换 effect/return）
- Modify: `ui/src/renderer/hooks/context/ConversationContext.tsx`（`ConversationContextValue` 新字段）
- Modify: `ui/src/renderer/pages/conversation/platforms/nomi/NomiChat.tsx:87-108`（context 透传）
- Modify: `ui/src/renderer/pages/conversation/Messages/MessageList.tsx:958-961`（options 透传）
- Test: `ui/src/renderer/pages/conversation/Messages/turnDisclosureModel.test.ts`

**Interfaces:**
- Consumes: `buildTurnDisclosureItems(items, options)`、`TurnDisclosureOutputItem`（turn_disclosure 分支含 `state/startAt/endAt/running/defaultCollapsed`）。
- Produces: `BuildTurnDisclosureOptions.stopNotice?: { stoppedAt: number }`；`ConversationContextValue.stopNotice?: { stoppedAt: number } | null`；`NomiMessageRuntime.stopNotice`。

- [ ] **Step 1: 在 turnDisclosureModel.test.ts 写失败测试**

该文件已有工厂 `item(id, role, options)` 与 `TURN_1` 常量，沿用即可。追加：

```ts
describe('stop notice', () => {
  test('marks the closed tail turn canceled and pins endAt to the stop moment', () => {
    const items = assignTurnIdsFromUserRequests([
      item('u1', 'user', { turnId: TURN_1, createdAt: 1_000 }),
      item('p1', 'process', { createdAt: 2_000 }),
    ]);
    const output = buildTurnDisclosureItems(items, { tailClosed: true, stopNotice: { stoppedAt: 5_000 } });
    const disclosure = output.find(
      (entry): entry is Extract<TurnDisclosureOutputItem, { type: 'turn_disclosure' }> =>
        entry.type === 'turn_disclosure'
    );
    expect(disclosure?.state).toBe('canceled');
    expect(disclosure?.endAt).toBe(5_000);
    expect(disclosure?.running).toBe(false);
    expect(disclosure?.defaultCollapsed).toBe(true);
  });

  test('ignores a stop notice that predates the tail turn', () => {
    const items = assignTurnIdsFromUserRequests([
      item('u1', 'user', { turnId: TURN_1, createdAt: 1_000 }),
      item('p1', 'process', { createdAt: 2_000 }),
    ]);
    const output = buildTurnDisclosureItems(items, { tailClosed: true, stopNotice: { stoppedAt: 500 } });
    const disclosure = output.find(
      (entry): entry is Extract<TurnDisclosureOutputItem, { type: 'turn_disclosure' }> =>
        entry.type === 'turn_disclosure'
    );
    expect(disclosure?.state).toBe('completed');
  });

  test('does not cancel a still-running tail turn', () => {
    const items = assignTurnIdsFromUserRequests([
      item('u1', 'user', { turnId: TURN_1, createdAt: 1_000 }),
      item('p1', 'process', { createdAt: 2_000, running: true }),
    ]);
    const output = buildTurnDisclosureItems(items, { tailClosed: false, stopNotice: { stoppedAt: 5_000 } });
    const disclosure = output.find(
      (entry): entry is Extract<TurnDisclosureOutputItem, { type: 'turn_disclosure' }> =>
        entry.type === 'turn_disclosure'
    );
    expect(disclosure?.state).toBe('running');
  });
});
```

注意：若 `TurnDisclosureOutputItem` 未在测试文件导入，需在文件顶部 import 中补上 `type TurnDisclosureOutputItem`。若工厂 `item()` 的 options 不含 `running`，查看其定义并按已有形状传（`processState: 'running'` 亦可）。

- [ ] **Step 2: 运行确认失败**

Run: `bun test ui/src/renderer/pages/conversation/Messages/turnDisclosureModel.test.ts`
Expected: 新 describe FAIL（TS 报 `stopNotice` 不在 options 类型上，或行为断言失败）。

- [ ] **Step 3: 模型实现**

`turnDisclosureModel.ts`：`BuildTurnDisclosureOptions` 增加字段：

```ts
export interface BuildTurnDisclosureOptions {
  tailClosed?: boolean;
  activeTurnId?: MessageId;
  /**
   * Present when the user stopped the latest turn in this session. The tail
   * disclosure (once closed) renders as canceled with `endAt` pinned to the
   * stop moment, so the header reads "you stopped after {duration}" even when
   * the backend never emitted a Canceled tool status.
   */
  stopNotice?: { stoppedAt: number };
}
```

在 `coalesceTurnDisclosures` 定义之后新增：

```ts
const applyStopNotice = (
  items: TurnDisclosureOutputItem[],
  stopNotice?: { stoppedAt: number }
): TurnDisclosureOutputItem[] => {
  if (!stopNotice) return items;
  const lastIndex = items.findLastIndex((entry) => entry.type === 'turn_disclosure');
  if (lastIndex === -1) return items;
  const disclosure = items[lastIndex];
  if (disclosure.type !== 'turn_disclosure') return items;
  // A live turn must keep ticking: the optimistic stop path flips
  // isProcessing first, so a running disclosure here means the notice is
  // stale (e.g. stop failed and the turn resumed).
  if (disclosure.running) return items;
  // A notice from an earlier turn must not cancel a newer one.
  if (stopNotice.stoppedAt < disclosure.startAt) return items;
  const next = items.slice();
  next[lastIndex] = {
    ...disclosure,
    state: 'canceled',
    endAt: stopNotice.stoppedAt,
    running: false,
    defaultCollapsed: true,
  };
  return next;
};
```

`buildTurnDisclosureItems` 最后一行 `return coalesceTurnDisclosures(output, processObservedAtByItemId);` 改为：

```ts
  return applyStopNotice(coalesceTurnDisclosures(output, processObservedAtByItemId), options.stopNotice);
```

- [ ] **Step 4: 跑模型测试**

Run: `bun test ui/src/renderer/pages/conversation/Messages/turnDisclosureModel.test.ts`
Expected: 全部 PASS。

- [ ] **Step 5: useNomiMessage 记录 stopNotice**

`ui/src/renderer/pages/conversation/platforms/nomi/useNomiMessage.ts`：

1. 在 `const [turnState, dispatchTurn] = useReducer(...)`（107 行）附近加：
   ```ts
   const [stopNotice, setStopNotice] = useState<{ stoppedAt: number } | null>(null);
   ```
2. `resetState`（722 行，唯一调用方是 NomiSendBox 停止路径，已 grep 验证）在 `dispatchTurn({ type: 'reset' });` 前加：
   ```ts
   setStopNotice({ stoppedAt: Date.now() });
   ```
3. `restoreRunningAfterStopFailure`（763 行）函数体开头加：
   ```ts
   setStopNotice(null);
   ```
4. `setWaitingResponse`（744 行）`if (value) {` 分支内加（用户提交新消息时清除）：
   ```ts
   setStopNotice(null);
   ```
5. turnStarted effect 的 `acceptStart`（558-567 行）内加：
   ```ts
   setStopNotice(null);
   ```
6. 会话切换 effect（651 行起）中 `setThought({ subject: '', description: '' });` 之后加：
   ```ts
   setStopNotice(null);
   ```
7. return 对象（797-813 行）加一行 `stopNotice,`。

- [ ] **Step 6: ConversationContext + NomiChat + MessageList 透传**

`ConversationContext.tsx` 在 `activeRequestMessageId` 字段后加：

```ts
  /**
   * Set when the user stopped the latest turn in this session. Message
   * rendering pins the tail disclosure to the stop moment ("you stopped
   * after {duration}"). Session-local; cleared when a new turn starts.
   */
  stopNotice?: { stoppedAt: number } | null;
```

`NomiChat.tsx` `conversationValue` useMemo（87-108 行）：对象里加 `stopNotice: turnActivity.stopNotice,`，依赖数组加 `turnActivity.stopNotice`。

`MessageList.tsx` 958-961 行：

```ts
    const disclosureItems = buildTurnDisclosureItems(modelInput, {
      tailClosed: conversationContext?.isProcessing !== true,
      activeTurnId: conversationContext?.activeTurnId,
      stopNotice: conversationContext?.stopNotice ?? undefined,
    })
```

并在 displayList useMemo 的依赖数组（1095-1102 行）加 `conversationContext?.stopNotice`。
（注意保持既有两行字符串逐字不变，只新增一行。）

- [ ] **Step 7: 结构测试保护 + 全量相关测试**

在 `turnProcessLayout.structure.test.ts` 追加：

```ts
  test('routes the user stop notice from the nomi runtime into the disclosure model', () => {
    expect(messageListSource.includes('stopNotice: conversationContext?.stopNotice ?? undefined')).toBe(true);
  });
```

Run: `bun test ui/src/renderer/pages/conversation/Messages ui/src/renderer/pages/conversation/platforms/nomi`
Expected: 全部 PASS（特别是 NomiChat.turnActivity.structure.test.ts 与 stopInteraction 相关测试不受影响）。

- [ ] **Step 8: Commit**

```bash
git add ui/src/renderer/pages/conversation/Messages/turnDisclosureModel.ts ui/src/renderer/pages/conversation/Messages/turnDisclosureModel.test.ts ui/src/renderer/pages/conversation/platforms/nomi/useNomiMessage.ts ui/src/renderer/hooks/context/ConversationContext.tsx ui/src/renderer/pages/conversation/platforms/nomi/NomiChat.tsx ui/src/renderer/pages/conversation/Messages/MessageList.tsx ui/src/renderer/pages/conversation/Messages/turnProcessLayout.structure.test.ts
git commit -m "feat(ui): pin stopped turns to the stop moment via a session stop notice"
```

---

### Task 3: 底部实时「处理步骤」区

**Files:**
- Create: `ui/src/renderer/pages/conversation/Messages/turnLiveStepModel.ts`
- Create: `ui/src/renderer/pages/conversation/Messages/turnLiveStepModel.test.ts`
- Modify: `ui/src/renderer/pages/conversation/Messages/MessageList.tsx`（VO 类型、displayList、renderItem）
- Modify: `ui/src/renderer/pages/conversation/Messages/messages.css`（`.turn-live-step` 样式，插在 `.turn-process-receipt__body` 块之后 ~656 行）
- Modify: `ui/src/renderer/services/i18n/locales/zh-CN/messages.json` 与 `en-US/messages.json`（`turnLiveStep` 键）
- Test: `ui/src/renderer/pages/conversation/Messages/turnLiveStep.structure.test.ts`（新建）

**Interfaces:**
- Consumes: `TurnDisclosureProcessState`（turnDisclosureModel）、`getProcessItemState`（turnProcessState）、MessageList 私有 `buildProcessReceiptSummary(item, state, t, workspaceRoots)`、`TurnProcessReceipt`（`receipt={{id,item,label,state,icon,defaultExpanded,hasDetail}}`，`hasDetail:false` 时为静态行，running 态自带 Spin）。
- Produces: `planTurnLiveStep(input: TurnLiveStepInput): TurnLiveStepPlan | null`；locale 键 `messages.turnLiveStep.analyzing` / `messages.turnLiveStep.composing`；DOM `data-testid='turn-live-step'`。

- [ ] **Step 1: 写模型失败测试 turnLiveStepModel.test.ts**

```ts
/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { describe, expect, test } from 'bun:test';
import { planTurnLiveStep } from './turnLiveStepModel';

const disclosure = (
  running: boolean,
  processItems: Array<{ id: string; state: 'completed' | 'running' | 'waiting' | 'failed' | 'canceled' }>
) => ({ running, processItems });

describe('planTurnLiveStep', () => {
  test('hidden while the conversation is not processing', () => {
    expect(
      planTurnLiveStep({
        isProcessing: false,
        disclosure: disclosure(true, [{ id: 'a', state: 'running' }]),
        hasStreamingReplyText: false,
      })
    ).toBeNull();
  });

  test('hidden when the tail turn has settled', () => {
    expect(
      planTurnLiveStep({
        isProcessing: true,
        disclosure: disclosure(false, [{ id: 'a', state: 'completed' }]),
        hasStreamingReplyText: false,
      })
    ).toBeNull();
  });

  test('waiting item wins over running item', () => {
    expect(
      planTurnLiveStep({
        isProcessing: true,
        disclosure: disclosure(true, [
          { id: 'a', state: 'running' },
          { id: 'b', state: 'waiting' },
        ]),
        hasStreamingReplyText: false,
      })
    ).toEqual({ kind: 'item', itemId: 'b', state: 'waiting' });
  });

  test('latest running item is the current step', () => {
    expect(
      planTurnLiveStep({
        isProcessing: true,
        disclosure: disclosure(true, [
          { id: 'a', state: 'completed' },
          { id: 'b', state: 'running' },
        ]),
        hasStreamingReplyText: false,
      })
    ).toEqual({ kind: 'item', itemId: 'b', state: 'running' });
  });

  test('streaming reply text without running items composes the reply', () => {
    expect(
      planTurnLiveStep({
        isProcessing: true,
        disclosure: disclosure(true, [{ id: 'a', state: 'completed' }]),
        hasStreamingReplyText: true,
      })
    ).toEqual({ kind: 'composing', state: 'running' });
  });

  test('fresh turn with no process rows analyzes the request', () => {
    expect(
      planTurnLiveStep({ isProcessing: true, disclosure: disclosure(true, []), hasStreamingReplyText: false })
    ).toEqual({ kind: 'analyzing', state: 'running' });
  });

  test('gap between steps prepares the next action', () => {
    expect(
      planTurnLiveStep({
        isProcessing: true,
        disclosure: disclosure(true, [{ id: 'a', state: 'completed' }]),
        hasStreamingReplyText: false,
      })
    ).toEqual({ kind: 'preparing', state: 'running' });
  });

  test('hidden without a tail disclosure', () => {
    expect(planTurnLiveStep({ isProcessing: true, hasStreamingReplyText: false })).toBeNull();
  });
});
```

- [ ] **Step 2: 运行确认失败**

Run: `bun test ui/src/renderer/pages/conversation/Messages/turnLiveStepModel.test.ts`
Expected: FAIL（模块不存在）。

- [ ] **Step 3: 实现 turnLiveStepModel.ts**

```ts
/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import type { TurnDisclosureProcessState } from './turnDisclosureModel';

/**
 * The live-step strip under the latest AI reply. It is the primary "still
 * working" signal now that the turn header reads "processed" throughout the
 * turn lifecycle; it disappears as soon as the turn settles.
 */
export type TurnLiveStepPlan =
  | { kind: 'item'; itemId: string; state: 'running' | 'waiting' }
  | { kind: 'composing'; state: 'running' }
  | { kind: 'analyzing'; state: 'running' }
  | { kind: 'preparing'; state: 'running' };

export interface TurnLiveStepInput {
  isProcessing: boolean;
  /** Tail turn disclosure with effective per-item states, when one exists. */
  disclosure?: {
    running: boolean;
    processItems: Array<{ id: string; state: TurnDisclosureProcessState }>;
  };
  /** True when the final assistant reply text is still streaming in. */
  hasStreamingReplyText: boolean;
}

export function planTurnLiveStep(input: TurnLiveStepInput): TurnLiveStepPlan | null {
  if (!input.isProcessing) return null;
  const disclosure = input.disclosure;
  if (!disclosure || !disclosure.running) return null;

  const waitingItem = disclosure.processItems.findLast((entry) => entry.state === 'waiting');
  if (waitingItem) return { kind: 'item', itemId: waitingItem.id, state: 'waiting' };

  const runningItem = disclosure.processItems.findLast((entry) => entry.state === 'running');
  if (runningItem) return { kind: 'item', itemId: runningItem.id, state: 'running' };

  if (input.hasStreamingReplyText) return { kind: 'composing', state: 'running' };
  if (disclosure.processItems.length === 0) return { kind: 'analyzing', state: 'running' };
  return { kind: 'preparing', state: 'running' };
}
```

- [ ] **Step 4: 模型测试通过**

Run: `bun test ui/src/renderer/pages/conversation/Messages/turnLiveStepModel.test.ts`
Expected: PASS。

- [ ] **Step 5: locale 新键 + 再生成类型**

`zh-CN/messages.json` 在 `"turnProcess": { … }` 块后加：

```json
  "turnLiveStep": {
    "analyzing": "正在分析需求",
    "composing": "正在整理回复"
  },
```

`en-US/messages.json` 同位置：

```json
  "turnLiveStep": {
    "analyzing": "Analyzing the request",
    "composing": "Composing the reply"
  },
```

Run: `bun run gen:i18n`

- [ ] **Step 6: MessageList 集成**

1. `ITurnActionsVO` 类型定义后（147 行附近）加：

```ts
type ITurnLiveStepVO = {
  type: 'turn_live_step';
  id: string;
  msg_id: MessageId;
  label: string;
  state: 'running' | 'waiting';
  icon: TurnProcessReceiptIcon;
  sourceMessageIds: SourceMessageId[];
  created_at: number;
};
```

2. `IProcessedItem` 联合类型追加 `| ITurnLiveStepVO`。
3. `getProcessedItemSourceMessageIds` 第一处类型列表（163-167 行）加 `item.type === 'turn_live_step' ||`。
4. `getProcessedItemCreatedAt` 的类型数组（199-207 行）加 `'turn_live_step',`。
5. displayList useMemo 中（deliverables 段之前、`const turnGates` 之前）加辅助函数：

```ts
    const isStreamingReplyText = (entry: IProcessedItem | undefined): boolean =>
      !!entry && 'type' in entry && entry.type === 'text' && (entry as IMessageText).position === 'left';

    const buildTurnLiveStep = (items: IProcessedItem[]): ITurnLiveStepVO | undefined => {
      if (conversationContext?.isProcessing !== true) return undefined;
      const tailDisclosure = items.findLast(
        (entry): entry is ITurnProcessDisclosureVO => 'type' in entry && entry.type === 'turn_process_disclosure'
      );
      if (!tailDisclosure) return undefined;
      const plan = planTurnLiveStep({
        isProcessing: true,
        disclosure: {
          running: tailDisclosure.running,
          processItems: tailDisclosure.processItems.map((processItem) => {
            const anchorId = getProcessedItemAnchorId(processItem);
            return {
              id: anchorId,
              state: tailDisclosure.processItemStates[anchorId] ?? getProcessItemState(processItem),
            };
          }),
        },
        hasStreamingReplyText: isStreamingReplyText(items.at(-1)),
      });
      if (!plan) return undefined;

      let label: string;
      let icon: TurnProcessReceiptIcon;
      if (plan.kind === 'item') {
        const processItem = tailDisclosure.processItems.find(
          (candidate) => getProcessedItemAnchorId(candidate) === plan.itemId
        );
        if (processItem && 'type' in processItem && processItem.type === 'thinking') {
          label = t('messages.processReceipt.thinkingRunning', { defaultValue: 'Thinking' });
          icon = 'thinking';
        } else if (processItem) {
          const summary = buildProcessReceiptSummary(processItem, plan.state, t, workspaceRoots);
          label = summary.label;
          icon = summary.icon;
        } else {
          label = t('messages.processReceipt.preparingAction', { defaultValue: 'Preparing next action' });
          icon = 'status';
        }
      } else if (plan.kind === 'composing') {
        label = t('messages.turnLiveStep.composing', { defaultValue: 'Composing the reply' });
        icon = 'status';
      } else if (plan.kind === 'analyzing') {
        label = t('messages.turnLiveStep.analyzing', { defaultValue: 'Analyzing the request' });
        icon = 'thinking';
      } else {
        label = t('messages.processReceipt.preparingAction', { defaultValue: 'Preparing next action' });
        icon = 'status';
      }

      return {
        type: 'turn_live_step',
        id: `turn-live-step-${tailDisclosure.msg_id}`,
        msg_id: tailDisclosure.msg_id,
        label,
        state: plan.state,
        icon,
        sourceMessageIds: [],
        created_at: tailDisclosure.endAt,
      };
    };
```

6. 两个返回路径都追加 live step（保持既有语句不动，只包装返回值）：
   - `if (deliverablesByTurn.size === 0) return disclosureItems;`（1037 行）改为：
     ```ts
     const liveStepForDisclosures = buildTurnLiveStep(disclosureItems);
     if (deliverablesByTurn.size === 0) {
       return liveStepForDisclosures ? [...disclosureItems, liveStepForDisclosures] : disclosureItems;
     }
     ```
   - 末尾 `return withDeliverables;`（1094 行）改为：
     ```ts
     const liveStep = buildTurnLiveStep(withDeliverables);
     return liveStep ? [...withDeliverables, liveStep] : withDeliverables;
     ```
7. `planTurnLiveStep` 导入：`import { planTurnLiveStep } from './turnLiveStepModel';`
8. `renderItem` 中 `turn_actions` 分支后加：

```tsx
    if ('type' in item && item.type === 'turn_live_step') {
      return (
        <div
          key={item.id}
          id={`message-${getProcessedItemAnchorId(item)}`}
          data-testid='turn-live-step'
          className='min-w-0 message-item px-8px m-t-10px max-w-full md:max-w-780px mx-auto turn_live_step'
        >
          <div className='turn-live-step'>
            <TurnProcessReceipt
              receipt={{
                id: item.id,
                item,
                label: item.label,
                state: item.state,
                icon: item.icon,
                defaultExpanded: false,
                hasDetail: false,
              }}
              renderProcessItem={() => null}
            />
          </div>
        </div>
      );
    }
```

9. `lastUserTextIndex`/`isActiveProcessTextItem`（1104-1122 行）的类型排除数组 `['turn_process_disclosure', 'process_receipt', 'artifact']` 两处各加 `'turn_live_step'`。

- [ ] **Step 7: messages.css 样式（`.turn-process-receipt__body` 块之后）**

```css
/* ── Live current-step strip under the latest streaming reply ── */

.turn-live-step .turn-process-receipt__label {
  animation: turn-live-step-breathing 1.8s ease-in-out infinite;
}

.turn-live-step .turn-process-receipt--waiting {
  color: var(--color-warning-6, #ff7d00);
}

.turn-live-step .turn-process-receipt--waiting .turn-process-receipt__label {
  animation: none;
}

@keyframes turn-live-step-breathing {
  0%,
  100% {
    opacity: 1;
  }
  50% {
    opacity: 0.55;
  }
}

@media (prefers-reduced-motion: reduce) {
  .turn-live-step .turn-process-receipt__label {
    animation: none;
  }
}
```

（标签省略号复用 `.turn-process-receipt__label` 既有规则；无新颜色/图标。）

- [ ] **Step 8: 新建 turnLiveStep.structure.test.ts**

```ts
/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { readFileSync } from 'node:fs';
import { describe, expect, test } from 'bun:test';

const messageListSource = readFileSync(new URL('./MessageList.tsx', import.meta.url), 'utf8');
const cssSource = readFileSync(new URL('./messages.css', import.meta.url), 'utf8');
const zhMessages = JSON.parse(
  readFileSync(new URL('../../../services/i18n/locales/zh-CN/messages.json', import.meta.url), 'utf8')
) as Record<string, Record<string, string> | string>;
const enMessages = JSON.parse(
  readFileSync(new URL('../../../services/i18n/locales/en-US/messages.json', import.meta.url), 'utf8')
) as Record<string, Record<string, string> | string>;

describe('turn live step strip', () => {
  test('appends the live step to the display list on both return paths', () => {
    expect(messageListSource.includes("import { planTurnLiveStep } from './turnLiveStepModel'")).toBe(true);
    expect(messageListSource.includes('const liveStepForDisclosures = buildTurnLiveStep(disclosureItems)')).toBe(true);
    expect(messageListSource.includes('const liveStep = buildTurnLiveStep(withDeliverables)')).toBe(true);
    expect(messageListSource.includes("data-testid='turn-live-step'")).toBe(true);
  });

  test('renders through the existing receipt row without detail expansion', () => {
    expect(messageListSource.includes("type: 'turn_live_step'")).toBe(true);
    expect(messageListSource.includes('hasDetail: false')).toBe(true);
  });

  test('breathes gently and respects reduced motion', () => {
    expect(cssSource.includes('@keyframes turn-live-step-breathing')).toBe(true);
    expect(cssSource.includes('.turn-live-step .turn-process-receipt__label')).toBe(true);
    const reducedMotionIndex = cssSource.indexOf('@media (prefers-reduced-motion: reduce)');
    expect(reducedMotionIndex).toBeGreaterThan(-1);
    expect(cssSource.slice(reducedMotionIndex).includes('.turn-live-step .turn-process-receipt__label')).toBe(true);
  });

  test('ships bilingual live-step copy', () => {
    expect((zhMessages.turnLiveStep as Record<string, string>).analyzing).toBe('正在分析需求');
    expect((zhMessages.turnLiveStep as Record<string, string>).composing).toBe('正在整理回复');
    expect((enMessages.turnLiveStep as Record<string, string>).analyzing).toBeTruthy();
    expect((enMessages.turnLiveStep as Record<string, string>).composing).toBeTruthy();
  });
});
```

注意：messages.css 已有多个 `@media (prefers-reduced-motion: reduce)` 块（853 行附近）。`reducedMotionIndex` 用 `cssSource.lastIndexOf(...)` 或直接把新的 reduced-motion 规则并入文件末尾已有块并调整断言——实现时以实际文件为准，断言"reduce 块内包含 .turn-live-step"成立即可。

- [ ] **Step 9: 跑测试**

Run: `bun test ui/src/renderer/pages/conversation/Messages`
Expected: 全部 PASS（含既有 turnProcessLayout / MessageList.turnDisclosure 结构测试）。

- [ ] **Step 10: Commit**

```bash
git add ui/src/renderer/pages/conversation/Messages/turnLiveStepModel.ts ui/src/renderer/pages/conversation/Messages/turnLiveStepModel.test.ts ui/src/renderer/pages/conversation/Messages/turnLiveStep.structure.test.ts ui/src/renderer/pages/conversation/Messages/MessageList.tsx ui/src/renderer/pages/conversation/Messages/messages.css ui/src/renderer/services/i18n/locales/zh-CN/messages.json ui/src/renderer/services/i18n/locales/en-US/messages.json ui/src/renderer/services/i18n/i18n-keys.d.ts
git commit -m "feat(ui): add live current-step strip under the latest streaming reply"
```

---

### Task 4: 失败卡片重试入口 + 补 conversation.stop.failed

**Files:**
- Modify: `ui/src/renderer/pages/conversation/Messages/components/MessageTips.tsx`
- Modify: `ui/src/renderer/pages/conversation/Messages/messages.css`（`.message-error-note__retry`，加在 error-note 样式块内）
- Modify: `ui/src/renderer/services/i18n/locales/zh-CN/conversation.json` 与 `en-US/conversation.json`（`stop.failed`）
- Test: `ui/src/renderer/pages/conversation/Messages/components/MessageTips.retry.structure.test.ts`（新建）

**Interfaces:**
- Consumes: `useMessageList`（`../hooks`，返回 `TMessage[]`）、`useConversationContextSafe`、`emitter.emit('sendbox.edit', { msgId, createdAt, content })`（共享 SendBox 已监听：回填输入框进入编辑模式，提交即截断重跑，仅 Nomi 提供 `onEditResubmit`）、`parseMessageFileMarker`（`./messageFileMarker`）、`common.retry`（两 locale 均为 Retry/重试，已存在）。
- Produces: DOM `data-testid='message-error-retry'`；locale 键 `conversation.stop.failed`。

- [ ] **Step 1: 新建失败的结构测试**

`MessageTips.retry.structure.test.ts`：

```ts
/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { readFileSync } from 'node:fs';
import { describe, expect, test } from 'bun:test';

const tipsSource = readFileSync(new URL('./MessageTips.tsx', import.meta.url), 'utf8');
const zhConversation = JSON.parse(
  readFileSync(new URL('../../../../services/i18n/locales/zh-CN/conversation.json', import.meta.url), 'utf8')
) as { stop?: { failed?: string } };
const enConversation = JSON.parse(
  readFileSync(new URL('../../../../services/i18n/locales/en-US/conversation.json', import.meta.url), 'utf8')
) as { stop?: { failed?: string } };

describe('message error retry entry', () => {
  test('offers a retry entry that recalls the failed request into the composer', () => {
    expect(tipsSource.includes("data-testid='message-error-retry'")).toBe(true);
    expect(tipsSource.includes("emitter.emit('sendbox.edit'")).toBe(true);
    expect(tipsSource.includes("conversationContext?.type !== 'nomi'")).toBe(true);
    expect(tipsSource.includes('common.retry')).toBe(true);
  });

  test('retry hides while the conversation is still processing or read-only', () => {
    expect(tipsSource.includes('conversationContext.isProcessing === true')).toBe(true);
    expect(tipsSource.includes('conversationContext.readOnly === true')).toBe(true);
  });

  test('stop failure toast copy exists in both locales', () => {
    expect(zhConversation.stop?.failed).toBeTruthy();
    expect(enConversation.stop?.failed).toBeTruthy();
  });
});
```

- [ ] **Step 2: 运行确认失败**

Run: `bun test ui/src/renderer/pages/conversation/Messages/components/MessageTips.retry.structure.test.ts`
Expected: 3 个测试 FAIL。

- [ ] **Step 3: MessageTips 实现重试入口**

在 `MessageTips.tsx` 顶部 import 增加：

```ts
import { emitter } from '@/renderer/utils/emitter';
import { useConversationContextSafe } from '@/renderer/hooks/context/ConversationContext';
import { useMessageList } from '../hooks';
import { parseMessageFileMarker } from './messageFileMarker';
```

在 `useFormatContent` 之后加 hook（组件外层同文件）：

```ts
/**
 * Retry entry for a failed turn: recalls the originating user request into
 * the composer via the shared `sendbox.edit` channel (edit mode: submitting
 * truncates and reruns). Only offered on the nomi surface, for errors that
 * answer the latest user request, once the turn has settled.
 */
const useErrorRetry = (message: IMessageTips): (() => void) | null => {
  const conversationContext = useConversationContextSafe();
  const messageList = useMessageList();
  return useMemo(() => {
    if (message.content.type !== 'error') return null;
    if (conversationContext?.type !== 'nomi') return null;
    if (conversationContext.readOnly === true) return null;
    if (conversationContext.isProcessing === true) return null;
    const lastRight = messageList.findLast((entry) => entry.type === 'text' && entry.position === 'right');
    if (!lastRight || lastRight.type !== 'text') return null;
    const retryMessageId = lastRight.message_id ?? lastRight.msg_id;
    const retryCreatedAt = lastRight.created_at;
    if (!retryMessageId || retryCreatedAt == null) return null;
    if ((message.created_at ?? 0) < retryCreatedAt) return null;
    const rawContent = typeof lastRight.content?.content === 'string' ? lastRight.content.content : '';
    const { text } = parseMessageFileMarker(rawContent, 'right');
    if (!text.trim()) return null;
    return () => emitter.emit('sendbox.edit', { msgId: retryMessageId, createdAt: retryCreatedAt, content: text });
  }, [conversationContext, message.content.type, message.created_at, messageList]);
};
```

组件内 `const { json, data } = useFormatContent(content);` 后加：

```ts
  const retry = useErrorRetry(message);
  const retryButton = retry ? (
    <button type='button' className='message-error-note__retry' data-testid='message-error-retry' onClick={retry}>
      {t('common.retry', { defaultValue: 'Retry' })}
    </button>
  ) : null;
```

结构化卡片分支：在 `message-error-note__actions` div 中 `<FeedbackButton …/>` 前插入 `{retryButton}`。若 `shouldShowFeedback` 为 false 时也要能显示 retry，把该 actions 容器的条件从 `shouldShowFeedback` 改为 `(shouldShowFeedback || retryButton)`（保持原缩进结构）。

json 分支与纯文本分支：把

```tsx
          {type === 'error' && (
            <div className='flex justify-end'>
              <FeedbackButton module='conversation-session' />
            </div>
          )}
```

改为（两处一致；纯文本分支的条件名是 `shouldShowFeedback`）：

```tsx
          {type === 'error' && (
            <div className='flex justify-end items-center gap-8px'>
              {retryButton}
              <FeedbackButton module='conversation-session' />
            </div>
          )}
```

注意 `IMessageTips` 的 `created_at` 字段可选性以实际类型为准（`message.created_at ?? 0` 已防御）。

- [ ] **Step 4: messages.css 加按钮样式**

找到 `.message-error-note__feedback` 或 error-note 样式块（约 110-350 行），邻近处加：

```css
.message-error-note__retry {
  border: 1px solid var(--color-border-2, #e5e6eb);
  border-radius: 6px;
  background: transparent;
  color: var(--color-text-2, #4e5969);
  cursor: pointer;
  font: inherit;
  font-size: 12px;
  line-height: 20px;
  padding: 0 10px;
}

.message-error-note__retry:hover {
  color: var(--color-text-1, #1d2129);
  border-color: var(--color-text-4, #c9cdd4);
}
```

- [ ] **Step 5: 补 conversation.stop.failed**

`zh-CN/conversation.json` 根级加（按字母序邻近位置，如 `"sessionMetrics"` 附近）：

```json
  "stop": {
    "failed": "停止当前任务失败，请重试。"
  },
```

`en-US/conversation.json` 同位置：

```json
  "stop": {
    "failed": "Failed to stop the current task. Please try again."
  },
```

Run: `bun run gen:i18n`

- [ ] **Step 6: 跑测试**

Run: `bun test ui/src/renderer/pages/conversation/Messages/components/MessageTips.retry.structure.test.ts ui/src/renderer/pages/conversation/Messages`
Expected: 全部 PASS。

- [ ] **Step 7: Commit**

```bash
git add ui/src/renderer/pages/conversation/Messages/components/MessageTips.tsx ui/src/renderer/pages/conversation/Messages/components/MessageTips.retry.structure.test.ts ui/src/renderer/pages/conversation/Messages/messages.css ui/src/renderer/services/i18n/locales/zh-CN/conversation.json ui/src/renderer/services/i18n/locales/en-US/conversation.json ui/src/renderer/services/i18n/i18n-keys.d.ts
git commit -m "feat(ui): add retry entry to turn failure card and stop-failure copy"
```

---

### Task 5: 全量验证门禁

**Files:**
- 无新改动（只跑验证；如有失败则修复后重跑）

- [ ] **Step 1: Messages 与 nomi 平台目录全量测试**

Run: `bun test ui/src/renderer/pages/conversation/Messages ui/src/renderer/pages/conversation/platforms/nomi`
Expected: 全部 PASS。

- [ ] **Step 2: 仓库级检查**

Run: `bun run check`
Expected: typecheck、check:i18n、check:theme、check:icons 等全部通过。

- [ ] **Step 3: 对照规格逐条走查**

- 头部 running/completed 均为「已处理 {耗时}」，running 有 1s ticker；
- 底部步骤区仅在尾部回合 live 时出现，waiting 显示警示色，完成/失败/停止后消失；
- 停止后头部「你在 {已耗时} 后停止了」，耗时=停止时刻；
- 失败卡片有「重试」按钮（nomi、非处理中、最新请求）。

- [ ] **Step 4: Commit（如走查产生修补）并汇报**

如实汇报：本机无 Mac 后端运行时，未做端到端会话驱动验证；门禁为单测+结构测试+类型/契约检查。
