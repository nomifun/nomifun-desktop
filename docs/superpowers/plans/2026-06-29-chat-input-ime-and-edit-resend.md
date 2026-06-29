# 会话输入框交互优化 实现计划（Enter 防误触 + 暂停后编辑重发）

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 修复输入法上屏 Enter 被误发送、新增发送键偏好；并为 Nomi 原生会话支持「编辑最近一条用户消息并截断重跑」。

**Architecture:** Part A（前端）集中在 `useCompositionInput` 的 IME 守卫与提交手势判定，配置项走现有 `configService`。Part B 后端新增 keyset 截断删除（repo）、引擎"回退最后一个 turn"原语、service `edit_and_resubmit` + 路由；前端给最近一条用户气泡加「编辑」入口，复用发送路径截断重跑。

**Tech Stack:** React + TypeScript（Arco Design、bun:test）、Rust（axum、sqlx 运行时查询、tokio、async-trait）。

## Global Constraints

- 注释/文案随上下文语言；交流用中文。提交以 `nomifun <rika00@qq.com>` 署名，**不**加 `Co-Authored-By` trailer：`git -c user.name=nomifun -c user.email=rika00@qq.com commit --author="nomifun <rika00@qq.com>" -m ...`。
- 前端测试运行器是 `bun:test`：`cd ui && bun test <path>`；导入 `import { describe, expect, test } from 'bun:test'`。
- Rust 测试：单测跑 `cargo test -p <crate> <filter>`。
- sqlx 用**运行时**查询（`sqlx::query`/`query_as::<_, T>`），占位符是匿名 `?`，按顺序 `.bind(...)`。
- 引擎 `AgentEngine` 的所有构造点都是 `Self { ... }` 字面量（无 `..Default`）：新增字段必须同时改 **3 处**——`new_with_provider`（engine.rs:230-265）、`resume_with_provider`（:305-340）、测试 `make_engine`（:1500-1535）。
- 编辑重发**仅 Nomi**、**仅最近一条**用户消息；语义为截断重跑。
- 分支：`feat/chat-input-ime-and-edit-resend`（已建，设计文档已提交）。

---

# Part A — Enter 发送（IME 防误触 + 发送键偏好）

## Task A1：useCompositionInput 健壮 IME 守卫 + 提交手势纯函数

**Files:**
- Modify: `ui/src/renderer/hooks/chat/useCompositionInput.ts`
- Test: `ui/src/renderer/hooks/chat/useCompositionInput.test.ts`（Create）

**Interfaces:**
- Produces:
  - `export type SendKeyMode = 'enter' | 'mod-enter'`
  - `export function isImeComposingKey(e, state): boolean`
  - `export function isSubmitGesture(e: { key: string; shiftKey?: boolean; metaKey?: boolean; ctrlKey?: boolean; altKey?: boolean }, mode: SendKeyMode): boolean`
  - hook 返回新增 `isImeActive: (e) => boolean`；`createKeyDownHandler(onEnterPress, intercept?, sendKey?: SendKeyMode)`

- [ ] **Step 1: 写失败测试** — `ui/src/renderer/hooks/chat/useCompositionInput.test.ts`

```ts
/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */
import { describe, expect, test } from 'bun:test';
import { isImeComposingKey, isSubmitGesture } from './useCompositionInput';

describe('isImeComposingKey', () => {
  const base = { key: 'Enter' as const };
  test('true when composing ref set', () => {
    expect(isImeComposingKey(base, { composing: true, justComposed: false })).toBe(true);
  });
  test('true right after compositionend (justComposed window)', () => {
    expect(isImeComposingKey(base, { composing: false, justComposed: true })).toBe(true);
  });
  test('true when native isComposing', () => {
    expect(isImeComposingKey({ ...base, nativeEvent: { isComposing: true } }, { composing: false, justComposed: false })).toBe(true);
  });
  test('true when keyCode 229', () => {
    expect(isImeComposingKey({ ...base, keyCode: 229 }, { composing: false, justComposed: false })).toBe(true);
  });
  test('false for a clean Enter', () => {
    expect(isImeComposingKey({ ...base, keyCode: 13, nativeEvent: { isComposing: false } }, { composing: false, justComposed: false })).toBe(false);
  });
});

describe('isSubmitGesture', () => {
  test('enter mode: plain Enter submits, Shift+Enter does not', () => {
    expect(isSubmitGesture({ key: 'Enter' }, 'enter')).toBe(true);
    expect(isSubmitGesture({ key: 'Enter', shiftKey: true }, 'enter')).toBe(false);
  });
  test('enter mode: Cmd+Enter still submits (legacy compatible)', () => {
    expect(isSubmitGesture({ key: 'Enter', metaKey: true }, 'enter')).toBe(true);
  });
  test('mod-enter mode: plain Enter does NOT submit', () => {
    expect(isSubmitGesture({ key: 'Enter' }, 'mod-enter')).toBe(false);
  });
  test('mod-enter mode: Cmd/Ctrl+Enter submits, Shift+Cmd+Enter does not', () => {
    expect(isSubmitGesture({ key: 'Enter', metaKey: true }, 'mod-enter')).toBe(true);
    expect(isSubmitGesture({ key: 'Enter', ctrlKey: true }, 'mod-enter')).toBe(true);
    expect(isSubmitGesture({ key: 'Enter', metaKey: true, shiftKey: true }, 'mod-enter')).toBe(false);
  });
  test('non-Enter never submits', () => {
    expect(isSubmitGesture({ key: 'a' }, 'enter')).toBe(false);
  });
});
```

- [ ] **Step 2: 跑测试确认失败**

Run: `cd ui && bun test src/renderer/hooks/chat/useCompositionInput.test.ts`
Expected: FAIL（`isImeComposingKey`/`isSubmitGesture` 未导出）

- [ ] **Step 3: 实现** — 用以下完整内容替换 `ui/src/renderer/hooks/chat/useCompositionInput.ts`

```ts
import { useCallback, useRef, useState } from 'react';

export type SendKeyMode = 'enter' | 'mod-enter';

type ComposingState = { composing: boolean; justComposed: boolean };
type ImeKeyLike = { key?: string; keyCode?: number; nativeEvent?: { isComposing?: boolean } };
type SubmitKeyLike = { key: string; shiftKey?: boolean; metaKey?: boolean; ctrlKey?: boolean; altKey?: boolean };

/**
 * 纯函数：判断这次 keydown 是否处于输入法合成中（绝不能触发发送）。
 * 组合多重信号以覆盖各浏览器/输入法的事件时序差异：
 * - composing：compositionstart→true / compositionend→false 的 ref
 * - justComposed：compositionend 后的一帧兜底窗口（覆盖"compositionend 先于 Enter keydown"）
 * - nativeEvent.isComposing：W3C 原生属性
 * - keyCode === 229：IME 处理中的 keydown
 */
export function isImeComposingKey(e: ImeKeyLike, state: ComposingState): boolean {
  return state.composing || state.justComposed || e.nativeEvent?.isComposing === true || e.keyCode === 229;
}

/**
 * 纯函数：在给定发送键偏好下，这次 keydown 是否为"提交"手势。
 * - 'enter'（默认，兼容旧行为）：Enter 且非 Shift 即提交（Cmd/Ctrl+Enter 也提交）
 * - 'mod-enter'：必须 Cmd/Ctrl+Enter；裸 Enter 不提交（留给 textarea 换行）
 */
export function isSubmitGesture(e: SubmitKeyLike, mode: SendKeyMode): boolean {
  if (e.key !== 'Enter' || e.shiftKey) return false;
  if (mode === 'mod-enter') return Boolean(e.metaKey || e.ctrlKey);
  return true;
}

/**
 * 共享的输入法合成事件处理 hook。消除 SendBox 与引导页中的 IME 处理重复代码。
 */
export const useCompositionInput = () => {
  const isComposing = useRef(false);
  const justComposedRef = useRef(false);
  const [isComposingState, setIsComposingState] = useState(false);

  const compositionHandlers = {
    onCompositionStartCapture: () => {
      isComposing.current = true;
      justComposedRef.current = false;
      setIsComposingState(true);
    },
    onCompositionEndCapture: () => {
      isComposing.current = false;
      setIsComposingState(false);
      // 一帧兜底：覆盖 compositionend 同 tick 先于 Enter keydown 的浏览器。
      justComposedRef.current = true;
      requestAnimationFrame(() => {
        justComposedRef.current = false;
      });
    },
  };

  const isImeActive = useCallback(
    (e: ImeKeyLike) => isImeComposingKey(e, { composing: isComposing.current, justComposed: justComposedRef.current }),
    []
  );

  const createKeyDownHandler = (
    onEnterPress: () => void,
    onKeyDownIntercept?: (e: React.KeyboardEvent) => boolean,
    sendKey: SendKeyMode = 'enter'
  ) => {
    return (e: React.KeyboardEvent) => {
      if (isImeActive(e)) return;
      if (onKeyDownIntercept?.(e)) return;
      if (isSubmitGesture(e, sendKey)) {
        e.preventDefault();
        onEnterPress();
      }
    };
  };

  return {
    isComposing,
    isComposingState,
    compositionHandlers,
    createKeyDownHandler,
    isImeActive,
  };
};
```

- [ ] **Step 4: 跑测试确认通过**

Run: `cd ui && bun test src/renderer/hooks/chat/useCompositionInput.test.ts`
Expected: PASS（全部用例）

- [ ] **Step 5: 提交**

```bash
git add ui/src/renderer/hooks/chat/useCompositionInput.ts ui/src/renderer/hooks/chat/useCompositionInput.test.ts
git -c user.name=nomifun -c user.email=rika00@qq.com commit --author="nomifun <rika00@qq.com>" -m "fix(chat): 输入法上屏 Enter 防误发送，提取提交手势纯函数"
```

## Task A2：新增 `chat.sendKey` 配置 + SendBox/引导页接入偏好

**Files:**
- Modify: `ui/src/common/config/configKeys.ts`（ConfigKeyMap，约 :55 区域）
- Modify: `ui/src/renderer/components/chat/SendBox/index.tsx`（:984 引入 useConfig、:1714-1733 onKeyDown）
- Modify: `ui/src/renderer/pages/guid/components/GuidInputCard.tsx`（:83-89 用 isImeActive）
- Modify: `ui/src/renderer/pages/guid/GuidPage.tsx`（:288 用 isSubmitGesture）

**Interfaces:**
- Consumes: A1 的 `isSubmitGesture`、`SendKeyMode`、hook 的 `isImeActive`、`createKeyDownHandler(onEnterPress, intercept?, sendKey?)`。

- [ ] **Step 1: 加配置 key** — `ui/src/common/config/configKeys.ts`，在 `'system.autoPreviewOfficeFiles'` 行后新增：

```ts
  // 发送键偏好：'enter'=Enter 发送/Shift+Enter 换行（默认）；'mod-enter'=Ctrl/⌘+Enter 发送、Enter 换行
  'chat.sendKey': 'enter' | 'mod-enter' | undefined;
```

- [ ] **Step 2: SendBox 接入偏好** — `ui/src/renderer/components/chat/SendBox/index.tsx`

在文件顶部 import 区加（与其它 hook 同组）：
```ts
import { useConfig } from '@renderer/hooks/config/useConfig';
```
在 `useCompositionInput()` 调用处（:984）下方加：
```ts
  const [sendKeyPref] = useConfig('chat.sendKey');
  const sendKey = sendKeyPref ?? 'enter';
```
把 textarea 的 `onKeyDown`（:1714 起）改为传入 `sendKey`，并让 Mod+Enter steer 拦截仅在 `'enter'` 模式生效：
```tsx
              onKeyDown={createKeyDownHandler(
                sendMessageHandler,
                (event) => {
                  if (handleAtFileMenuKeyDown(event) || handleOverlayKeyDown(event) || handleHistoryKeyDown(event)) {
                    return true;
                  }
                  // Mod(Ctrl/Cmd)+Enter steers the draft into the running turn.
                  // Only in 'enter' mode — in 'mod-enter' mode Mod+Enter IS the submit gesture.
                  if (
                    sendKey === 'enter' &&
                    onSteer &&
                    steerAvailable &&
                    event.key === 'Enter' &&
                    !event.shiftKey &&
                    (event.metaKey || event.ctrlKey)
                  ) {
                    event.preventDefault();
                    steerMessageHandler();
                    return true;
                  }
                  return false;
                },
                sendKey
              )}
```

- [ ] **Step 3: 引导页输入卡用健壮 IME 守卫** — `ui/src/renderer/pages/guid/components/GuidInputCard.tsx:83-89`

```tsx
  const { compositionHandlers, isImeActive } = useCompositionInput();
  const textareaAutoSize = isMobile ? { minRows: 2, maxRows: 8 } : { minRows: 2, maxRows: 20 };

  const handleKeyDown = (e: React.KeyboardEvent) => {
    if (isImeActive(e)) return;
    onKeyDown(e);
  };
```

- [ ] **Step 4: 引导页发送判定按偏好** — `ui/src/renderer/pages/guid/GuidPage.tsx`

顶部 import 加：
```ts
import { isSubmitGesture } from '@/renderer/hooks/chat/useCompositionInput';
import { useConfig } from '@renderer/hooks/config/useConfig';
```
在 `handleInputKeyDown` 所在组件体内取偏好：
```ts
  const [sendKeyPref] = useConfig('chat.sendKey');
  const sendKey = sendKeyPref ?? 'enter';
```
把 `handleInputKeyDown` 末尾的发送分支（:288-292）改为：
```ts
      if (isSubmitGesture(event, sendKey)) {
        event.preventDefault();
        if (!guidInput.input.trim()) return;
        send.sendMessageHandler();
      }
```
并把 `sendKey` 加入 `useCallback` 依赖数组（:294）。

- [ ] **Step 5: 类型检查 + 提交**

Run: `cd ui && bun run typecheck`
Expected: 无报错

```bash
git add ui/src/common/config/configKeys.ts ui/src/renderer/components/chat/SendBox/index.tsx ui/src/renderer/pages/guid/components/GuidInputCard.tsx ui/src/renderer/pages/guid/GuidPage.tsx
git -c user.name=nomifun -c user.email=rika00@qq.com commit --author="nomifun <rika00@qq.com>" -m "feat(chat): 新增发送键偏好(chat.sendKey)并在会话框/引导页生效"
```

## Task A3：设置面板「发送键」一行 + i18n

**Files:**
- Modify: `ui/src/renderer/components/settings/SettingsModal/contents/SystemModalContent/index.tsx`
- Modify: `ui/src/renderer/services/i18n/locales/en-US/settings.json`
- Modify: `ui/src/renderer/services/i18n/locales/zh-CN/settings.json`

- [ ] **Step 1: 加 i18n key**（两文件，紧邻 `keepAwake` 键）

`en-US/settings.json`：
```json
  "sendKey": "Send with",
  "sendKeyDesc": "Choose which key sends a message. \"Enter\" sends on Enter (Shift+Enter for a new line). \"Ctrl/⌘+Enter\" makes Enter insert a new line and sends only with the modifier.",
  "sendKeyEnter": "Enter",
  "sendKeyModEnter": "Ctrl/⌘+Enter",
```
`zh-CN/settings.json`：
```json
  "sendKey": "发送快捷键",
  "sendKeyDesc": "选择用哪个键发送消息。「Enter」按 Enter 发送（Shift+Enter 换行）；「Ctrl/⌘+Enter」让 Enter 换行、仅在按住修饰键时发送。",
  "sendKeyEnter": "Enter 发送",
  "sendKeyModEnter": "Ctrl/⌘+Enter 发送",
```

- [ ] **Step 2: 设置面板加状态 + 读取 + change 回调** — `SystemModalContent/index.tsx`

在其它 `useState` 旁（约 :55-58）加：
```ts
  const [sendKey, setSendKey] = useState<'enter' | 'mod-enter'>('enter');
```
在启动读取 `useEffect`（:85-90）内加一行：
```ts
    setSendKey(configService.get('chat.sendKey') ?? 'enter');
```
在 change 回调附近（:179-185 样板旁）加：
```ts
  const handleSendKeyChange = useCallback((value: 'enter' | 'mod-enter') => {
    setSendKey(value);
    configService.set('chat.sendKey', value).catch(() => {
      setSendKey((prev) => (prev === 'enter' ? 'mod-enter' : 'enter'));
      configService.setLocal('chat.sendKey', value === 'enter' ? 'mod-enter' : 'enter');
    });
  }, []);
```

- [ ] **Step 3: 加 preferenceItems 一行** — 在 `preferenceItems` 数组（:216 起，建议紧跟 `language` 项后）插入：

```tsx
    {
      key: 'sendKey',
      label: t('settings.sendKey'),
      description: t('settings.sendKeyDesc'),
      component: (
        <NomiSelect className='w-200px' value={sendKey} onChange={(v) => handleSendKeyChange(v as 'enter' | 'mod-enter')}>
          <NomiSelect.Option value='enter'>{t('settings.sendKeyEnter')}</NomiSelect.Option>
          <NomiSelect.Option value='mod-enter'>{t('settings.sendKeyModEnter')}</NomiSelect.Option>
        </NomiSelect>
      ),
    },
```
确认顶部已 import `NomiSelect`（若无）：`import NomiSelect from '@renderer/components/base/NomiSelect';`（按该文件既有 import 风格；`LanguageSwitcher` 同目录用法可参照）。

- [ ] **Step 4: 类型检查 + i18n 校验 + 提交**

Run: `cd ui && bun run typecheck` 然后（仓库根）`bun run check:i18n`
Expected: 均通过

```bash
git add ui/src/renderer/components/settings/SettingsModal/contents/SystemModalContent/index.tsx ui/src/renderer/services/i18n/locales/en-US/settings.json ui/src/renderer/services/i18n/locales/zh-CN/settings.json
git -c user.name=nomifun -c user.email=rika00@qq.com commit --author="nomifun <rika00@qq.com>" -m "feat(settings): 系统设置新增「发送快捷键」偏好项"
```

---

# Part B — 暂停后编辑重发（仅 Nomi、仅最近一条、截断重跑）

## Task B1：repo 新增 `delete_messages_from`（keyset 截断删除）

**Files:**
- Modify: `crates/backend/nomifun-db/src/repository/conversation.rs`（trait，加默认体）
- Modify: `crates/backend/nomifun-db/src/repository/sqlite_conversation.rs`（impl + 测试）

**Interfaces:**
- Produces: `async fn delete_messages_from(&self, conv_id: i64, from_created_at: i64, from_id: &str) -> Result<u64, DbError>`（删除 `(created_at,id) >= (from_created_at, from_id)` 的行，返回删除条数）。

- [ ] **Step 1: trait 加带默认体的方法** — `conversation.rs`，紧邻 `delete_messages_by_conversation`（:115-116）后：

```rust
    /// Deletes the message at the `(created_at, id)` keyset cursor (inclusive)
    /// and every newer message in the conversation. Returns the number of rows
    /// deleted. Default no-op so mock repos compile; SQLite overrides it.
    async fn delete_messages_from(
        &self,
        _conv_id: i64,
        _from_created_at: i64,
        _from_id: &str,
    ) -> Result<u64, DbError> {
        Ok(0)
    }
```

- [ ] **Step 2: 写失败测试** — `sqlite_conversation.rs` 的 `mod tests`（:801）末尾加：

```rust
    #[tokio::test]
    async fn delete_messages_from_removes_cursor_and_newer() {
        let (repo, _db) = setup().await;
        let conv = repo.create(&sample_conversation(SYSTEM_USER_ID)).await.unwrap();

        let mk = |id: &str, created_at: i64| MessageRow {
            id: id.to_string(),
            conversation_id: conv.id,
            msg_id: Some(id.to_string()),
            r#type: "text".to_string(),
            content: r#"{"content":"x"}"#.to_string(),
            position: Some("right".to_string()),
            status: Some("finish".to_string()),
            hidden: false,
            created_at,
        };
        // 三条：t=100,200,300
        repo.insert_message(&mk("m1", 100)).await.unwrap();
        repo.insert_message(&mk("m2", 200)).await.unwrap();
        repo.insert_message(&mk("m3", 300)).await.unwrap();

        // 从 m2 (t=200) 起（含）删除 → 删 m2、m3，留 m1
        let deleted = repo.delete_messages_from(conv.id, 200, "m2").await.unwrap();
        assert_eq!(deleted, 2);

        assert!(repo.get_message(conv.id, "m1").await.unwrap().is_some());
        assert!(repo.get_message(conv.id, "m2").await.unwrap().is_none());
        assert!(repo.get_message(conv.id, "m3").await.unwrap().is_none());
    }
```

- [ ] **Step 3: 跑测试确认失败**

Run: `cargo test -p nomifun-db delete_messages_from_removes_cursor_and_newer`
Expected: FAIL（默认体返回 0，断言 `deleted == 2` 失败）

- [ ] **Step 4: SQLite 实现** — `sqlite_conversation.rs`，紧邻 `delete_messages_by_conversation`（:459-466）后：

```rust
    async fn delete_messages_from(
        &self,
        conv_id: i64,
        from_created_at: i64,
        from_id: &str,
    ) -> Result<u64, DbError> {
        // Keyset 截断：删除 (created_at, id) >= 游标 的所有消息。
        // 命中复合索引 idx_messages_conv_created_id。
        let result = sqlx::query(
            "DELETE FROM messages \
             WHERE conversation_id = ? \
               AND (created_at > ? OR (created_at = ? AND id >= ?))",
        )
        .bind(conv_id)
        .bind(from_created_at)
        .bind(from_created_at)
        .bind(from_id)
        .execute(&self.pool)
        .await?;

        Ok(result.rows_affected())
    }
```

- [ ] **Step 5: 跑测试确认通过**

Run: `cargo test -p nomifun-db delete_messages_from_removes_cursor_and_newer`
Expected: PASS

- [ ] **Step 6: 提交**

```bash
git add crates/backend/nomifun-db/src/repository/conversation.rs crates/backend/nomifun-db/src/repository/sqlite_conversation.rs
git -c user.name=nomifun -c user.email=rika00@qq.com commit --author="nomifun <rika00@qq.com>" -m "feat(db): 新增 delete_messages_from 按 keyset 游标截断删除消息"
```

## Task B2：引擎 turn 起始锚点 + `rewind_last_turn`

**Files:**
- Modify: `crates/agent/nomi-agent/src/engine.rs`

**Interfaces:**
- Produces: `pub fn rewind_last_turn(&mut self) -> bool`（把内存 transcript 截断到最后一个 turn 起始，成功 true）。锚点字段 `last_turn_start_len: Option<usize>`。

- [ ] **Step 1: 加字段（3 处构造点）**

结构体 `AgentEngine`（:192 `steering_inbox` 后、`}` 前）加：
```rust
    /// transcript 长度锚点：最近一个 turn 的用户消息 push 之前的 messages.len()。
    /// 供 rewind_last_turn 把内存历史回退到最后一个用户 turn 之前。压缩会重写
    /// 整个 messages，使下标失效，故压缩时清空；clear_context 时一并清空。
    /// 仅内存态，不持久化到 session。
    last_turn_start_len: Option<usize>,
```
`new_with_provider` 初始化器尾部（:264 `steering_inbox: None,` 后）加 `last_turn_start_len: None,`。
`resume_with_provider` 初始化器尾部（:339 `steering_inbox: None,` 后）加 `last_turn_start_len: None,`。
测试 `make_engine` 尾部（:1534 `steering_inbox: None,` 后）加 `last_turn_start_len: None,`。

- [ ] **Step 2: 写失败测试** — `engine.rs` 的 `mod set_config_tests`（:1371）内加（紧邻 make_engine 后）：

```rust
    #[test]
    fn rewind_last_turn_truncates_to_marker() {
        use nomi_types::message::{ContentBlock, Message, Role};
        let mut engine = make_engine("rewind");
        // 既有历史：U0, A0
        engine.messages.push(Message::now(Role::User, vec![ContentBlock::Text { text: "u0".into() }]));
        engine.messages.push(Message::now(Role::Assistant, vec![ContentBlock::Text { text: "a0".into() }]));
        // 标记最后一个 turn 起始 = 当前长度(2)，再 push U1（被中断的 turn）
        engine.last_turn_start_len = Some(engine.messages.len());
        engine.messages.push(Message::now(Role::User, vec![ContentBlock::Text { text: "u1".into() }]));
        assert_eq!(engine.messages.len(), 3);

        assert!(engine.rewind_last_turn());
        assert_eq!(engine.messages.len(), 2); // U1 被回退
        assert!(engine.last_turn_start_len.is_none()); // 锚点被消费

        // 再次回退无锚点 → false
        assert!(!engine.rewind_last_turn());
    }

    #[test]
    fn rewind_last_turn_rejects_stale_marker() {
        let mut engine = make_engine("rewind-stale");
        // 锚点越界（如压缩后未清理的极端情况）→ 拒绝
        engine.last_turn_start_len = Some(5);
        assert!(!engine.rewind_last_turn());
    }
```

- [ ] **Step 3: 跑测试确认失败**

Run: `cargo test -p nomi-agent rewind_last_turn`
Expected: FAIL（`rewind_last_turn` 不存在，编译错误）

- [ ] **Step 4: 实现 rewind + 设/清锚点**

在 `clear_context`（:1305）旁新增方法：
```rust
    /// 把内存 transcript 回退到最近一个 turn 的用户消息之前（丢弃最后一个用户
    /// turn 及其后内容），用于"编辑最近一条用户消息并重跑"。成功返回 true；
    /// 无有效锚点（如已被压缩清空）返回 false，调用方应回退处理。
    pub fn rewind_last_turn(&mut self) -> bool {
        let Some(start) = self.last_turn_start_len else {
            return false;
        };
        if start > self.messages.len() {
            self.last_turn_start_len = None;
            return false;
        }
        self.messages.truncate(start);
        self.last_turn_start_len = None;
        self.save_session();
        true
    }
```
在 `run_inner` 用户消息 push 处（:670）**之前**记录锚点：
```rust
        self.current_msg_id = msg_id.to_string();
        self.output.emit_stream_start(msg_id);
        self.last_turn_start_len = Some(self.messages.len());
        self.messages.push(Message::now(
```
压缩重写处（:1158 `self.messages = result.messages;` 后）清空锚点：
```rust
                    self.messages = result.messages;
                    self.last_turn_start_len = None;
                    compacted = true;
```
`clear_context`（:1306 `self.messages.clear();` 后）清空锚点：
```rust
    pub fn clear_context(&mut self) {
        self.messages.clear();
        self.last_turn_start_len = None;
        self.compact_state = CompactState::new();
```

- [ ] **Step 5: 跑测试确认通过**

Run: `cargo test -p nomi-agent rewind_last_turn`
Expected: PASS（两个用例）

- [ ] **Step 6: 提交**

```bash
git add crates/agent/nomi-agent/src/engine.rs
git -c user.name=nomifun -c user.email=rika00@qq.com commit --author="nomifun <rika00@qq.com>" -m "feat(engine): 新增 turn 起始锚点与 rewind_last_turn 回退最后一个用户 turn"
```

## Task B3：manager + AgentInstance 暴露 `rewind_last_turn`（Nomi-only）

**Files:**
- Modify: `crates/backend/nomifun-ai-agent/src/manager/nomi/agent.rs`
- Modify: `crates/backend/nomifun-ai-agent/src/agent_task.rs`

**Interfaces:**
- Consumes: B2 的 `engine.rewind_last_turn()`。
- Produces:
  - `NomiAgentManager::rewind_last_turn(&self) -> Result<(), AppError>`（inherent）
  - `AgentInstance::rewind_last_turn(&self) -> Result<(), AppError>`（Nomi-only match）

- [ ] **Step 1: manager 方法** — `manager/nomi/agent.rs`，在 inherent impl 块内（与 `clear_context`/`steer` 同块，约 :906-921 旁）加：

```rust
    /// 回退最后一个用户 turn（编辑最近一条用户消息重跑）：先停掉在飞 turn，
    /// 再锁引擎截断到该 turn 起始。无有效锚点（已压缩等）返回 BadRequest。
    pub async fn rewind_last_turn(&self) -> Result<(), AppError> {
        self.request_stop(None, "rewind_last_turn");
        let mut engine = self.engine.lock().await;
        if !engine.rewind_last_turn() {
            return Err(AppError::BadRequest(
                "无法回退上一轮（上下文可能已被压缩），请清空上下文后重试".into(),
            ));
        }
        Ok(())
    }
```

- [ ] **Step 2: AgentInstance 委派（Nomi-only）** — `agent_task.rs`，仿 `steer`（:378-393）在 `steer` 方法后加：

```rust
    /// 回退最后一个用户 turn。仅 Nomi 原生引擎支持；其它外部 agent 上下文不可控，
    /// 返回 BadRequest（前端不会对非 Nomi 暴露入口）。
    pub async fn rewind_last_turn(&self) -> Result<(), AppError> {
        match self {
            Self::Nomi(m) => m.rewind_last_turn().await,
            Self::Acp(_) | Self::OpenClaw(_) | Self::Nanobot(_) | Self::Remote(_) => Err(
                AppError::BadRequest("Edit & resubmit is not supported for this agent type".into()),
            ),
            #[cfg(any(test, feature = "test-support"))]
            Self::Mock(_) => Ok(()),
        }
    }
```

- [ ] **Step 3: 编译确认**

Run: `cargo build -p nomifun-ai-agent`
Expected: 成功（无类型/match 缺臂错误）

- [ ] **Step 4: 提交**

```bash
git add crates/backend/nomifun-ai-agent/src/manager/nomi/agent.rs crates/backend/nomifun-ai-agent/src/agent_task.rs
git -c user.name=nomifun -c user.email=rika00@qq.com commit --author="nomifun <rika00@qq.com>" -m "feat(agent): manager/AgentInstance 暴露 rewind_last_turn(仅 Nomi)"
```

## Task B4：service `edit_and_resubmit` + 路由

**Files:**
- Modify: `crates/backend/nomifun-conversation/src/service.rs`
- Modify: `crates/backend/nomifun-conversation/src/routes.rs`

**Interfaces:**
- Consumes: `conversation_repo.{get, get_messages_keyset, delete_messages_from}`、`task_manager.get_task`、`AgentInstance::rewind_last_turn`、`self.send_message`。
- Produces:
  - `ConversationService::edit_and_resubmit(&self, user_id, conversation_id, message_id: &str, req: SendMessageRequest, task_manager) -> Result<String, AppError>`
  - 路由 `POST /api/conversations/{id}/messages/{messageId}/edit-resubmit`

- [ ] **Step 1: service 方法** — `service.rs`，在 `steer_message` 后（约 :1976 之后）新增。校验链：归属 → Nomi → 目标是最近一条 right/text → 取游标 → 拿在飞 agent → rewind → 删 DB → 复用 send_message。

```rust
    /// 编辑最近一条用户消息并截断重跑（仅 Nomi）。
    #[tracing::instrument(skip_all, fields(user_id = %user_id, conversation_id = %conversation_id, message_id = %message_id))]
    pub async fn edit_and_resubmit(
        &self,
        user_id: &str,
        conversation_id: &str,
        message_id: &str,
        req: SendMessageRequest,
        task_manager: &Arc<dyn IWorkerTaskManager>,
    ) -> Result<String, AppError> {
        if req.content.trim().is_empty() {
            return Err(AppError::BadRequest("Message content must not be empty".into()));
        }
        let conv_id = parse_conv_id(conversation_id)?;

        // 1. 归属校验
        let row = self
            .conversation_repo
            .get(conv_id)
            .await?
            .filter(|r| r.user_id == user_id)
            .ok_or_else(|| AppError::NotFound(format!("Conversation {conversation_id} not found")))?;

        // 2. 仅 Nomi
        if row.r#type != "nomi" {
            return Err(AppError::BadRequest("Edit & resubmit is only supported for Nomi conversations".into()));
        }

        // 3. 目标必须是最近一条用户(right/text)消息（保证引擎"回退最后一个 turn"
        //    与 DB"删除该条及其后"对齐）。
        let recent = self
            .conversation_repo
            .get_messages_keyset(conv_id, None, 50)
            .await?;
        let latest_user = recent
            .items
            .iter()
            .find(|m| m.position.as_deref() == Some("right") && m.r#type == "text");
        let Some(target) = latest_user else {
            return Err(AppError::BadRequest("No editable user message found".into()));
        };
        if target.id != message_id {
            return Err(AppError::BadRequest("Only the most recent user message can be edited".into()));
        }
        let (from_created_at, from_id) = (target.created_at, target.id.clone());

        // 4. 取在飞 agent 并回退最后一个 turn（内部会先停掉在飞 turn）。
        let agent = self.task(conversation_id)?;
        agent.rewind_last_turn().await?;

        // 5. 截断 DB：删除目标(含)及其后所有消息。
        self.conversation_repo
            .delete_messages_from(conv_id, from_created_at, &from_id)
            .await?;

        // 6. 复用正常发送：重新插入用户消息行 + 起新 turn。
        self.send_message(user_id, conversation_id, req, task_manager).await
    }
```

> 说明：`self.task(conversation_id)` 在无在飞 agent 时返回 `NotFound`；前端在该错误下提示用户重试（暂停后 agent 仍存活，常态可用）。

- [ ] **Step 2: 路由注册** — `routes.rs` 的 `conversation_routes`（:22）内，`messages/{messageId}` 路由旁加：

```rust
        .route(
            "/api/conversations/{id}/messages/{messageId}/edit-resubmit",
            post(edit_resubmit),
        )
```

- [ ] **Step 3: 路由 handler** — `routes.rs`，仿 `send_msg`（:197-216）加（注意双 Path 参数）：

```rust
async fn edit_resubmit(
    State(state): State<ConversationRouterState>,
    Extension(user): Extension<CurrentUser>,
    Path((id, message_id)): Path<(String, String)>,
    body: Result<Json<SendMessageRequest>, JsonRejection>,
) -> Result<(StatusCode, Json<ApiResponse<SendMessageResponse>>), AppError> {
    let Json(req) = body.map_err(|e| AppError::BadRequest(e.to_string()))?;
    let msg_id = state
        .service
        .edit_and_resubmit(&user.id, &id, &message_id, req, &state.task_manager)
        .await?;
    Ok((
        StatusCode::ACCEPTED,
        Json(ApiResponse::ok(SendMessageResponse { msg_id })),
    ))
}
```

- [ ] **Step 4: 写测试**（service 层，复用既有 mock；验证"非最近一条 → BadRequest"与"非 Nomi → BadRequest"）

在 `crates/backend/nomifun-conversation/src/service_test.rs` 末尾，仿既有 service 测试新增两条：一条插入一个非 Nomi 会话调用 `edit_and_resubmit` 断言 `Err(BadRequest)`；一条 Nomi 会话但传入非最新 message_id 断言 `Err(BadRequest)`。（沿用该文件既有 mock 构造 service 的 helper —— 实现时参照文件内最近的 `send_message`/`steer_message` 测试样板复制其 service 搭建部分。）

- [ ] **Step 5: 跑测试 + 编译**

Run: `cargo test -p nomifun-conversation edit_and_resubmit`
Expected: PASS（两条新测试），且 `cargo build -p nomifun-conversation` 成功

- [ ] **Step 6: 提交**

```bash
git add crates/backend/nomifun-conversation/src/service.rs crates/backend/nomifun-conversation/src/routes.rs crates/backend/nomifun-conversation/src/service_test.rs
git -c user.name=nomifun -c user.email=rika00@qq.com commit --author="nomifun <rika00@qq.com>" -m "feat(conversation): 新增 edit_and_resubmit 服务与 /edit-resubmit 路由(仅 Nomi、仅最近一条)"
```

## Task B5：前端 ipcBridge + emitter 事件

**Files:**
- Modify: `ui/src/common/adapter/ipcBridge.ts`
- Modify: `ui/src/renderer/utils/emitter.ts`

**Interfaces:**
- Produces:
  - `ipcBridge.conversation.editResubmit.invoke({ conversation_id, msg_id, input, files? }) -> { msg_id }`
  - emitter 事件 `'sendbox.edit': [{ msgId: string; createdAt: number; content: string }]`

- [ ] **Step 1: ipcBridge 方法** — `ipcBridge.ts`，紧邻 `steer`（:270-277）后加：

```ts
  editResubmit: httpPost<ISendMessageResult, { conversation_id: number; msg_id: string; input: string; files?: string[] }>(
    (p) => `/api/conversations/${p.conversation_id}/messages/${p.msg_id}/edit-resubmit`,
    (p) => ({
      content: p.input,
      files: p.files,
    })
  ),
```

- [ ] **Step 2: emitter 事件类型** — `emitter.ts`，在 `'sendbox.reply.clear'`（:66）后加：

```ts
  'sendbox.edit': [{ msgId: string; createdAt: number; content: string }]; // edit a sent user message
```

- [ ] **Step 3: 类型检查 + 提交**

Run: `cd ui && bun run typecheck`
Expected: 通过

```bash
git add ui/src/common/adapter/ipcBridge.ts ui/src/renderer/utils/emitter.ts
git -c user.name=nomifun -c user.email=rika00@qq.com commit --author="nomifun <rika00@qq.com>" -m "feat(chat): ipcBridge.editResubmit 与 sendbox.edit 事件"
```

## Task B6：hooks 新增 `useRemoveMessagesFrom`

**Files:**
- Modify: `ui/src/renderer/pages/conversation/Messages/hooks.ts`

**Interfaces:**
- Produces: `export const useRemoveMessagesFrom: () => (createdAt: number) => void`（移除本地 `created_at >= createdAt` 的消息）。

- [ ] **Step 1: 实现** — `hooks.ts`，紧邻 `useRemoveMessageByMsgId`（:401-410）后加：

```ts
export const useRemoveMessagesFrom = () => {
  const update = useUpdateMessageList();

  return useCallback(
    (createdAt: number) => {
      update((list) => list.filter((message) => (message.created_at ?? 0) < createdAt));
    },
    [update]
  );
};
```

- [ ] **Step 2: 类型检查 + 提交**

Run: `cd ui && bun run typecheck`
Expected: 通过

```bash
git add ui/src/renderer/pages/conversation/Messages/hooks.ts
git -c user.name=nomifun -c user.email=rika00@qq.com commit --author="nomifun <rika00@qq.com>" -m "feat(chat): 新增 useRemoveMessagesFrom 本地按时间截断消息"
```

## Task B7：SendBox「编辑模式」

**Files:**
- Modify: `ui/src/renderer/components/chat/SendBox/index.tsx`

**Interfaces:**
- Consumes: emitter `'sendbox.edit'`；新增可选 prop `onEditResubmit?: (msgId: string, message: string) => Promise<void>`。
- Produces: 编辑模式下提交走 `onEditResubmit`，并显示"编辑中"提示条。

- [ ] **Step 1: 加 prop 与状态** — 在 SendBox props 类型里（onSteer 旁，:164 区域）加：
```ts
  /** When provided (Nomi only), enables "edit a sent message" mode: the message text is
   *  recalled into the composer and submitting calls this instead of onSend. */
  onEditResubmit?: (msgId: string, message: string) => Promise<void>;
```
在解构参数列表加 `onEditResubmit,`。在状态区（`replyQuote` 旁，:253）加：
```ts
  const [editingMsgId, setEditingMsgId] = useState<string | null>(null);
  const editPrevDraftRef = useRef<string | null>(null);
```

- [ ] **Step 2: 监听 sendbox.edit** — 在其它 `useAddEventListener`（:268-269）旁加：
```ts
  useAddEventListener(
    'sendbox.edit',
    (payload) => {
      if (!onEditResubmit) return;
      editPrevDraftRef.current = latestInputRef.current;
      setEditingMsgId(payload.msgId);
      setReplyQuote(null);
      setInputRef.current(payload.content);
      requestAnimationFrame(() => {
        const textarea = containerRef.current?.querySelector('textarea');
        if (textarea instanceof HTMLTextAreaElement) {
          textarea.focus();
          const caret = textarea.value.length;
          textarea.setSelectionRange(caret, caret);
        }
      });
    },
    [onEditResubmit]
  );
```

- [ ] **Step 3: 取消编辑 helper + 提交分流** — 在 `sendMessageHandler`（:1234）开头分流；并加取消函数：
```ts
  const cancelEdit = () => {
    setEditingMsgId(null);
    const prev = editPrevDraftRef.current ?? '';
    editPrevDraftRef.current = null;
    setInput(prev);
  };
```
在 `sendMessageHandler` 函数体最前（`if (isUploading) return;` 之后）加：
```ts
    if (editingMsgId && onEditResubmit) {
      if (!input.trim()) return;
      const finalMessage = input;
      const targetId = editingMsgId;
      setEditingMsgId(null);
      editPrevDraftRef.current = null;
      setInput('');
      setIsLoading(true);
      onEditResubmit(targetId, finalMessage)
        .catch(() => {})
        .finally(() => setIsLoading(false));
      return;
    }
```

- [ ] **Step 4: 编辑提示条 UI** — 在 `replyQuote` 预览卡块（:1561-1577）后加：
```tsx
          {editingMsgId && (
            <div className='flex items-center gap-10px mb-8px px-12px py-8px rd-10px bg-fill-1 b-1 b-solid b-border-2'>
              <span className='text-13px text-t-primary'>{t('conversation.editMessage.banner')}</span>
              <div
                className='ml-auto flex-shrink-0 p-2px rd-full cursor-pointer hover:bg-fill-3 transition-colors'
                onClick={cancelEdit}
                style={{ lineHeight: 0 }}
              >
                <CloseSmall theme='outline' size='14' />
              </div>
            </div>
          )}
```

- [ ] **Step 5: 类型检查 + 提交**

Run: `cd ui && bun run typecheck`
Expected: 通过

```bash
git add ui/src/renderer/components/chat/SendBox/index.tsx
git -c user.name=nomifun -c user.email=rika00@qq.com commit --author="nomifun <rika00@qq.com>" -m "feat(chat): SendBox 支持编辑模式(回填+提示条+提交分流)"
```

## Task B8：NomiSendBox 接 `onEditResubmit` + 还原附件

**Files:**
- Modify: `ui/src/renderer/pages/conversation/platforms/nomi/NomiSendBox.tsx`

**Interfaces:**
- Consumes: `ipcBridge.conversation.editResubmit`、`useRemoveMessagesFrom`、`emitter('sendbox.edit')` 由 MessageText 触发。
- Produces: `<SendBox onEditResubmit={handleEditResubmit} />`。

- [ ] **Step 1: 引入 hook** — 顶部按既有 import 风格加 `useRemoveMessagesFrom`（与 `removeMessageByMsgId` 同源 `Messages/hooks`），并在组件体取：
```ts
  const removeMessagesFrom = useRemoveMessagesFrom();
```

- [ ] **Step 2: handleEditResubmit** — 仿 `executeCommand`（:223）加：
```ts
  const handleEditResubmit = useCallback(
    async (msgId: string, message: string) => {
      const filesToSend = collectSelectedFiles(uploadFile, atPath);
      clearFiles();
      emitter.emit('nomi.selected.file.clear');
      setWaitingResponse(true);
      try {
        const res = await ipcBridge.conversation.editResubmit.invoke({
          conversation_id,
          msg_id: msgId,
          input: buildDisplayMessage(message, filesToSend, workspacePath),
          files: filesToSend,
        });
        // 截断点之后本地立即移除旧消息，避免 DB 对齐前的闪烁。
        // 用新插入用户消息的 msg_id 之外，统一靠 history.refresh 兜底。
        emitter.emit('chat.history.refresh');
        if (filesToSend.length > 0) emitter.emit('nomi.workspace.refresh');
        setActiveMsgId(res.msg_id);
      } catch (error) {
        setWaitingResponse(false);
        Message.error(getConversationRuntimeWorkspaceErrorMessage(error, t));
        throw error;
      }
    },
    [atPath, conversation_id, uploadFile, workspacePath, setActiveMsgId, setWaitingResponse, t]
  );
```

> 注：`removeMessagesFrom` 用于进入编辑时即时清旧消息（见 Step 3 的 MessageText 触发链）；此处提交后以 `chat.history.refresh` 与后端对齐为准。

- [ ] **Step 3: 传给 SendBox** — 在 `<SendBox ... />`（:696-801）属性中、`onSend={onSendHandler}` 旁加：
```tsx
        onEditResubmit={handleEditResubmit}
```

- [ ] **Step 4: 类型检查 + 提交**

Run: `cd ui && bun run typecheck`
Expected: 通过

```bash
git add ui/src/renderer/pages/conversation/platforms/nomi/NomiSendBox.tsx
git -c user.name=nomifun -c user.email=rika00@qq.com commit --author="nomifun <rika00@qq.com>" -m "feat(nomi): NomiSendBox 接入 editResubmit 提交编辑重跑"
```

## Task B9：MessageText 用户气泡「编辑」按钮（Nomi + 最近一条 + idle）

**Files:**
- Modify: `ui/src/renderer/pages/conversation/Messages/components/MessageText.tsx`
- Modify: `ui/src/renderer/services/i18n/locales/en-US/conversation.json`
- Modify: `ui/src/renderer/services/i18n/locales/zh-CN/conversation.json`

**Interfaces:**
- Consumes: `useConversationContextSafe()`（取 `type`、`conversation_id`、`workspace`）、`useMessageList()`（判断最近一条）、`emitter('sendbox.edit')`、`useRemoveMessagesFrom`、`parseFileMarker`。
- 触发：点编辑 → 本地移除该条及其后 → emit `'sendbox.edit'`。

- [ ] **Step 1: i18n**（两文件，conversation 命名空间）

`en-US/conversation.json` 加：
```json
  "editMessage": { "banner": "Editing message — sending will re-run from here", "action": "Edit" },
```
`zh-CN/conversation.json` 加：
```json
  "editMessage": { "banner": "正在编辑消息——发送后将从这里重新生成", "action": "编辑" },
```

- [ ] **Step 2: MessageText 计算可编辑 + 编辑按钮** — 在 `MessageText` 组件体内（`isUserMessage` 定义后，:120 区域）加：

```tsx
  const conversationCtx = useConversationContextSafe();
  const messageList = useMessageList();
  const removeMessagesFrom = useRemoveMessagesFrom();

  // 仅 Nomi、用户文本消息、且为最近一条用户消息时可编辑。
  const isLatestUserMessage = useMemo(() => {
    if (!isUserMessage) return false;
    const lastRight = [...messageList].reverse().find((m) => m.position === 'right' && m.type === 'text');
    return lastRight?.msg_id != null && lastRight.msg_id === message.msg_id;
  }, [isUserMessage, messageList, message.msg_id]);

  const canEdit = conversationCtx?.type === 'nomi' && isUserMessage && message.type === 'text' && isLatestUserMessage;

  const handleEdit = () => {
    if (!message.msg_id || !message.created_at) return;
    const { text } = parseFileMarker(typeof message.content?.content === 'string' ? message.content.content : '');
    removeMessagesFrom(message.created_at);
    emitter.emit('sendbox.edit', { msgId: message.msg_id, createdAt: message.created_at, content: text });
  };

  const editButton = canEdit ? (
    <Tooltip content={t('conversation.editMessage.action', { defaultValue: 'Edit' })}>
      <div
        className='p-4px rd-4px cursor-pointer hover:bg-3 transition-colors opacity-0 pointer-events-none group-hover:opacity-100 group-hover:pointer-events-auto focus-within:opacity-100 focus-within:pointer-events-auto'
        onClick={handleEdit}
        style={{ lineHeight: 0 }}
      >
        <Edit theme='outline' size='16' fill={iconColors.secondary} />
      </div>
    </Tooltip>
  ) : null;
```
顶部 import 补：`import { Edit } from '@icon-park/react';`、`emitter`（`@/renderer/utils/emitter`）、`useConversationContextSafe`、`useMessageList`、`useRemoveMessagesFrom`（按文件既有 import 路径风格）。

- [ ] **Step 3: 放进悬浮工具行** — 把 hover 工具行（:232-245）里 `{copyButton}` 旁加上 `{editButton}`：
```tsx
            {copyButton}
            {editButton}
```

- [ ] **Step 4: 类型检查 + i18n 校验 + 提交**

Run: `cd ui && bun run typecheck` 然后（仓库根）`bun run check:i18n`
Expected: 均通过

```bash
git add ui/src/renderer/pages/conversation/Messages/components/MessageText.tsx ui/src/renderer/services/i18n/locales/en-US/conversation.json ui/src/renderer/services/i18n/locales/zh-CN/conversation.json
git -c user.name=nomifun -c user.email=rika00@qq.com commit --author="nomifun <rika00@qq.com>" -m "feat(chat): 用户消息悬浮工具行新增「编辑」按钮(仅 Nomi+最近一条)"
```

## Task B10：端到端联调与回归校验

**Files:** 无新增（验证）

- [ ] **Step 1: 全量类型检查 + i18n + 主题契约**

Run: `cd ui && bun run typecheck` ；仓库根 `bun run check:i18n`
Expected: 通过

- [ ] **Step 2: Rust 相关 crate 测试**

Run: `cargo test -p nomifun-db -p nomi-agent -p nomifun-conversation`
Expected: 全绿（含 B1/B2/B4 新测试）

- [ ] **Step 3: 手动联调（dev）** — `bun run dev` 后在 Nomi 会话：
  - 发消息→暂停→悬浮最近一条用户气泡→点编辑→输入框回填且出现"编辑中"条→改文本→Enter 提交→旧的被中断回复消失、新内容重新生成。
  - 切到 `mod-enter` 偏好：会话框裸 Enter 换行、Ctrl/⌘+Enter 发送；中文输入法上屏 Enter 不误发送。
  - 取消编辑恢复原草稿；非 Nomi 会话与非最近一条用户消息无编辑入口。

- [ ] **Step 4: 收尾提交（如有联调微调）**

```bash
git add -A && git -c user.name=nomifun -c user.email=rika00@qq.com commit --author="nomifun <rika00@qq.com>" -m "chore(chat): 编辑重发与发送键偏好联调收尾"
```

---

## Self-Review 记录

- **覆盖**：问题一（IME 守卫 A1 / 发送键偏好 A2 / 设置 UI A3）；问题二（repo B1 / 引擎 B2 / manager B3 / service+route B4 / bridge+emitter B5 / hooks B6 / SendBox B7 / NomiSendBox B8 / MessageText B9 / 联调 B10）。设计文档各节均有对应任务。
- **类型一致性**：`isSubmitGesture`/`isImeComposingKey`/`SendKeyMode`（A1）→ A2 使用一致；`delete_messages_from(conv_id,i64,&str)`（B1）→ B4 调用一致；`rewind_last_turn`（B2→B3→B4）签名一致；`editResubmit` 参数 `{conversation_id,msg_id,input,files?}`（B5）→ B8 调用一致；`'sendbox.edit'` 负载 `{msgId,createdAt,content}`（B5）→ B7 监听 / B9 触发一致。
- **已知边界**：编辑需在飞 agent（暂停后常态存活）；agent 被回收/重启后 `edit_and_resubmit` 返回 NotFound（前端提示重试）。锚点被压缩清空时 `rewind_last_turn` 返回 false → service 返回 BadRequest。截断点之后 artifacts 不清理（MVP）。
