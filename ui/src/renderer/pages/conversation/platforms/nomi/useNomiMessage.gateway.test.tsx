/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */
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

test('paused gateway account System notice remains visible through FinishPaused without reopening or persisting the Turn', async () => {
  let stream: Parameters<typeof ipcBridge.conversation.responseStream.on>[0] | undefined;
  spyOn(ipcBridge.conversation.responseStream, 'on').mockImplementation((listener) => { stream = listener; return () => {}; });
  for (const name of ['turnStarted', 'turnCompleted', 'turnPaused', 'userCreated', 'messageAnnotated', 'reconnected'] as const) {
    spyOn(ipcBridge.conversation[name], 'on').mockImplementation(() => () => {});
  }
  spyOn(ipcBridge.conversation.get, 'invoke').mockResolvedValue({ id: conversationId, type: 'nomi', status: 'running',
    extra: { execution_phase: 'paused', execution_pause: { reason: 'PROVIDER_BALANCE_REQUIRED', cleanup_proven: true, paused_at_ms: 1234 } },
    runtime: { state: 'idle', is_processing: false, can_send_message: false, active_turn_id: turnId },
  } as TChatConversation);
  const hook = renderHook(() => ({ activity: useNomiMessage(conversationId), messages: useMessageList() }), { wrapper });
  await waitFor(() => expect(hook.result.current.activity.hasHydratedRunningState).toBe(true));
  expect(hook.result.current.activity.pauseNotice?.cleanupProven).toBe(true);
  expect(hook.result.current.activity.running).toBe(false);
  const wire = { conversation_id: conversationId, msg_id: messageId, turn_id: turnId, created_at: 1235 };
  act(() => stream!({ ...wire, type: 'system', data: { kind: 'cron_response', message: 'ordinary system data' } }));
  expect(hook.result.current.messages).toHaveLength(0);
  const error = { code: 'USER_LLM_PROVIDER_BILLING_REQUIRED', ownership: 'user_llm_provider',
    message: 'The model gateway balance is insufficient. Top up the account to continue.', retryable: false,
    resolution: { kind: 'check_provider_billing', target: 'provider_settings' },
  };
  act(() => stream!({ ...wire, type: 'system', data: { kind: 'model_gateway_account_action', error } }));
  await waitFor(() => expect(hook.result.current.messages).toHaveLength(1));
  const notice = hook.result.current.messages[0];
  expect(notice.type).toBe('tips');
  if (notice.type !== 'tips') throw new Error('expected transient account notice');
  expect(notice.content.error).toEqual(error);
  expect(notice.message_id).toBeUndefined();
  expect(hook.result.current.activity.running).toBe(false);
  expect(hook.result.current.activity.pauseNotice?.turnId).toBe(turnId);
  act(() => stream!({ ...wire, type: 'finish', data: { paused: true } }));
  await act(async () => { await Promise.resolve(); });
  expect(hook.result.current.messages).toHaveLength(1);
  expect(hook.result.current.messages[0]).toBe(notice);
  expect(hook.result.current.activity.running).toBe(false);
  expect(hook.result.current.activity.pauseNotice?.turnId).toBe(turnId);
});
