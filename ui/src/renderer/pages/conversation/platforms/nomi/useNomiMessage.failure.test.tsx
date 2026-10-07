import { afterEach, expect, mock, spyOn, test } from 'bun:test';
import { act, cleanup, renderHook, waitFor } from '@testing-library/react';
import { createElement, type PropsWithChildren } from 'react';
import { ipcBridge } from '@/common';
import type { TChatConversation } from '@/common/config/storage';
import { parseConversationId, parseMessageId } from '@/common/types/ids';
import { MessageListProvider, useMessageList } from '../../Messages/hooks';
import { useNomiMessage } from './useNomiMessage';

const conversationId = parseConversationId('0190f5fe-7c00-7a00-8000-000000000971');
const messageId = parseMessageId('0190f5fe-7c00-7a00-8000-000000000972');
const turnId = parseMessageId('0190f5fe-7c00-7a00-8000-000000000973');
const wrapper = ({ children }: PropsWithChildren) => createElement(MessageListProvider, { initialValue: [] }, children);
afterEach(() => { cleanup(); mock.restore(); });

test.each([
  'USER_LLM_PROVIDER_UNAVAILABLE', 'USER_LLM_PROVIDER_BILLING_REQUIRED',
  'USER_LLM_PROVIDER_AUTH_FAILED', 'USER_LLM_PROVIDER_RATE_LIMITED',
])('%s settles from the canonical failure and makes the next input available without manual ending', async (code) => {
  let stream: Parameters<typeof ipcBridge.conversation.responseStream.on>[0] | undefined;
  spyOn(ipcBridge.conversation.responseStream, 'on').mockImplementation(listener => { stream = listener; return () => {}; });
  for (const name of ['turnStarted', 'turnCompleted', 'turnPaused', 'userCreated', 'messageAnnotated', 'reconnected'] as const) {
    spyOn(ipcBridge.conversation[name], 'on').mockImplementation(() => () => {});
  }
  let current = { id: conversationId, type: 'nomi', status: 'running',
    extra: { execution_phase: 'running' },
    runtime: { state: 'running', is_processing: true, can_send_message: false, active_turn_id: turnId },
  } as TChatConversation;
  spyOn(ipcBridge.conversation.get, 'invoke').mockImplementation(async () => current);
  const persist = spyOn(ipcBridge.conversation.update, 'invoke').mockResolvedValue(true);
  const hook = renderHook(() => ({ activity: useNomiMessage(conversationId), messages: useMessageList() }), { wrapper });
  await waitFor(() => expect(hook.result.current.activity.hasHydratedRunningState).toBe(true));
  expect(hook.result.current.activity.running).toBe(true);
  current = { ...current, status: 'pending', extra: { execution_phase: 'ready' },
    runtime: { state: 'idle', is_processing: false, can_send_message: true },
  } as TChatConversation;
  const error = { code, ownership: 'user_llm_provider', message: 'The model request failed',
    detail: 'Original provider diagnostic', retryable: false, modelName: 'admitted-model' };
  const wire = { conversation_id: conversationId, msg_id: messageId, turn_id: turnId, created_at: 1235 };
  act(() => stream!({ ...wire, type: 'error', data: error }));
  await waitFor(() => expect(hook.result.current.messages).toHaveLength(1));
  await waitFor(() => expect(hook.result.current.activity.running).toBe(false));
  expect(hook.result.current.activity.pauseNotice).toBeNull();
  expect(hook.result.current.activity.getTurnCompletionGeneration()).toBe(1);
  const failure = hook.result.current.messages[0];
  if (failure.type !== 'tips') throw new Error('expected shared failed Turn note');
  expect(failure.content.error).toEqual(error);
  expect(failure.turn_id).toBe(turnId);
  expect(failure.created_at).toBe(1235);
  expect(failure.content.execution_pause).toBeUndefined();
  expect(persist).not.toHaveBeenCalled();
});
