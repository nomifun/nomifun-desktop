import { expect, test } from 'bun:test';
import { composeMessage, transformUserCreatedEvent, type IMessageText } from '@/common/chat/chatLib';
import { parseConversationId, parseMessageId } from '@/common/types/ids';
import { composeMessageForTest, mergeFetchedMessagesForConversation, normalizeDbMessage } from './hooks';

const conversation = parseConversationId('0190f5fe-7c00-7a00-8000-000000000051');
const message = parseMessageId('0190f5fe-7c00-7a00-8000-000000000052');
const base: IMessageText = { id: 'original-render-row', type: 'text', position: 'right', conversation_id: conversation,
  msg_id: message, message_id: message, created_at: 1, content: { content: 'Keep this complete user body', input_delivery: { state: 'pending', event_seq: 10 } } };
const later: IMessageText = { ...base, id: 'later-assistant', msg_id: parseMessageId('0190f5fe-7c00-7a00-8000-000000000053'),
  message_id: undefined, position: 'left', content: { content: 'Working' } };

for (const [name, merge] of [['indexed Messages hook', composeMessageForTest], ['shared composer', composeMessage]] as const) {
  test(`${name} applies repeated userCreated delivery facts without duplicating or concatenating the body`, () => {
    let rows = [base, later];
    for (const input_delivery of [
      { state: 'blocked' as const, reason: 'SKILL_UNAVAILABLE', event_seq: 11 },
      { state: 'pending' as const, event_seq: 12 },
      { state: 'claimed' as const, event_seq: 13 },
      { state: 'pending' as const, event_seq: 10 },
    ]) {
      const event = transformUserCreatedEvent({ conversation_id: conversation, msg_id: message,
        content: base.content.content, input_delivery, position: 'right', status: 'finish', created_at: 999 }, conversation)!;
      rows = merge(event, rows) as IMessageText[];
      expect(rows).toHaveLength(2);
      expect(rows[0].content.content).toBe(base.content.content);
      expect(rows[0].id).toBe(base.id);
      expect(rows[0].created_at).toBe(1);
      expect(rows[0].content.input_delivery?.event_seq).toBe(Math.max(input_delivery.event_seq, rows[0].content.input_delivery!.event_seq!));
    }
    expect(rows[0].content.input_delivery).toEqual({ state: 'claimed', event_seq: 13 });
    const history = normalizeDbMessage(base) as IMessageText;
    const merged = mergeFetchedMessagesForConversation(rows, [history], conversation).find(row => row.msg_id === message);
    expect(merged?.type).toBe('text');
    if (merged?.type !== 'text') throw new Error('original user row was lost');
    expect(merged.content.input_delivery).toEqual({ state: 'claimed', event_seq: 13 });
  });
}

test('context-clear notification retires an old pending hint without reporting execution failure', () => {
  const event = transformUserCreatedEvent({ conversation_id: conversation, msg_id: message,
    content: base.content.content, input_delivery: { state: 'discarded', reason: 'CONTEXT_CLEARED', event_seq: 30 },
    position: 'right', status: 'finish', created_at: 1 }, conversation)!;
  const [updated] = composeMessageForTest(event, [base]) as IMessageText[];
  expect(updated.content.input_delivery).toEqual({ state: 'discarded', reason: 'CONTEXT_CLEARED', event_seq: 30 });
  expect(updated.status).toBe('finish');
});
