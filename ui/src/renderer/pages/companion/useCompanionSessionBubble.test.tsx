/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { afterEach, describe, expect, jest, mock, spyOn, test } from 'bun:test';
import { act, cleanup, renderHook } from '@testing-library/react';
import { createInstance } from 'i18next';
import { I18nextProvider } from 'react-i18next';
import type { PropsWithChildren } from 'react';
import { ipcBridge } from '@/common';
import type { TChatConversation } from '@/common/config/storage';
import type { TMessage } from '@/common/chat/chatLib';
import type { IConversationTurnCompletedEvent, IConversationTurnStartedEvent, IResponseMessage, IUserMessageCreatedEvent } from '@/common/adapter/ipcBridge';
import { parseCompanionId, parseConversationId, parseMessageId } from '@/common/types/ids';
import nomi from '@/renderer/services/i18n/locales/en-US/nomi.json';
import { useCompanionSessionBubble } from './useCompanionSessionBubble';

const i18n = createInstance();
await i18n.init({ lng: 'en-US', resources: { 'en-US': { translation: { nomi } } }, interpolation: { escapeValue: false } });
const wrapper = ({ children }: PropsWithChildren) => <I18nextProvider i18n={i18n}>{children}</I18nextProvider>;
const companionId = parseCompanionId('019f0000-0000-7000-8000-000000000911');
const conversationId = parseConversationId('019f0000-0000-7000-8000-000000000912');
const root = parseMessageId('019f0000-0000-7000-8000-000000000913');
const successor = parseMessageId('019f0000-0000-7000-8000-000000000914');
const segment = parseMessageId('019f0000-0000-7000-8000-000000000915');
const idle = { id: conversationId, status: 'finished', runtime: { state: 'idle', can_send_message: true,
  is_processing: false, has_runtime: false } } as TChatConversation;
const busy = (turn = root) => ({ ...idle, status: 'running', runtime: { state: 'running', can_send_message: false,
  is_processing: true, has_runtime: true, active_turn_id: turn } }) as TChatConversation;
const accepted = (turn = root, extra: Partial<IUserMessageCreatedEvent> = {}): IUserMessageCreatedEvent => ({
  conversation_id: conversationId, msg_id: turn, content: 'Question', position: 'right', status: 'finish', created_at: 100, ...extra,
});
const content = (text: string, extra: Partial<IResponseMessage> = {}): IResponseMessage => ({
  conversation_id: conversationId, turn_id: root, msg_id: segment, type: 'content', data: { content: text }, ...extra,
});
const completed = (turn = root): IConversationTurnCompletedEvent => ({
  conversation_id: conversationId, turn_id: turn, status: 'finished', state: 'ai_waiting_input', detail: '',
  can_send_message: true, runtime: idle.runtime!, workspace: '', model: { platform: '', name: '', use_model: '' },
  last_message: { content: {}, created_at: 100 },
});
const stored = (text: string): TMessage => ({ id: 'saved', message_id: segment, msg_id: segment, turn_id: root,
  conversation_id: conversationId, position: 'left', type: 'text', content: { content: text }, created_at: 100 });
afterEach(() => { cleanup(); jest.useRealTimers(); mock.restore(); });

function transport(initial: TChatConversation = idle) {
  let authority = initial;
  let saved: TMessage[] = [];
  let bindingListener: ((value: { conversation_id: typeof conversationId }) => void) | null = null;
  let delayBinding = false;
  let runtimeQuery: (() => Promise<TChatConversation>) | null = null;
  spyOn(ipcBridge.companion.getCompanionSession, 'invoke').mockImplementation(() => delayBinding
    ? new Promise(resolve => { bindingListener = resolve; }) : Promise.resolve({ conversation_id: conversationId }));
  spyOn(ipcBridge.conversation.get, 'invoke').mockImplementation(() => runtimeQuery ? runtimeQuery() : Promise.resolve(authority));
  const history = spyOn(ipcBridge.database.getConversationMessages, 'invoke').mockImplementation(() =>
    Promise.resolve({ items: saved, total: saved.length, has_more: false }));
  const stop = spyOn(ipcBridge.conversation.stop, 'invoke').mockResolvedValue(undefined);
  let user: (event: IUserMessageCreatedEvent) => void = () => {};
  let start: (event: IConversationTurnStartedEvent) => void = () => {};
  let finish: (event: IConversationTurnCompletedEvent) => void = () => {};
  let stream: (event: IResponseMessage) => void = () => {};
  let reconnect: () => void = () => {};
  spyOn(ipcBridge.conversation.userCreated, 'on').mockImplementation(listener => { user = listener; return () => {}; });
  spyOn(ipcBridge.conversation.turnStarted, 'on').mockImplementation(listener => { start = listener; return () => {}; });
  spyOn(ipcBridge.conversation.turnCompleted, 'on').mockImplementation(listener => { finish = listener; return () => {}; });
  spyOn(ipcBridge.conversation.responseStream, 'on').mockImplementation(listener => { stream = listener; return () => {}; });
  spyOn(ipcBridge.conversation.reconnected, 'on').mockImplementation(listener => { reconnect = () => listener(); return () => {}; });
  spyOn(ipcBridge.conversation.turnPaused, 'on').mockImplementation(() => () => {});
  spyOn(ipcBridge.companion.onConfigUpdated, 'on').mockImplementation(() => () => {});
  return { user: (event: IUserMessageCreatedEvent) => user(event), finish: (event: IConversationTurnCompletedEvent) => finish(event),
    stream: (event: IResponseMessage) => stream(event), reconnect: () => reconnect(),
    start: (turn = root) => start({ conversation_id: conversationId, turn_id: turn, status: 'running', state: 'ai_generating',
      detail: '', can_send_message: false, runtime: busy(turn).runtime! }),
    setAuthority: (value: TChatConversation) => { authority = value; }, setSaved: (value: TMessage[]) => { saved = value; },
    deferBinding: () => { delayBinding = true; }, resolveBinding: () => {
      delayBinding = false; bindingListener?.({ conversation_id: conversationId });
    }, setRuntimeQuery: (value: (() => Promise<TChatConversation>) | null) => { runtimeQuery = value; }, stop, history };
}
const flush = async () => act(async () => { await Promise.resolve(); await Promise.resolve(); });

describe('companion bubble observes canonical cross-window lifecycle', () => {
  test('main-window accepted input opens the bubble and long reasoning never times out', async () => {
    jest.useFakeTimers();
    const wire = transport();
    const view = renderHook(() => useCompanionSessionBubble(companionId), { wrapper });
    await flush();
    wire.setAuthority(busy());
    await act(async () => { wire.user(accepted()); wire.start(); wire.stream(content('Hello')); });
    expect(view.result.current.bubble).toBe('Hello');
    expect(view.result.current.running).toBe(true);
    await act(async () => { wire.stream(content('', { type: 'thinking' })); jest.advanceTimersByTime(60_000); });
    expect(view.result.current.bubble).toBe('Hello');
    expect(view.result.current.running).toBe(true);
  });
  test('late terminals and POST receipts cannot clear or reopen a successor', async () => {
    const wire = transport();
    const view = renderHook(() => useCompanionSessionBubble(companionId), { wrapper });
    await flush();
    wire.setAuthority(busy());
    await act(async () => { wire.user(accepted()); wire.stream(content('First')); });
    wire.setAuthority(idle);
    await act(async () => { wire.finish(completed()); });
    expect(view.result.current.running).toBe(false);
    wire.setAuthority(busy(successor));
    await act(async () => { wire.user(accepted(successor)); wire.start(successor); });
    await act(async () => { wire.finish(completed()); wire.stream(content('Old')); view.result.current.reconcile(root); });
    expect(view.result.current.turnId).toBe(successor);
    expect(view.result.current.running).toBe(true);
    expect(view.result.current.bubble).toBe('…');
  });
  test('mid-turn mount hydrates committed text and reconnect repairs a lost terminal', async () => {
    const wire = transport(busy());
    wire.setSaved([stored('Partial')]);
    const view = renderHook(() => useCompanionSessionBubble(companionId), { wrapper });
    await flush();
    expect(view.result.current.bubble).toBe('Partial');
    wire.setAuthority(idle);
    wire.setSaved([stored('Complete answer from the disconnected Turn')]);
    await act(async () => { wire.reconnect(); });
    expect(view.result.current.bubble).toBe('Complete answer from the disconnected Turn');
    expect(view.result.current.running).toBe(false);
  });
  test('binding resolution retains early streams and terminal even when admission was missed', async () => {
    const wire = transport();
    wire.deferBinding();
    const view = renderHook(() => useCompanionSessionBubble(companionId), { wrapper });
    await act(async () => { wire.stream(content('Already finished')); wire.finish(completed()); });
    await act(async () => { wire.resolveBinding(); });
    expect(view.result.current.bubble).toBe('Already finished');
    expect(view.result.current.phase).toBe('settled');
  });
  test('bound session retains a terminal that arrives before the first runtime read returns', async () => {
    const wire = transport();
    let resolveRuntime: ((conversation: TChatConversation) => void) | undefined;
    wire.setRuntimeQuery(() => new Promise(resolve => { resolveRuntime = resolve; }));
    const view = renderHook(() => useCompanionSessionBubble(companionId), { wrapper });
    await flush();
    expect(view.result.current.conversationId).toBe(conversationId);
    expect(view.result.current.turnId).toBeNull();
    await act(async () => { wire.stream(content('Completed during hydration')); wire.finish(completed()); });
    await act(async () => { resolveRuntime?.(idle); });
    expect(view.result.current.bubble).toBe('Completed during hydration');
    expect(view.result.current.phase).toBe('settled');
  });
  test('dismissal is per Turn, cancellation names that Turn, and a new remote input uses the same session', async () => {
    const wire = transport(busy());
    const view = renderHook(() => useCompanionSessionBubble(companionId), { wrapper });
    await flush();
    await act(async () => { wire.stream(content('First')); view.result.current.dismiss(); });
    expect(wire.stop).toHaveBeenCalledWith({ conversation_id: conversationId, expected_turn_id: root });
    await act(async () => { wire.stream(content('Ignored')); });
    expect(view.result.current.bubble).toBe('');
    wire.setAuthority(busy(successor));
    await act(async () => { wire.user(accepted(successor, { channel_platform: 'telegram', content: 'Remote question' })); });
    expect(view.result.current.running).toBe(true);
    expect(view.result.current.bubble).toBe('…');
    expect(view.result.current.remoteHeader).toEqual({ platform: 'telegram', inbound: 'Remote question' });
  });
  test('hover pauses completion dismissal and leaving grants another full reading interval', async () => {
    jest.useFakeTimers();
    const wire = transport(busy());
    const view = renderHook(() => useCompanionSessionBubble(companionId), { wrapper });
    await flush();
    wire.setAuthority(idle);
    await act(async () => { wire.stream(content('Read me')); wire.finish(completed()); });
    act(() => { jest.advanceTimersByTime(12_000); view.result.current.setHovered(true); });
    act(() => { jest.advanceTimersByTime(60_000); });
    expect(view.result.current.bubble).toBe('Read me');
    act(() => { view.result.current.setHovered(false); jest.advanceTimersByTime(23_999); });
    expect(view.result.current.bubble).toBe('Read me');
    act(() => { jest.advanceTimersByTime(1); });
    expect(view.result.current.bubble).toBe('');
  });
  test('an older in-flight runtime read cannot settle the newly accepted root', async () => {
    const wire = transport(busy());
    const view = renderHook(() => useCompanionSessionBubble(companionId), { wrapper });
    await flush();
    let resolveOld: ((value: TChatConversation) => void) | undefined;
    wire.setRuntimeQuery(() => new Promise(resolve => { resolveOld = resolve; }));
    await act(async () => { view.result.current.reconcile(); });
    wire.setRuntimeQuery(null);
    wire.setAuthority(busy(successor));
    await act(async () => { wire.user(accepted(successor)); });
    await act(async () => { resolveOld?.(idle); });
    expect(view.result.current.turnId).toBe(successor);
    expect(view.result.current.running).toBe(true);
  });
  test('failed interruption restores normal streaming for the exact still-running Turn', async () => {
    const wire = transport(busy());
    wire.stop.mockRejectedValue(new Error('Temporary transport failure'));
    spyOn(console, 'warn').mockImplementation(() => {});
    const view = renderHook(() => useCompanionSessionBubble(companionId), { wrapper });
    await flush();
    await act(async () => { wire.stream(content('First')); view.result.current.interrupt(); });
    await act(async () => { wire.stream(content(' continues')); });
    expect(view.result.current.running).toBe(true);
    expect(view.result.current.bubble).toBe('First continues');
  });
  test('history recovery pages through a tool-heavy Turn only as far as its accepted root', async () => {
    const wire = transport(busy());
    const newestId = parseMessageId('019f0000-0000-7000-8000-000000000916');
    const process = { ...stored(''), message_id: newestId, msg_id: newestId, type: 'thinking', created_at: 200,
      content: { content: 'Thinking', status: 'done' } } as TMessage;
    wire.history.mockImplementation(async query => query.cursor === ''
      ? { items: [process], total: 102, has_more: true }
      : { items: [stored('Earlier response'), { ...stored('Question'), message_id: root, msg_id: root, position: 'right' }],
        total: 102, has_more: true });
    const view = renderHook(() => useCompanionSessionBubble(companionId), { wrapper });
    await flush();
    expect(view.result.current.bubble).toBe('Earlier response');
    expect(wire.history).toHaveBeenCalledTimes(2);
    expect(wire.history.mock.calls[1]![0].cursor).toBe(`200:${newestId}`);
  });
  test('temporary binding failures retry with bounded backoff and recover without a renderer remount', async () => {
    jest.useFakeTimers();
    const wire = transport(busy());
    let attempts = 0;
    spyOn(ipcBridge.companion.getCompanionSession, 'invoke').mockImplementation(async () => {
      if (++attempts === 1) throw new Error('Backend starting');
      return { conversation_id: conversationId };
    });
    spyOn(console, 'warn').mockImplementation(() => {});
    const view = renderHook(() => useCompanionSessionBubble(companionId, '天天'), { wrapper });
    await flush();
    expect(view.result.current.conversationId).toBeNull();
    await act(async () => { jest.advanceTimersByTime(120); });
    expect(view.result.current.conversationId).toBe(conversationId);
    await act(async () => { wire.stream(content('', { type: 'tool_call' })); });
    expect(view.result.current.bubble).toBe(i18n.t('nomi.companion.usingTools', { name: '天天' }));
  });
  test('reconnect re-reads a history snapshot raced by live append and recovers the missing prefix', async () => {
    const wire = transport(busy());
    wire.setSaved([stored('One')]);
    const view = renderHook(() => useCompanionSessionBubble(companionId), { wrapper });
    await flush();
    let resolveSnapshot: ((page: Awaited<ReturnType<typeof ipcBridge.database.getConversationMessages.invoke>>) => void) | undefined;
    let historyAttempts = 0;
    wire.history.mockImplementation(() => ++historyAttempts === 1 ? new Promise(resolve => { resolveSnapshot = resolve; })
      : Promise.resolve({ items: [stored('One two three')], total: 1, has_more: false }));
    await act(async () => { wire.reconnect(); });
    await act(async () => { wire.stream(content(' three')); });
    expect(view.result.current.bubble).toBe('One three');
    await act(async () => { resolveSnapshot?.({ items: [stored('One two')], total: 1, has_more: false }); });
    expect(view.result.current.bubble).toBe('One two three');
    expect(view.result.current.running).toBe(true);
    expect(historyAttempts).toBe(2);
  });
  test('two raced snapshots leave recovery pending until a backed-off third read restores the prefix', async () => {
    jest.useFakeTimers();
    const wire = transport(busy());
    wire.setSaved([stored('One')]);
    const view = renderHook(() => useCompanionSessionBubble(companionId), { wrapper });
    await flush();
    type HistoryPage = Awaited<ReturnType<typeof ipcBridge.database.getConversationMessages.invoke>>;
    const snapshots: Array<(page: HistoryPage) => void> = [];
    let attempts = 0;
    wire.history.mockImplementation(() => ++attempts <= 2 ? new Promise(resolve => snapshots.push(resolve))
      : Promise.resolve({ items: [stored('One missing two three four')], total: 1, has_more: false }));
    await act(async () => { wire.reconnect(); });
    await act(async () => { wire.stream(content(' three')); snapshots[0]!({ items: [stored('One missing two')], total: 1, has_more: false }); });
    await act(async () => { wire.stream(content(' four')); snapshots[1]!({ items: [stored('One missing two three')], total: 1, has_more: false }); });
    expect(view.result.current.bubble).toBe('One three four');
    expect(attempts).toBe(2);
    await act(async () => { jest.advanceTimersByTime(119); });
    expect(attempts).toBe(2);
    await act(async () => { jest.advanceTimersByTime(1); });
    expect(attempts).toBe(3);
    expect(view.result.current.bubble).toBe('One missing two three four');
    expect(view.result.current.running).toBe(true);
  });
  test.each(['successor', 'terminal', 'unmount'] as const)('pending history retry cannot outlive %s', async disposition => {
    jest.useFakeTimers();
    const wire = transport(busy());
    wire.setSaved([stored('One')]);
    const view = renderHook(() => useCompanionSessionBubble(companionId), { wrapper });
    await flush();
    type HistoryPage = Awaited<ReturnType<typeof ipcBridge.database.getConversationMessages.invoke>>;
    const snapshots: Array<(page: HistoryPage) => void> = [];
    let attempts = 0;
    wire.history.mockImplementation(() => ++attempts <= 2 ? new Promise(resolve => snapshots.push(resolve))
      : Promise.resolve({ items: [disposition === 'successor' ? { ...stored('Successor'), turn_id: successor }
        : stored('Completed answer')], total: 1, has_more: false }));
    await act(async () => { wire.reconnect(); });
    await act(async () => { wire.stream(content(' three')); snapshots[0]!({ items: [stored('One missing two')], total: 1, has_more: false }); });
    await act(async () => { wire.stream(content(' four')); snapshots[1]!({ items: [stored('One missing two three')], total: 1, has_more: false }); });
    if (disposition === 'successor') {
      wire.setAuthority(busy(successor));
      await act(async () => { wire.user(accepted(successor)); });
      expect(view.result.current.turnId).toBe(successor);
      expect(view.result.current.bubble).toBe('Successor');
    } else if (disposition === 'terminal') {
      wire.setAuthority(idle);
      await act(async () => { wire.finish(completed()); });
      expect(view.result.current.phase).toBe('settled');
      expect(view.result.current.bubble).toBe('Completed answer');
    } else view.unmount();
    const completedAttempts = attempts;
    await act(async () => { jest.advanceTimersByTime(1_000); });
    expect(attempts).toBe(completedAttempts);
    if (disposition === 'successor') expect(view.result.current.bubble).toBe('Successor');
  });
});
