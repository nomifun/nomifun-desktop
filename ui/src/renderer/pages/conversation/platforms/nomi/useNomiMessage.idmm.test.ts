import { afterEach, expect, mock, spyOn, test } from 'bun:test';
import { act, cleanup, renderHook, waitFor } from '@testing-library/react';
import { createElement, type PropsWithChildren } from 'react';
import { ipcBridge } from '@/common';
import type { TChatConversation } from '@/common/config/storage';
import type { TMessage } from '@/common/chat/chatLib';
import { parseConversationId, parseMessageId } from '@/common/types/ids';
import { MessageListProvider, useMessageList } from '../../Messages/hooks';
import { useNomiMessage } from './useNomiMessage';

const conversationId = parseConversationId('0190f5fe-7c00-7a00-8000-000000000961');
const noticeId = parseMessageId('0190f5fe-7c00-7a00-8000-000000000962');
const wrapper = ({ children }: PropsWithChildren) => createElement(MessageListProvider, { initialValue: [] }, children);
afterEach(() => { cleanup(); mock.restore(); });

test('canonical annotation notice is fetched, validated and shown immediately without simulating a model Turn', async () => {
  let annotated: Parameters<typeof ipcBridge.conversation.messageAnnotated.on>[0] | undefined;
  spyOn(ipcBridge.conversation.messageAnnotated, 'on').mockImplementation(listener => { annotated = listener; return () => {}; });
  for (const name of ['turnStarted', 'turnCompleted', 'turnPaused', 'userCreated', 'responseStream', 'reconnected'] as const) {
    spyOn(ipcBridge.conversation[name], 'on').mockImplementation(() => () => {});
  }
  spyOn(ipcBridge.conversation.get, 'invoke').mockResolvedValue({ id: conversationId, type: 'nomi', status: 'finished',
    extra: { execution_phase: 'finished' }, runtime: { state: 'idle', is_processing: false, can_send_message: true } } as TChatConversation);
  const notice = { decision: { intervention_id: '0190f5fe-7c00-7a00-8000-000000000081', source: 'rule' as const,
    reason_code: 'sensitive_input_required', rationale: 'Permission needs a person.',
    question: { message_id: '0190f5fe-7c00-7a00-8000-000000000083', sequence: 5, fingerprint: 'a'.repeat(64) } },
    status: 'waiting_for_human' as const, created_at: 1234 };
  const getMessage = spyOn(ipcBridge.database.getConversationMessage, 'invoke').mockResolvedValue({
    id: 'notice-row', message_id: noticeId, msg_id: noticeId, conversation_id: conversationId, created_at: 1234,
    type: 'tips', position: 'center', content: { content: '', type: 'warning', idmm_notice: notice },
  });
  const hook = renderHook(() => ({ activity: useNomiMessage(conversationId), messages: useMessageList() }), { wrapper });
  await waitFor(() => expect(hook.result.current.activity.hasHydratedRunningState).toBe(true));
  await act(async () => { annotated!({ conversation_id: conversationId, message_id: noticeId }); });
  await waitFor(() => expect(hook.result.current.messages).toHaveLength(1));
  expect(getMessage).toHaveBeenCalledWith({ conversation_id: conversationId, message_id: noticeId });
  const row = hook.result.current.messages[0];
  expect(row.type).toBe('tips');
  if (row.type !== 'tips') throw new Error('expected canonical notice');
  expect(row.content.idmm_notice).toEqual(notice);
  expect(hook.result.current.activity.running).toBe(false);
  expect(hook.result.current.activity.pauseNotice).toBeNull();
});

test('a delayed canonical notice is ordered before newer input and scoped responses cannot enter another conversation', async () => {
  const otherConversationId = parseConversationId('0190f5fe-7c00-7a00-8000-000000000963');
  const latestId = parseMessageId('0190f5fe-7c00-7a00-8000-000000000964');
  const latest: TMessage = { id: 'newer-human', message_id: latestId, msg_id: latestId, conversation_id: conversationId,
    type: 'text', position: 'right', created_at: 30, content: { content: 'A newer request' } };
  const seeded = ({ children }: PropsWithChildren) => createElement(MessageListProvider, { initialValue: [latest] }, children);
  let annotated: Parameters<typeof ipcBridge.conversation.messageAnnotated.on>[0] | undefined;
  spyOn(ipcBridge.conversation.messageAnnotated, 'on').mockImplementation(listener => { annotated = listener; return () => {}; });
  for (const name of ['turnStarted', 'turnCompleted', 'turnPaused', 'userCreated', 'responseStream', 'reconnected'] as const) {
    spyOn(ipcBridge.conversation[name], 'on').mockImplementation(() => () => {});
  }
  spyOn(ipcBridge.conversation.get, 'invoke').mockImplementation(async ({ conversation_id }) => ({ id: conversation_id,
    type: 'nomi', status: 'finished', extra: { execution_phase: 'finished' },
    runtime: { state: 'idle', is_processing: false, can_send_message: true } } as TChatConversation));
  const responses: Array<(message: TMessage) => void> = [];
  const getMessage = spyOn(ipcBridge.database.getConversationMessage, 'invoke').mockImplementation(() =>
    new Promise<TMessage>(resolve => responses.push(resolve)));
  const notice: TMessage = { id: 'earlier-notice', message_id: noticeId, msg_id: noticeId, conversation_id: conversationId,
    type: 'tips', position: 'center', created_at: 20, content: { content: '', type: 'warning', idmm_notice: {
      decision: { intervention_id: parseMessageId('0190f5fe-7c00-7a00-8000-000000000965'), source: 'rule',
        reason_code: 'sensitive_input_required', rationale: 'Permission needs a person.', question: {
          message_id: parseMessageId('0190f5fe-7c00-7a00-8000-000000000966'), sequence: 1, fingerprint: 'a'.repeat(64),
        } }, status: 'waiting_for_human', created_at: 20,
    } } };
  const hook = renderHook(({ id }) => ({ activity: useNomiMessage(id), messages: useMessageList() }), {
    wrapper: seeded, initialProps: { id: conversationId },
  });
  await waitFor(() => expect(hook.result.current.activity.hasHydratedRunningState).toBe(true));
  act(() => annotated!({ conversation_id: conversationId, message_id: noticeId }));
  await act(async () => responses[0](notice));
  await waitFor(() => expect(hook.result.current.messages.map(row => row.id)).toEqual(['earlier-notice', 'newer-human']));
  expect(hook.result.current.activity.running).toBe(false);

  const interaction = { kind: 'desktop' as const, robot_id: 'fixture-camera', connection_id: 'fixture-link',
    request_id: 'fixture-observation', input_modality: 'text' as const, output_mode: 'desktop' as const };
  const observations = [{ question: 'What is visible?', answer: 'A fixture image.', observed_at: 20,
    image: { id: parseMessageId('0190f5fe-7c00-7a00-8000-000000000967'), path: '/fixture/image.png',
      mime_type: 'image/png', sha256: 'a'.repeat(64) } }];
  act(() => annotated!({ conversation_id: conversationId, message_id: latestId }));
  await act(async () => responses[1]({ ...latest, content: { content: 'A newer', interaction, observations } }));
  const updated = hook.result.current.messages.find(row => row.msg_id === latestId);
  expect(updated?.type).toBe('text');
  if (updated?.type !== 'text') throw new Error('expected annotated input');
  expect(updated.content.content).toBe('A newer request');
  expect(updated.content.interaction).toEqual(interaction);
  expect(updated.content.observations).toEqual(observations);

  act(() => annotated!({ conversation_id: conversationId, message_id: noticeId }));
  hook.rerender({ id: otherConversationId });
  await act(async () => responses[2]({ ...notice, id: 'stale-response' }));
  expect(hook.result.current.messages.map(row => row.id)).toEqual(['earlier-notice', 'newer-human']);
  act(() => annotated!({ conversation_id: conversationId, message_id: noticeId }));
  expect(getMessage).toHaveBeenCalledTimes(3);
  act(() => annotated!({ conversation_id: otherConversationId, message_id: noticeId }));
  await act(async () => responses[3]({ ...notice, id: 'wrong-scope-response' }));
  expect(hook.result.current.messages.map(row => row.id)).toEqual(['earlier-notice', 'newer-human']);
});
