import { afterEach, expect, mock, spyOn, test } from 'bun:test';
import { act, cleanup, renderHook, waitFor } from '@testing-library/react';
import { createElement, type PropsWithChildren } from 'react';
import { ipcBridge } from '@/common';
import type { TChatConversation } from '@/common/config/storage';
import type { IResponseMessage } from '@/common/adapter/ipcBridge';
import { parseConversationId, parseMessageId } from '@/common/types/ids';
import { MessageListProvider, useMessageList } from '../../Messages/hooks';
import { useNomiMessage } from './useNomiMessage';

const conversationId = parseConversationId('0190f5fe-7c00-7a00-8000-000000000941');
const sourceTurnId = parseMessageId('0190f5fe-7c00-7a00-8000-000000000942');
const noteId = parseMessageId('0190f5fe-7c00-7a00-8000-000000000943');
const nextTurnId = parseMessageId('0190f5fe-7c00-7a00-8000-000000000944');
const request = { request_id: 'ask:report-path', question: 'Where should the report be saved?', remaining_work: 'Save and verify the report.' };
const wrapper = ({ children }: PropsWithChildren) => createElement(MessageListProvider, { initialValue: [] }, children);
afterEach(() => { cleanup(); mock.restore(); });

test.each(['idle', 'awaiting admission', 'newer Turn running', 'source Turn finishes'] as const)(
  '%s: input handoff remains an original-Turn note and only authority releases activity', async scenario => {
    let stream!: (message: IResponseMessage) => void;
    spyOn(ipcBridge.conversation.responseStream, 'on').mockImplementation(listener => { stream = listener; return () => {}; });
    for (const name of ['turnStarted', 'turnCompleted', 'turnPaused', 'userCreated', 'messageAnnotated', 'reconnected'] as const) {
      spyOn(ipcBridge.conversation[name], 'on').mockImplementation(() => () => {});
    }
    const activeTurnId = scenario === 'newer Turn running' ? nextTurnId : scenario === 'source Turn finishes' ? sourceTurnId : undefined;
    const snapshot = (active?: typeof sourceTurnId): TChatConversation => ({ id: conversationId, type: 'nomi',
      status: active ? 'running' : 'finished', extra: { workspace: '',  },
      runtime: { state: active ? 'running' : 'idle', is_processing: Boolean(active),
        can_send_message: !active, active_turn_id: active,
        ...(active ? { processing_started_at: 1000 } : {}),
      },
    } as TChatConversation);
    let current = snapshot(activeTurnId);
    spyOn(ipcBridge.conversation.get, 'invoke').mockImplementation(async () => current);
    const hook = renderHook(() => ({ activity: useNomiMessage(conversationId), messages: useMessageList() }), { wrapper });
    await waitFor(() => expect(hook.result.current.activity.hasHydratedRunningState).toBe(true));
    if (scenario === 'awaiting admission') act(() => hook.result.current.activity.setWaitingResponse(true));
    const previousGeneration = hook.result.current.activity.getTurnCompletionGeneration();
    const frame: IResponseMessage = { conversation_id: conversationId, turn_id: sourceTurnId, msg_id: noteId,
      type: 'tips', created_at: 2000, stream_complete: true,
      data: { type: 'warning', content: '', user_input_request: request, started_at_ms: 1000, finished_at_ms: 2000 } };
    act(() => stream(frame));
    await waitFor(() => expect(hook.result.current.messages).toHaveLength(1));
    // Even a typed handoff cannot substitute for release of the exact owner.
    expect(hook.result.current.activity.running).toBe(scenario !== 'idle');
    expect(hook.result.current.activity.getTurnCompletionGeneration()).toBe(previousGeneration);
    if (scenario === 'source Turn finishes') {
      current = snapshot();
      act(() => stream(frame));
      await waitFor(() => expect(hook.result.current.activity.running).toBe(false));
      expect(hook.result.current.activity.getTurnCompletionGeneration()).toBe(previousGeneration + 1);
    }
    expect(hook.result.current.activity.pauseNotice).toBeNull();
    expect(hook.result.current.messages[0]).toMatchObject({ turn_id: sourceTurnId,
      content: { type: 'warning', user_input_request: request, finished_at_ms: 2000 } });
    expect(hook.result.current.messages[0]).not.toHaveProperty('content.error');
    if (scenario === 'newer Turn running') expect(hook.result.current.activity.activeTurnId).toBe(nextTurnId);
  },
);
