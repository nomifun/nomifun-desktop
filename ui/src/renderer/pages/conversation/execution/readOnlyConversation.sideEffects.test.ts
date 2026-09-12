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
import { MessageListProvider } from '../Messages/hooks';
import * as localCron from '../platforms/nomi/localCronCommands';
import { NomiMessageBufferStore } from '../platforms/nomi/nomiMessageBuffer';
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
// its read-only predicates in the test. Both modes receive the same live events.
async function mountTranscript(readOnly: boolean) {
  let onStream: ((message: IResponseMessage) => void) | undefined;
  spyOn(ipcBridge.conversation.responseStream, 'on').mockImplementation((listener) => {
    onStream = listener;
    return () => { onStream = undefined; };
  });
  for (const event of ['turnStarted', 'turnCompleted', 'userCreated', 'reconnected'] as const) {
    spyOn(ipcBridge.conversation[event], 'on').mockImplementation(() => () => {});
  }
  // Missing conversation is an authoritative idle hydration, with no network.
  spyOn(ipcBridge.conversation.get, 'invoke').mockRejectedValue(new BackendHttpError({
    method: 'GET', path: '/api/conversations/fixture', status: 404, body: { code: 'NOT_FOUND' },
  }));
  const persist = spyOn(ipcBridge.conversation.update, 'invoke').mockResolvedValue(true);
  const process = spyOn(localCron, 'processLocalCronResponse').mockResolvedValue({
    systemResponses: [],
  });
  const append = spyOn(NomiMessageBufferStore.prototype, 'append');
  const replace = spyOn(NomiMessageBufferStore.prototype, 'replace');
  const hook = renderHook(() => useNomiMessage(conversationId, { readOnly }), { wrapper });
  await waitFor(() => expect(hook.result.current.hasHydratedRunningState).toBe(true));
  await act(async () => { hook.result.current.setWaitingResponse(true); });
  const emit = async (message: Pick<IResponseMessage, 'type' | 'data'>) => {
    await act(async () => {
      onStream!({ ...message, conversation_id: conversationId, msg_id: messageId });
      // Drain the message-list batch timer while React is still inside act.
      await new Promise((resolve) => setTimeout(resolve, 0));
    });
  };
  return { hook, emit, persist, process, append, replace };
}

describe('read-only execution transcript side effects', () => {
  for (const readOnly of [false, true]) {
    test('readOnly=' + readOnly + ': live metrics render but only writable transcripts persist', async () => {
      const { hook, emit, persist } = await mountTranscript(readOnly);
      await emit({ type: 'turn_metrics', data: { input_tokens: 3, output_tokens: 5 } });
      expect(hook.result.current.tokenUsage?.total_tokens).toBe(8);
      expect(persist).toHaveBeenCalledTimes(readOnly ? 0 : 1);
      if (!readOnly) expect(persist.mock.calls[0]?.[0].conversation_id).toBe(conversationId);
    });

    test('readOnly=' + readOnly + ': text buffering and legacy post-process respect mode', async () => {
      const { emit, append, replace, process } = await mountTranscript(readOnly);
      await emit({ type: 'content', data: 'legacy response' });
      await emit({ type: 'text', data: { content: 'replacement', replace: true } });
      await emit({ type: 'finish', data: undefined });
      expect(append).toHaveBeenCalledTimes(readOnly ? 0 : 1);
      expect(replace).toHaveBeenCalledTimes(readOnly ? 0 : 1);
      expect(process).toHaveBeenCalledTimes(readOnly ? 0 : 1);
      if (!readOnly) expect(process).toHaveBeenCalledWith(conversationId, 'replacement');
    });
  }
});
