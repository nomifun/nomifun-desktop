import { afterEach, expect, mock, spyOn, test } from 'bun:test';
import { act, cleanup, renderHook, waitFor } from '@testing-library/react';
import { createElement, type PropsWithChildren } from 'react';
import { ipcBridge } from '@/common';
import type { TChatConversation } from '@/common/config/storage';
import { parseConversationId, parseMessageId } from '@/common/types/ids';
import { MessageListProvider } from '../../Messages/hooks';
import { useNomiMessage } from './useNomiMessage';

const conversationId = parseConversationId('0190f5fe-7c00-7a00-8000-000000000981');
const turnId = parseMessageId('0190f5fe-7c00-7a00-8000-000000000982');
const foreignTurnId = parseMessageId('0190f5fe-7c00-7a00-8000-000000000983');
const wrapper = ({ children }: PropsWithChildren) => createElement(MessageListProvider, { initialValue: [] }, children);
const paused = {
  status: 'running', extra: { workspace: '/fixture', execution_phase: 'paused', execution_pause: {
    reason: 'EXECUTION_MODEL_PROVIDER_UNAVAILABLE', cleanup_proven: true, paused_at_ms: 123,
  } },
  runtime: { state: 'idle', has_runtime: false, is_processing: false, can_send_message: false, active_turn_id: turnId },
} as TChatConversation;
const processing = { ...paused, extra: { ...paused.extra, execution_phase: 'running' },
  runtime: { ...paused.runtime, state: 'running', is_processing: true } } as TChatConversation;

afterEach(() => { cleanup(); mock.restore(); });

async function mount(initial: TChatConversation) {
  let current = initial;
  let notifyPause: Parameters<typeof ipcBridge.conversation.turnPaused.on>[0] | undefined;
  let notifyStream: Parameters<typeof ipcBridge.conversation.responseStream.on>[0] | undefined;
  spyOn(ipcBridge.conversation.turnPaused, 'on').mockImplementation(listener => { notifyPause = listener; return () => {}; });
  spyOn(ipcBridge.conversation.responseStream, 'on').mockImplementation(listener => { notifyStream = listener; return () => {}; });
  for (const name of ['turnStarted', 'turnCompleted', 'userCreated', 'messageAnnotated', 'reconnected'] as const) {
    spyOn(ipcBridge.conversation[name], 'on').mockImplementation(() => () => {});
  }
  const get = spyOn(ipcBridge.conversation.get, 'invoke').mockImplementation(async () => current);
  const persist = spyOn(ipcBridge.conversation.update, 'invoke').mockResolvedValue(true);
  const hook = renderHook(() => useNomiMessage(conversationId), { wrapper });
  await waitFor(() => expect(hook.result.current.hasHydratedRunningState).toBe(true));
  return { hook, get, persist, project: (value: TChatConversation) => { current = value; },
    pause: async (id = turnId) => { await act(async () => { notifyPause!({ conversation_id: conversationId, turn_id: id }); }); },
    lateContent: async () => { await act(async () => {
      notifyStream!({ conversation_id: conversationId, turn_id: turnId, msg_id: turnId, type: 'content', data: { content: 'late fragment' } });
      await new Promise(resolve => setTimeout(resolve, 0));
    }); },
  };
}

test('live pause verifies the canonical owner, stops activity and fences late stream activity', async () => {
  const run = await mount(processing);
  expect(run.hook.result.current.running).toBe(true);
  const completions = run.hook.result.current.getTurnCompletionGeneration();
  const before = run.get.mock.calls.length;
  await run.pause(foreignTurnId);
  expect(run.get.mock.calls.length).toBe(before);
  expect(run.hook.result.current.running).toBe(true);
  run.project(paused);
  await run.pause();
  await waitFor(() => expect(run.hook.result.current.pauseNotice?.turnId).toBe(turnId));
  expect(run.hook.result.current.running).toBe(false);
  expect(run.hook.result.current.getTurnCompletionGeneration()).toBe(completions);
  await run.lateContent();
  expect(run.hook.result.current.running).toBe(false);
  expect(run.hook.result.current.pauseNotice?.reason).toBe('EXECUTION_MODEL_PROVIDER_UNAVAILABLE');
  expect(run.persist).not.toHaveBeenCalled();
});

test('cold paused hydration shows the cause without completing or rewriting the original turn', async () => {
  const run = await mount(paused);
  expect(run.hook.result.current.running).toBe(false);
  expect(run.hook.result.current.pauseNotice).toEqual({ turnId,
    reason: 'EXECUTION_MODEL_PROVIDER_UNAVAILABLE', cleanupProven: true, pausedAt: 123 });
  expect(run.hook.result.current.getTurnCompletionGeneration()).toBe(0);
  expect(run.persist).not.toHaveBeenCalled();
});
