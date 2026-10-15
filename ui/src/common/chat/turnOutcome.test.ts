import { describe, expect, test } from 'bun:test';
import { parseConversationId, parseMessageId } from '../types/ids';
import { normalizeAgentTurnOutcome } from './turnOutcome';
import { normalizeAgentStatusContent, transformMessage, mergeAgentStatusContent, mergeTipsContent } from './chatLib';
import { normalizeDbMessage } from '@/renderer/pages/conversation/Messages/hooks';
const message = parseMessageId('019f0000-0000-7000-8000-000000000901');
const session = parseConversationId('019f0000-0000-7000-8000-000000000902');

describe('canonical public Turn result boundary', () => {
  test('validates exact identities and never reconstructs an outcome from text or status', () => {
    expect(normalizeAgentTurnOutcome({ kind: 'answered', message_ids: [message] })).toEqual({ kind: 'answered', message_ids: [message] });
    expect(normalizeAgentTurnOutcome({ kind: 'failed', message_ids: [] })).toEqual({ kind: 'failed', message_ids: [] });
    expect(normalizeAgentTurnOutcome({ kind: 'input_required', request_id: 'ask:profile' })).toEqual({ kind: 'input_required', request_id: 'ask:profile' });
    for (const value of [{ kind: 'answered', message_ids: [] }, { kind: 'answered', message_ids: [message, message] },
      { kind: 'answered', message_ids: ['bad'] }, { kind: 'input_required', request_id: '' }, { kind: 'failed' },
      { kind: 'unavailable', content: 'Success' }, { status: 'completed', content: 'Done' }]) {
      expect(normalizeAgentTurnOutcome(value)).toBeUndefined();
    }
  });

  test('live and reopened handoff receipts normalize to the same typed content', () => {
    const request = { request_id: 'ask:profile', question: '姓名？', remaining_work: '生成演示文稿。' };
    const content = { type: 'warning', content: '', user_input_request: request,
      turn_outcome: { kind: 'input_required', request_id: request.request_id }, started_at_ms: 1000, finished_at_ms: 2000 };
    const live = transformMessage({ type: 'tips', conversation_id: session, turn_id: message, msg_id: message, data: content })!;
    const history = normalizeDbMessage({ id: 'row', conversation_id: session, message_id: message, msg_id: message,
      turn_id: message, type: 'tips', content } as any);
    expect(live.content).toEqual(history.content);
  });

  test('late process/raw transport frames cannot retract a committed result receipt', () => {
    const receipt = normalizeAgentStatusContent({ backend: 'nomi', status: 'prepared', turn_summary: true,
      turn_state: 'completed', turn_outcome: { kind: 'answered', message_ids: [message] } });
    expect(mergeAgentStatusContent(receipt, normalizeAgentStatusContent({ backend: 'nomi', status: 'preparing' }))).toEqual(receipt);
    const note = { type: 'warning' as const, content: '', turn_outcome: { kind: 'input_required' as const, request_id: 'ask:profile' } };
    expect(mergeTipsContent(note, { type: 'warning', content: '' })).toEqual(note);
  });
});
