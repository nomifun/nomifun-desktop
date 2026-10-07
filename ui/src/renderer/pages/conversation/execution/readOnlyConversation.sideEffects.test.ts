/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { afterEach, describe, expect, mock, spyOn, test } from 'bun:test';
import { act, cleanup, renderHook, waitFor } from '@testing-library/react';
import { createElement, type PropsWithChildren } from 'react';
import { ipcBridge } from '@/common';
import { BackendHttpError } from '@/common/adapter/httpBridge';
import type { IResponseMessage } from '@/common/adapter/ipcBridge';
import { parseConversationId, parseMessageId } from '@/common/types/ids';
import { MessageListProvider, useMessageList } from '../Messages/hooks';
import { useNomiMessage } from '../platforms/nomi/useNomiMessage';

const conversationId = parseConversationId('0190f5fe-7c00-7a00-8000-000000000971');
const messageId = parseMessageId('0190f5fe-7c00-7a00-8000-000000000972');
const wrapper = ({ children }: PropsWithChildren) =>
  createElement(MessageListProvider, { initialValue: [] }, children);

afterEach(() => {
  cleanup();
  mock.restore();
});

// Exercise the actual hook and capture its transport callback; do not restate
// its authority predicates in the test. Every transcript shares this consumer.
async function mountTranscript() {
  let onStream: ((message: IResponseMessage) => void) | undefined;
  spyOn(ipcBridge.conversation.responseStream, 'on').mockImplementation((listener) => {
    onStream = listener;
    return () => { onStream = undefined; };
  });
  for (const event of [
    'turnStarted',
    'turnCompleted',
    'turnPaused',
    'userCreated',
    'messageAnnotated',
    'reconnected',
  ] as const) {
    spyOn(ipcBridge.conversation[event], 'on').mockImplementation(() => () => {});
  }
  // Missing conversation is an authoritative idle hydration, with no network.
  spyOn(ipcBridge.conversation.get, 'invoke').mockRejectedValue(new BackendHttpError({
    method: 'GET', path: '/api/conversations/fixture', status: 404, body: { code: 'NOT_FOUND' },
  }));
  const persist = spyOn(ipcBridge.conversation.update, 'invoke').mockResolvedValue(true);
  const hook = renderHook(() => ({
    runtime: useNomiMessage(conversationId),
    messages: useMessageList(),
  }), { wrapper });
  await waitFor(() => expect(hook.result.current.runtime.hasHydratedRunningState).toBe(true));
  await act(async () => { hook.result.current.runtime.setWaitingResponse(true); });
  const emit = async (message: Pick<IResponseMessage, 'type' | 'data'>) => {
    await act(async () => {
      onStream!({ ...message, conversation_id: conversationId, msg_id: messageId });
      // Drain the message-list batch timer while React is still inside act.
      await new Promise((resolve) => setTimeout(resolve, 0));
    });
  };
  return { hook, emit, persist };
}

describe('read-only execution transcript side effects', () => {
    test('live metrics render without mutating frozen Session extra', async () => {
      const { hook, emit, persist } = await mountTranscript();
      await emit({ type: 'turn_metrics', data: { input_tokens: 3, output_tokens: 5 } });
      expect(hook.result.current.runtime.tokenUsage?.total_tokens).toBe(8);
      expect(persist).not.toHaveBeenCalled();
    });

    test('renders canonical text replacements without rewriting on finish', async () => {
      const { hook, emit, persist } = await mountTranscript();
      await emit({ type: 'content', data: 'first fragment' });
      await emit({ type: 'text', data: { content: 'replacement', replace: true } });
      await emit({ type: 'finish', data: undefined });
      const answer = hook.result.current.messages.find(message => message.type === 'text');
      expect(answer?.type).toBe('text');
      if (answer?.type !== 'text') throw new Error('expected text projection');
      expect(answer.content.content).toBe('replacement');
      expect(persist).not.toHaveBeenCalled();
    });
});
