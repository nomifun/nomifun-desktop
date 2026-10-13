import { afterEach, expect, mock, spyOn, test } from 'bun:test';
import { act, cleanup, renderHook, waitFor } from '@testing-library/react';
import { createElement, type PropsWithChildren } from 'react';
import { ipcBridge } from '@/common';
import type { TChatConversation } from '@/common/config/storage';
import type { IResponseMessage } from '@/common/adapter/ipcBridge';
import { parseConversationId, parseKnowledgeBaseId, parseMessageId } from '@/common/types/ids';
import { MessageListProvider, useMessageList } from '../../Messages/hooks';
import { useNomiMessage } from './useNomiMessage';

const conversationId = parseConversationId('0190f5fe-7c00-7a00-8000-000000000961');
const originalTurnId = parseMessageId('0190f5fe-7c00-7a00-8000-000000000962');
const summaryId = parseMessageId('0190f5fe-7c00-7a00-8000-000000000963');
const nextTurnId = parseMessageId('0190f5fe-7c00-7a00-8000-000000000964');
const kb = parseKnowledgeBaseId('0190f5fe-7c00-7a00-8000-000000000965');
const wrapper = ({ children }: PropsWithChildren) => createElement(MessageListProvider, { initialValue: [] }, children);
afterEach(() => { cleanup(); mock.restore(); });

test.each(['idle', 'awaiting admission', 'original Turn release pending', 'next Turn running'] as const)(
  '%s: write-back progress and late results update the original receipt without changing Turn activity', async scenario => {
    let stream!: (message: IResponseMessage) => void;
    spyOn(ipcBridge.conversation.responseStream, 'on').mockImplementation(listener => { stream = listener; return () => {}; });
    for (const name of ['turnStarted', 'turnCompleted', 'turnPaused', 'userCreated', 'messageAnnotated', 'reconnected'] as const) {
      spyOn(ipcBridge.conversation[name], 'on').mockImplementation(() => () => {});
    }
    const activeTurnId = scenario === 'original Turn release pending' ? originalTurnId
      : scenario === 'next Turn running' ? nextTurnId : undefined;
    spyOn(ipcBridge.conversation.get, 'invoke').mockResolvedValue({ id: conversationId, type: 'nomi',
      status: activeTurnId ? 'running' : 'finished', runtime: {
        state: activeTurnId ? 'running' : 'idle', is_processing: Boolean(activeTurnId),
        can_send_message: !activeTurnId, has_runtime: Boolean(activeTurnId), active_turn_id: activeTurnId,
        ...(activeTurnId ? { processing_started_at: 3000 } : {}),
      },
    } as TChatConversation);
    const hook = renderHook(() => ({ activity: useNomiMessage(conversationId), messages: useMessageList() }), { wrapper });
    await waitFor(() => expect(hook.result.current.activity.hasHydratedRunningState).toBe(true));
    if (scenario === 'awaiting admission') act(() => hook.result.current.activity.setWaitingResponse(true));
    const running = hook.result.current.activity.running;
    const completionGeneration = hook.result.current.activity.getTurnCompletionGeneration();
    const startedAt = hook.result.current.activity.activeTurnStartedAt;
    const outcome = scenario === 'next Turn running' ? 'failed' : 'written';
    const receipt = (status: 'running' | 'written' | 'failed', event_seq: number): IResponseMessage => ({
      conversation_id: conversationId, msg_id: summaryId, turn_id: originalTurnId,
      type: 'agent_status', stream_complete: true, created_at: 1000,
      data: { backend: 'nomi', status: 'prepared', turn_summary: true, turn_state: 'completed',
        started_at_ms: 1000, finished_at_ms: 2000, knowledge_writeback: { status, event_seq,
          written: status === 'written' ? [{ kb_id: kb, rel_path: 'notes.md' }] : [],
          ...(status === 'running' ? {} : { diagnostic: {
            stage: status === 'failed' ? 'first_frame' : 'write',
            ...(status === 'failed' ? { code: 'USER_LLM_PROVIDER_TIMEOUT', provider_diagnostic: { reason: 'request_timeout' } } : {}),
            elapsed_ms: 1200, received_frames: status === 'failed' ? 0 : 3, received_bytes: status === 'failed' ? 0 : 400,
            output_bytes: status === 'failed' ? 0 : 200, input_tokens_estimate: 3000, output_token_limit: 1000,
          } }),
        } },
    });
    act(() => stream(receipt('running', 11)));
    await waitFor(() => expect(hook.result.current.messages).toHaveLength(1));
    act(() => { stream(receipt(outcome, 12)); stream(receipt('running', 11)); });
    await waitFor(() => expect(hook.result.current.messages[0]).toMatchObject({
      turn_id: originalTurnId, content: { knowledge_writeback: { status: outcome, event_seq: 12,
        diagnostic: { stage: outcome === 'failed' ? 'first_frame' : 'write', elapsed_ms: 1200, input_tokens_estimate: 3000 } } },
    }));
    expect(hook.result.current.messages).toHaveLength(1);
    expect(hook.result.current.activity.running).toBe(running);
    expect(hook.result.current.activity.activeTurnId).toBe(activeTurnId);
    expect(hook.result.current.activity.activeTurnStartedAt).toBe(startedAt);
    expect(hook.result.current.activity.getTurnCompletionGeneration()).toBe(completionGeneration);
  },
);
