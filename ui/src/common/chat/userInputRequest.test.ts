import { expect, test } from 'bun:test';
import type { IResponseMessage } from '@/common/adapter/ipcBridge';
import { parseConversationId, parseMessageId } from '@/common/types/ids';
import { normalizeDbMessage } from '@/renderer/pages/conversation/Messages/hooks';
import { agentUserInputRequestText, isUserInputRequestPresentation, normalizeAgentUserInputRequest, transformMessage } from './chatLib';

const conversationId = parseConversationId('0190f5fe-7c00-7a00-8000-000000000951');
const turnId = parseMessageId('0190f5fe-7c00-7a00-8000-000000000952');
const messageId = parseMessageId('0190f5fe-7c00-7a00-8000-000000000953');
const request = { request_id: 'ask:choose-path', question: 'Which directory should I use?', remaining_work: 'Write and verify the report.' };
const wire: IResponseMessage = { conversation_id: conversationId, turn_id: turnId, msg_id: messageId,
  type: 'tips', created_at: 2000, stream_complete: true,
  data: { type: 'warning', content: '', started_at_ms: 1000, finished_at_ms: 2000, user_input_request: request } };

test('live and reopened history keep the exact typed question and original Turn timing', () => {
  expect(isUserInputRequestPresentation(wire)).toBe(true);
  const live = transformMessage(wire);
  if (live?.type !== 'tips') throw new Error('expected a handoff tip');
  expect(live.content).toMatchObject({ user_input_request: request, started_at_ms: 1000, finished_at_ms: 2000 });
  expect(live.content.error).toBeUndefined();
  const history = normalizeDbMessage({ ...live, message_id: messageId });
  expect(history).toMatchObject({ turn_id: turnId, message_id: messageId, content: live.content });
});

test('question words and missing host identities do not create an input handoff', () => {
  const plain = { ...wire, data: { type: 'warning', content: 'Which directory should I use?' } };
  expect(isUserInputRequestPresentation(plain)).toBe(false);
  for (const message of [{ ...wire, turn_id: undefined }, plain]) {
    expect(transformMessage(message)).not.toHaveProperty('content.user_input_request');
  }
  expect(isUserInputRequestPresentation({ ...wire, data: { type: 'error', content: '', user_input_request: request } })).toBe(false);
});

test('handoff JSON uses the same UTF-8 and control-character limits as the host', () => {
  expect(normalizeAgentUserInputRequest({ ...request, question: 'Choose:\n\tone option' })?.question).toBe('Choose:\n\tone option');
  for (const value of [{ ...request, request_id: 'x'.repeat(257) }, { ...request, question: '界'.repeat(5462) },
    { ...request, question: '\rChoose' }, { ...request, remaining_work: ' ' }, { ...request, question: '' },
    { ...request, guessed_status: 'paused' }]) expect(normalizeAgentUserInputRequest(value)).toBeUndefined();
});

test('plain reply consumers format only a validated typed request without inferring a question from prose', () => {
  expect(agentUserInputRequestText(request)).toBe(`${request.question}\n\n${request.remaining_work}`);
  expect(agentUserInputRequestText(request.question)).toBeUndefined();
  expect(agentUserInputRequestText({ ...request, remaining_work: '' })).toBeUndefined();
});
