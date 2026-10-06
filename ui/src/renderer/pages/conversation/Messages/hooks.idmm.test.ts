import { expect, test } from 'bun:test';
import { transformMessage, transformUserCreatedEvent, preferTextMessageVersion, type IMessageText, type IMessageTips } from '@/common/chat/chatLib';
import { parseConversationId, parseMessageId } from '@/common/types/ids';
import type { IdmmDecisionExplanation } from '@/common/types/idmm';
import { createStoredMessageMapper } from '@/common/adapter/storedMessageMapper';
import { composeMessageForTest, normalizeDbMessage, mergeFetchedMessagesForConversation } from './hooks';
import { getConversationInputHistory } from '@/renderer/utils/chat/messageHistory';

const conversationId = parseConversationId('0190f5fe-7c00-7a00-8000-000000000051');
const messageId = parseMessageId('0190f5fe-7c00-7a00-8000-000000000052');
const decision: IdmmDecisionExplanation = { intervention_id: '0190f5fe-7c00-7a00-8000-000000000081', source: 'rule',
  reason_code: 'rule_selected_safe_option', rationale: 'The recommended option is safe.',
  question: { message_id: '0190f5fe-7c00-7a00-8000-000000000083', sequence: 5, fingerprint: 'a'.repeat(64) } };
const base = { id: 'reply', type: 'text' as const, position: 'right' as const, conversation_id: conversationId,
  message_id: messageId, msg_id: messageId, created_at: 1, content: { content: '2' } };

test('live automatic replies preserve typed explanation without guessing from origin', () => {
  const event = { conversation_id: conversationId, msg_id: messageId, content: '2', position: 'right' as const,
    status: 'finish', created_at: 1, origin: 'idmm' };
  expect(transformUserCreatedEvent(event, conversationId)?.content.idmm_decision).toBeUndefined();
  const live = transformUserCreatedEvent({ ...event, idmm_decision: decision }, conversationId)!;
  expect(live.position).toBe('right');
  expect(live.content).toEqual({ content: '2', idmm_decision: decision });
  expect(composeMessageForTest(live, [base])[0].content).toEqual(live.content);
});

test('canonical history hydration preserves the explanation with a longer live body', () => {
  const mapped = createStoredMessageMapper(() => 'saved')({ message_id: messageId, conversation_id: conversationId,
    msg_id: messageId, type: 'text', position: 'right', status: 'finish', hidden: false, created_at: 1,
    content: { content: '2', idmm_decision: decision } });
  const saved = normalizeDbMessage(mapped) as IMessageText;
  const live: IMessageText = { ...base, content: { content: '2 longer live body' } };
  expect(preferTextMessageVersion(saved, live).content.idmm_decision).toEqual(decision);
  expect(mergeFetchedMessagesForConversation([live], [saved], conversationId)[0].content).toEqual({
    content: '2 longer live body', idmm_decision: decision,
  });
  expect(getConversationInputHistory([saved], conversationId)).toEqual([]);
  expect(getConversationInputHistory([base], conversationId)).toEqual(['2']);
});

test('live and stored notices retain the same typed historical outcome', () => {
  const notice = { decision, status: 'waiting_for_human' as const, created_at: 1234 };
  const data = { content: '', type: 'warning', idmm_notice: notice };
  const live = transformMessage({ type: 'tips', conversation_id: conversationId, msg_id: messageId, data }) as IMessageTips;
  expect(live.content.idmm_notice).toEqual(notice);
  const saved = normalizeDbMessage({ ...base, type: 'tips', position: 'center', content: data } as IMessageTips) as IMessageTips;
  expect(saved.content.idmm_notice).toEqual(notice);
});
