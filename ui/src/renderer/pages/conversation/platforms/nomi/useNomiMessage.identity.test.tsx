import { afterEach, expect, mock, spyOn, test } from 'bun:test';
import { act, cleanup, renderHook, waitFor } from '@testing-library/react';
import { createElement, type PropsWithChildren } from 'react';
import { ipcBridge } from '@/common';
import type { TChatConversation } from '@/common/config/storage';
import { parseConversationId, parseMessageId } from '@/common/types/ids';
import { MessageListProvider } from '../../Messages/hooks';
import { useNomiMessage } from './useNomiMessage';

const conversationId = parseConversationId('0190f5fe-7c00-7a00-8000-000000000981');
const requestId = parseMessageId('0190f5fe-7c00-7a00-8000-000000000982');
const turnId = parseMessageId('0190f5fe-7c00-7a00-8000-000000000983');
const nextTurnId = parseMessageId('0190f5fe-7c00-7a00-8000-000000000984');
const resumedTurnId = parseMessageId('0190f5fe-7c00-7a00-8000-000000000985');
const wrapper = ({ children }: PropsWithChildren) => createElement(MessageListProvider, { initialValue: [] }, children);
afterEach(() => { cleanup(); mock.restore(); });

test('verified hydration and turn.started publish exact identity and stopping clears the visible active identity', async () => {
  let started: Parameters<typeof ipcBridge.conversation.turnStarted.on>[0] | undefined;
  let reconnect: Parameters<typeof ipcBridge.conversation.reconnected.on>[0] | undefined;
  spyOn(ipcBridge.conversation.turnStarted, 'on').mockImplementation(listener => { started = listener; return () => {}; });
  spyOn(ipcBridge.conversation.reconnected, 'on').mockImplementation(listener => { reconnect = listener; return () => {}; });
  for (const name of ['responseStream', 'turnCompleted', 'turnPaused', 'userCreated', 'messageAnnotated'] as const) {
    spyOn(ipcBridge.conversation[name], 'on').mockImplementation(() => () => {});
  }
  const runtime = { state: 'running' as const, is_processing: true, can_send_message: false, has_runtime: true, active_turn_id: turnId, processing_started_at: 1000 };
  const get = spyOn(ipcBridge.conversation.get, 'invoke').mockResolvedValue({ id: conversationId, type: 'nomi', status: 'running', runtime } as TChatConversation);
  const hook = renderHook(() => useNomiMessage(conversationId), { wrapper });
  await waitFor(() => expect(hook.result.current.hasHydratedRunningState).toBe(true));
  expect(hook.result.current.running).toBe(true);
  expect(hook.result.current.activeTurnId).toBe(turnId);
  expect(hook.result.current.activeTurnStartedAt).toBe(1000);
  expect(hook.result.current.activeRequestMessageId).toBeUndefined();
  act(() => hook.result.current.setActiveMsgId(requestId));
  expect(hook.result.current.activeRequestMessageId).toBe(requestId);
  // Reconnection can discover a successor even when the old terminal was lost.
  // Its identity must not reassign the former Turn's request to the new Turn.
  get.mockResolvedValue({ id: conversationId, type: 'nomi', status: 'running',
    runtime: { ...runtime, active_turn_id: nextTurnId, processing_started_at: 2000 } } as TChatConversation);
  act(() => reconnect!());
  await waitFor(() => expect(hook.result.current.activeTurnId).toBe(nextTurnId));
  expect(hook.result.current.activeTurnStartedAt).toBe(2000);
  expect(hook.result.current.activeRequestMessageId).toBeUndefined();
  act(() => hook.result.current.resetState());
  expect(hook.result.current.activeTurnId).toBeUndefined();
  expect(hook.result.current.activeTurnStartedAt).toBeUndefined();
  expect(hook.result.current.activeRequestMessageId).toBeUndefined();
  act(() => hook.result.current.confirmStopped());
  act(() => {
    hook.result.current.setWaitingResponse(true);
    hook.result.current.setActiveMsgId(requestId);
  });
  expect(hook.result.current.activeTurnId).toBeUndefined();
  get.mockResolvedValue({ id: conversationId, type: 'nomi', status: 'running',
    runtime: { ...runtime, active_turn_id: resumedTurnId } } as TChatConversation);
  act(() => started!({ conversation_id: conversationId, turn_id: resumedTurnId, status: 'running',
    state: 'ai_generating', detail: '', can_send_message: false, runtime: { ...runtime, active_turn_id: resumedTurnId, processing_started_at: 3000 } }));
  await waitFor(() => expect(hook.result.current.activeTurnId).toBe(resumedTurnId));
  expect(hook.result.current.activeTurnStartedAt).toBe(3000);
  expect(hook.result.current.activeRequestMessageId).toBe(requestId);
});

test('a full chat remount recovers the same canonical start without resetting its clock', async () => {
  for (const name of ['responseStream', 'turnStarted', 'turnCompleted', 'turnPaused', 'userCreated', 'messageAnnotated', 'reconnected'] as const) {
    spyOn(ipcBridge.conversation[name], 'on').mockImplementation(() => () => {});
  }
  const snapshot = { id: conversationId, type: 'nomi', status: 'running', runtime: {
    state: 'running', is_processing: true, can_send_message: false, has_runtime: true,
    active_turn_id: turnId, processing_started_at: 1000,
  } } as TChatConversation;
  const get = spyOn(ipcBridge.conversation.get, 'invoke').mockResolvedValue(snapshot);
  const first = renderHook(() => useNomiMessage(conversationId), { wrapper });
  await waitFor(() => expect(first.result.current.hasHydratedRunningState).toBe(true));
  expect(first.result.current.activeTurnStartedAt).toBe(1000);
  first.unmount();
  let resolveSnapshot!: (value: TChatConversation) => void;
  get.mockImplementation(() => new Promise(resolve => { resolveSnapshot = resolve; }));
  const returned = renderHook(() => useNomiMessage(conversationId), { wrapper });
  expect(returned.result.current.hasHydratedRunningState).toBe(false);
  await act(async () => resolveSnapshot(snapshot));
  await waitFor(() => expect(returned.result.current.hasHydratedRunningState).toBe(true));
  expect(returned.result.current.running).toBe(true);
  expect(returned.result.current.activeTurnId).toBe(turnId);
  expect(returned.result.current.activeTurnStartedAt).toBe(1000);
});
