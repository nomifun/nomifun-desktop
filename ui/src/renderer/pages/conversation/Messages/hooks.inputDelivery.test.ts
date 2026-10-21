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
  test(`${name} keeps the chosen exact Turn and steering position across stale ACK, notification and history`, () => {
    const target = parseMessageId('0190f5fe-7c00-7a00-8000-000000000061');
    const staleTarget = parseMessageId('0190f5fe-7c00-7a00-8000-000000000062');
    const delivered: IMessageText = { ...base, turn_id: target, content: { ...base.content,
      input_delivery: { state: 'claimed', kind: 'steering', placement: 'transcript', turn_id: target, order_seq: 30, event_seq: 40 } } };
    const stale: IMessageText = { ...base, turn_id: staleTarget, content: { ...base.content,
      input_delivery: { state: 'pending', kind: 'steering', placement: 'steering', turn_id: staleTarget, order_seq: 20, event_seq: 20 } } };
    let rows = merge(delivered, [base, later]);
    rows = merge(stale, rows);
    rows = merge({ ...base, content: { ...base.content, input_delivery: { state: 'pending' } } }, rows);
    rows = mergeFetchedMessagesForConversation(rows, [normalizeDbMessage(stale)], conversation);
    const original = rows.find(row => row.msg_id === message);
    expect(original?.type).toBe('text');
    if (original?.type !== 'text') throw new Error('original input was lost');
    expect(original.content.input_delivery).toEqual(delivered.content.input_delivery);
    expect(original.turn_id).toBe(target);
    expect(rows.filter(row => row.msg_id === message)).toHaveLength(1);
  });
}

for (const [name, merge] of [['indexed Messages hook', composeMessageForTest], ['shared composer', composeMessage]] as const) {
  test(`${name} clears a prior steering owner when the unclaimed input becomes an independent root`, () => {
    const originalTurn = parseMessageId('0190f5fe-7c00-7a00-8000-000000000059');
    const steering: IMessageText = { ...base, turn_id: originalTurn, content: { ...base.content,
      input_delivery: { state: 'pending', kind: 'steering', placement: 'steering', turn_id: originalTurn, event_seq: 20, order_seq: 20 } } };
    const resumed: IMessageText = { ...base, content: { ...base.content,
      input_delivery: { state: 'pending', kind: 'root', placement: 'transcript', event_seq: 30, order_seq: 10 } } };
    const rows = merge(resumed, [steering]) as IMessageText[];
    expect(rows[0].turn_id).toBeUndefined();
    const staleHistory = mergeFetchedMessagesForConversation(rows, [normalizeDbMessage(steering)], conversation);
    expect(staleHistory[0].turn_id).toBeUndefined();
    expect(staleHistory[0].type === 'text' && staleHistory[0].content.input_delivery?.kind).toBe('root');
  });
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

for (const [name, merge] of [['indexed Messages hook', composeMessageForTest], ['shared composer', composeMessage]] as const) {
  test(`${name} keeps owner cancellation on its exact accepted row across stale echoes and history`, () => {
    const delivery = { state: 'cancelled' as const, reason: 'CANCELLED_BY_OWNER', event_seq: 30 };
    const event = transformUserCreatedEvent({ conversation_id: conversation, msg_id: message,
      content: base.content.content, input_delivery: delivery, position: 'right', status: 'finish', created_at: 1 }, conversation)!;
    let rows = merge(event, [base, later]) as IMessageText[];
    rows = merge({ ...base, id: 'late-local-echo', content: { ...base.content, input_delivery: { state: 'pending' } } }, rows) as IMessageText[];
    rows = mergeFetchedMessagesForConversation(rows, [normalizeDbMessage(base)], conversation) as IMessageText[];
    expect(rows).toHaveLength(2);
    expect(rows[0]).toMatchObject({ id: base.id, msg_id: message, content: { content: base.content.content, input_delivery: delivery } });
    expect(rows[1]).toEqual(later);
  });
}
