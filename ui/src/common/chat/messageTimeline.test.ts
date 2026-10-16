import { describe, expect, test } from 'bun:test';
import { applyCanonicalTimeline, normalizeMessageTimeline } from './messageTimeline';
import { mergeToolCallContent, normalizeToolCallContent, type IMessageText, type IMessageToolCall } from './chatLib';
import { composeMessageForTest, drainPendingMessageUpdates, mergeFetchedMessagesForConversation } from '@/renderer/pages/conversation/Messages/hooks';
import { parseConversationId, parseMessageId } from '../types/ids';

const conversation = parseConversationId('0190f5fe-7c00-7000-8000-000000000001');
const root = parseMessageId('0190f5fe-7c00-7000-8000-000000000002');
const source = parseMessageId('0190f5fe-7c00-7000-8000-000000000003');
const timeline = (through_seq = 10) => ({ through_seq, segments: [{ segment_id: 'part:1',
  group_id: `process:${root}`, first_seq: 3, last_seq: 5, kind: 'narration' as const,
  state: 'completed' as const, start_utf16: 0, end_utf16: 3 }] });
const text: IMessageText = { id: 'stable-render-key', msg_id: source, turn_id: root,
  conversation_id: conversation, type: 'text', position: 'left', content: { content: '中文🙂' } };

describe('canonical timeline notifications', () => {
  test('updates references without replacing source content or local identity', () => {
    const rows = applyCanonicalTimeline([text], { type: 'canonical_timeline', conversation_id: conversation,
      turn_id: root, updates: [{ source_message_id: source, timeline: timeline() }] });
    expect(rows[0].id).toBe(text.id);
    expect(rows[0].type === 'text' && rows[0].content.content).toBe('中文🙂');
    expect(rows[0].content.timeline?.through_seq).toBe(10);
    expect(applyCanonicalTimeline(rows, { type: 'canonical_timeline', conversation_id: conversation,
      turn_id: root, updates: [{ source_message_id: source, timeline: timeline(9) }] })).toBe(rows);
  });

  test('same call ID in another Turn cannot be updated', () => {
    const tool: IMessageToolCall = { ...text, type: 'tool_call', content: { call_id: 'shared', name: 'read_file' } };
    const other = { ...tool, id: 'other', turn_id: source };
    const metadata = { through_seq: 10, segments: [{ segment_id: 'tool:start', group_id: `process:${root}`,
      first_seq: 3, last_seq: 8, kind: 'tool', state: 'completed', call_id: 'shared' }] };
    const rows = applyCanonicalTimeline([tool, other], { type: 'canonical_timeline', conversation_id: conversation,
      turn_id: root, updates: [{ source_message_id: source, call_id: 'shared', timeline: metadata }] });
    expect(rows[0].content.timeline).toBeDefined();
    expect(rows[1]).toBe(other);
  });

  test('source accumulates through interleaving while reference updates share the same batch', () => {
    const thinking = { ...text, id: 'reasoning', type: 'thinking' as const, msg_id: root,
      content: { content: 'reason', status: 'thinking' as const } };
    let rows = composeMessageForTest(text, []);
    rows = composeMessageForTest(thinking, rows);
    rows = composeMessageForTest({ ...text, content: { content: '继续' } }, rows);
    expect(rows.filter(row => row.type === 'text')).toHaveLength(1);
    expect(rows[0].type === 'text' && rows[0].content.content).toBe('中文🙂继续');
    const pending = { current: [{ add: false, message: { type: 'canonical_timeline' as const,
      conversation_id: conversation, turn_id: root, updates: [{ source_message_id: source, timeline: timeline() }] } }] };
    drainPendingMessageUpdates(pending, update => { rows = update(rows); });
    expect(rows[0].content.timeline?.through_seq).toBe(10);
    expect(rows).toHaveLength(2);
  });

  test('rejects future, duplicate and malformed references', () => {
    const valid = timeline();
    expect(normalizeMessageTimeline(valid)).toBeDefined();
    expect(normalizeMessageTimeline({ ...valid, through_seq: 2 })).toBeUndefined();
    expect(normalizeMessageTimeline({ ...valid, segments: [...valid.segments, ...valid.segments] })).toBeUndefined();
    expect(normalizeMessageTimeline({ ...valid, segments: [{ ...valid.segments[0], end_utf16: -1 }] })).toBeUndefined();
  });

  test('raw tool frames and older stream versions cannot erase canonical references or hold evidence', () => {
    const current = { call_id: 'call', name: 'read_file', timeline: timeline(20), status: 'error' as const,
      runtime_hold: 'arguments_rejected' as const, result_seq: 20, error: 'canonical refusal', command_not_started: true };
    const incoming = normalizeToolCallContent({ call_id: 'call', name: 'read_file', status: 'running' });
    const merged = mergeToolCallContent(current, incoming);
    expect(merged.timeline?.through_seq).toBe(20);
    expect(merged.runtime_hold).toBe('arguments_rejected');
    expect(merged.error).toBe('canonical refusal');
    expect(merged.command_not_started).toBe(true);
    expect(mergeToolCallContent(current, { ...incoming, timeline: timeline(19), result_seq: 19 }).timeline?.through_seq).toBe(20);
  });

  test('history body selection cannot revive withdrawn segments from an older snapshot', () => {
    const stream = { ...text, type: 'thinking' as const, content: { content: 'short', status: 'done' as const,
      timeline: { through_seq: 20, segments: [] } } };
    const database = { ...stream, message_id: source, content: { content: 'longer saved canonical body', status: 'thinking' as const, timeline: timeline(10) } };
    const rows = mergeFetchedMessagesForConversation([stream], [database], conversation);
    expect(rows[0].type === 'thinking' && rows[0].content.content).toBe('longer saved canonical body');
    expect(rows[0].content.timeline?.through_seq).toBe(20);
    expect(rows[0].content.timeline?.segments).toEqual([]);
  });
});
