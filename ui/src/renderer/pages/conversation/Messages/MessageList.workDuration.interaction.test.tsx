/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { act, cleanup, render } from '@testing-library/react';
import { afterEach, beforeEach, expect, mock, spyOn, test } from 'bun:test';
import { useLayoutEffect } from 'react';
import { createInstance } from 'i18next';
import { I18nextProvider, initReactI18next } from 'react-i18next';
import { MemoryRouter } from 'react-router-dom';
import { ipcBridge } from '@/common';
import { createStoredMessageMapper } from '@/common/adapter/storedMessageMapper';
import { transformUserCreatedEvent, type TMessage } from '@/common/chat/chatLib';
import { parseConversationId, parseMessageId } from '@/common/types/ids';
import { ConversationProvider, type ConversationContextValue } from '@/renderer/hooks/context/ConversationContext';
import { PreviewProvider } from '../Preview';
import messagesLocale from '@/renderer/services/i18n/locales/en-US/messages.json';
import conversationLocale from '@/renderer/services/i18n/locales/en-US/conversation.json';
import MessageList from './MessageList';
import { MessageListProvider, useUpdateMessageList } from './hooks';

const i18n = createInstance();
await i18n.use(initReactI18next).init({ lng: 'en-US', resources: {
  'en-US': { translation: { messages: messagesLocale, conversation: conversationLocale } },
} });

const WALL = 1_780_000_000_000;
const SIX_HOURS = 6 * 60 * 60 * 1000;
const CURSOR = WALL - SIX_HOURS;
const messageId = (wall: number, index: number) => {
  const time = wall.toString(16).padStart(12, '0');
  return parseMessageId(`${time.slice(0, 8)}-${time.slice(8)}-7000-8000-${String(index).padStart(12, '0')}`);
};
const root = messageId(WALL, 1);
const olderRoot = messageId(CURSOR, 2);
const products: Array<'companion' | 'nomi'> = ['companion', 'nomi'];

beforeEach(() => {
  spyOn(ipcBridge.fileStream.contentUpdate, 'on').mockImplementation(() => () => {});
  spyOn(ipcBridge.knowledge.onTreeChanged, 'on').mockImplementation(() => () => {});
  spyOn(ipcBridge.knowledge.onEntryContentUpdated, 'on').mockImplementation(() => () => {});
  spyOn(ipcBridge.workspaceOfficeWatch.start, 'invoke').mockResolvedValue(undefined);
  spyOn(ipcBridge.workspaceOfficeWatch.stop, 'invoke').mockResolvedValue(undefined);
  spyOn(ipcBridge.workspaceOfficeWatch.fileAdded, 'on').mockImplementation(() => () => {});
  spyOn(ipcBridge.fs.listWorkspaceFiles, 'invoke').mockResolvedValue([]);
});
afterEach(() => { cleanup(); mock.restore(); });

function harness(product: typeof products[number], initial: TMessage[], context: ConversationContextValue) {
  let update!: ReturnType<typeof useUpdateMessageList>;
  const Controls = () => {
    const updateMessages = useUpdateMessageList();
    useLayoutEffect(() => { update = updateMessages; }, [updateMessages]);
    return null;
  };
  // Both actual product entrypoints mount NomiChat and this same typed context;
  // Companion is a product surface, not a second ConversationContext type.
  const view = (value = context, messages = initial) => <MemoryRouter>
    <I18nextProvider i18n={i18n}>
      <PreviewProvider persistNamespace={`work-duration-${product}`} subscribeGlobalOpen={false}>
        <ConversationProvider value={value}>
          <MessageListProvider initialValue={messages}>
            <div data-product-surface={product}><MessageList /><Controls /></div>
          </MessageListProvider>
        </ConversationProvider>
      </PreviewProvider>
    </I18nextProvider>
  </MemoryRouter>;
  return { view, publish: (messages: TMessage[]) => act(() => update(() => messages)) };
}

function currentLabel(container: HTMLElement) {
  return container.querySelector(`#message-turn-disclosure-${root} .turn-process-disclosure__label`)?.textContent;
}

test.each(products)('%s: first accepted paint, start, stream, settlement and cold history share the wall clock', (product) => {
  let now = WALL;
  spyOn(Date, 'now').mockImplementation(() => now);
  let intervalId = 0;
  const ticks = new Map<number, () => void>();
  spyOn(window, 'setInterval').mockImplementation(((handler: TimerHandler) => {
    const id = ++intervalId;
    ticks.set(id, handler as () => void);
    return id;
  }) as typeof window.setInterval);
  spyOn(window, 'clearInterval').mockImplementation(id => { ticks.delete(Number(id)); });
  const conversationId = parseConversationId(product === 'companion'
    ? '0190f5fe-7c00-7a00-8000-000000001201'
    : '0190f5fe-7c00-7a00-8000-000000001202');
  const accepted = transformUserCreatedEvent({ conversation_id: conversationId, msg_id: root,
    content: 'Explain Java variables', position: 'right', status: 'finish', hidden: false,
    created_at: CURSOR + 1, display_at_ms: WALL }, conversationId)!;
  let messages: TMessage[] = [accepted];
  let context: ConversationContextValue = { conversation_id: conversationId, type: 'nomi', readOnly: true,
    isProcessing: true, isTurnStateHydrated: true, activeRequestMessageId: root };
  const timeline = harness(product, messages, context);
  const page = render(timeline.view());
  // This assertion is deliberately before any runtime/start/history promise or
  // timer callback. The accepted notification already has the real wall time;
  // its created_at is only Session/keyset order and is six hours older.
  expect(currentLabel(page.container)).toBe('Worked for 0s');
  context = { ...context, activeTurnId: root };
  page.rerender(timeline.view(context));
  expect(currentLabel(page.container)).toBe('Worked for 0s');

  const append = (...rows: TMessage[]) => {
    messages = [...messages, ...rows];
    timeline.publish(messages);
  };
  now = WALL + 2000;
  append({ id: 'current-thinking', msg_id: messageId(WALL + 2000, 3), turn_id: root,
    conversation_id: conversationId, position: 'left', type: 'thinking', created_at: CURSOR + 2,
    content: { content: 'Inspecting the variable declaration.', status: 'thinking' } });
  expect(currentLabel(page.container)).toBe('Worked for 2s');

  // Canonical Turn start replaces the provisional accepted-input clock. It is
  // a later wall-clock event, not a Session seq or a previous Turn's start.
  now = WALL + 3000;
  context = { ...context, activeTurnStartedAt: WALL + 1000 };
  page.rerender(timeline.view(context));
  expect(currentLabel(page.container)).toBe('Worked for 2s');
  now = WALL + 4000;
  append({ id: 'current-answer', msg_id: messageId(WALL + 4000, 4), turn_id: root,
    conversation_id: conversationId, position: 'left', type: 'text', created_at: CURSOR + 3,
    content: { content: '`int count = 1;` declares an integer.', display_at_ms: now } });
  expect(currentLabel(page.container)).toBe('Worked for 3s');

  append({ id: 'late-old-user', msg_id: olderRoot, conversation_id: conversationId,
    type: 'text', position: 'right', created_at: CURSOR - 10,
    content: { content: 'Previous task', display_at_ms: CURSOR } },
  { id: 'late-old-summary', msg_id: messageId(CURSOR + 5000, 5), turn_id: olderRoot,
    conversation_id: conversationId, type: 'agent_status', created_at: CURSOR - 8,
    content: { backend: 'nomi', status: 'prepared', turn_summary: true, turn_state: 'completed',
      started_at_ms: CURSOR, finished_at_ms: CURSOR + 17_000 } });
  expect(currentLabel(page.container)).toBe('Worked for 3s');
  expect(page.container.querySelectorAll('.turn-process-disclosure--live')).toHaveLength(1);
  now = WALL + 6000;
  act(() => { for (const tick of [...ticks.values()]) tick(); });
  expect(currentLabel(page.container)).toBe('Worked for 5s');

  now = WALL + 6500;
  append({ id: 'current-summary', msg_id: messageId(now, 6), turn_id: root,
    conversation_id: conversationId, type: 'agent_status', created_at: CURSOR + 4,
    content: { backend: 'nomi', status: 'prepared', turn_summary: true, turn_state: 'completed',
      started_at_ms: WALL + 1000, finished_at_ms: now } });
  expect(page.container.querySelector('.turn-process-disclosure--live')).toBeNull();
  expect(currentLabel(page.container)).toBe('Worked for 5s');
  now += 30_000;
  act(() => { for (const tick of [...ticks.values()]) tick(); });
  expect(currentLabel(page.container)).toBe('Worked for 5s');
  context = { ...context, isProcessing: false, activeTurnId: undefined, activeTurnStartedAt: undefined };
  page.rerender(timeline.view(context));
  expect(currentLabel(page.container)).toBe('Worked for 5s');
  page.unmount();

  let renderKey = 0;
  const fromStored = createStoredMessageMapper(() => `cold-clock-${++renderKey}`);
  const stored = messages.map(message => fromStored({
    message_id: message.msg_id!, msg_id: message.msg_id, conversation_id: conversationId,
    type: message.type, position: message.position, status: 'finish', hidden: false,
    created_at: message.created_at, content: { ...message.content, turn_id: message.turn_id },
  }));
  const cold = render(harness(product, stored, context).view());
  expect(currentLabel(cold.container)).toBe('Worked for 5s');
  expect(cold.container.querySelector('.turn-process-disclosure--live')).toBeNull();
});

test.each(products)('%s: a partial active history window uses the canonical start outside its page', (product) => {
  spyOn(Date, 'now').mockReturnValue(WALL);
  const conversationId = parseConversationId(product === 'companion'
    ? '0190f5fe-7c00-7a00-8000-000000001211'
    : '0190f5fe-7c00-7a00-8000-000000001212');
  const context: ConversationContextValue = { conversation_id: conversationId, type: 'nomi', readOnly: true,
    isProcessing: true, isTurnStateHydrated: true, activeTurnId: root, activeTurnStartedAt: WALL - 5000 };
  const messages: TMessage[] = [{ id: 'windowed-thinking', msg_id: messageId(WALL, 7), turn_id: root,
    conversation_id: conversationId, position: 'left', type: 'thinking', created_at: CURSOR + 7,
    content: { content: 'The accepted row and canonical start are outside this page.', status: 'thinking' } }];
  const page = render(harness(product, messages, context).view());
  expect(currentLabel(page.container)).toBe('Worked for 5s');
});

test.each(products)('%s: public text before runtime start uses its display wall time without a user row', (product) => {
  spyOn(Date, 'now').mockReturnValue(WALL);
  const conversationId = parseConversationId(product === 'companion'
    ? '0190f5fe-7c00-7a00-8000-000000001221'
    : '0190f5fe-7c00-7a00-8000-000000001222');
  const context: ConversationContextValue = { conversation_id: conversationId, type: 'nomi', readOnly: true,
    isProcessing: true, isTurnStateHydrated: false };
  const messages: TMessage[] = [{ id: 'early-public-text', msg_id: messageId(WALL, 8), turn_id: root,
    conversation_id: conversationId, position: 'left', type: 'text', created_at: CURSOR + 8,
    content: { content: 'I will explain the declaration.', display_at_ms: WALL } }];
  const page = render(harness(product, messages, context).view());
  expect(currentLabel(page.container)).toBe('Worked for 0s');
});
