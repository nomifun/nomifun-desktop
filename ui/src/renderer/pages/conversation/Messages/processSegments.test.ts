import { expect, test } from 'bun:test';
import type { TMessage } from '@/common/chat/chatLib';
import { orderProcessSegments, projectProcessSegments } from './processSegments';
const textOf = (message: TMessage) => message.type === 'text' || message.type === 'thinking' ? message.content.content : '';
const source = (type: 'thinking' | 'text', content: string, segments: unknown[]): TMessage => ({ id: type, msg_id: type,
  turn_id: 'turn', conversation_id: 'session', type, position: 'left', content: { content, status: 'done', timeline: { through_seq: 20, segments } } }) as unknown as TMessage;
const segment = (id: string, seq: number, kind: 'reasoning' | 'narration', start: number, end: number, group = 'first') => ({
  segment_id: id, group_id: group, first_seq: seq, last_seq: seq, kind, state: 'completed', start_utf16: start, end_utf16: end,
});
test('canonical references restore same-source interleaving and UTF-16 ranges without new Message identities', () => {
  const thought = source('thinking', '甲😀乙', [segment('r1', 2, 'reasoning', 0, 3), segment('r2', 6, 'reasoning', 3, 4, 'second')]);
  const narration = source('text', 'AB', [segment('t1', 4, 'narration', 0, 1), segment('t2', 8, 'narration', 1, 2)]);
  const rows = orderProcessSegments(projectProcessSegments([thought, narration], new Set()));
  expect(rows.map(row => row.id)).toEqual(['r1', 't1', 'r2', 't2']);
  expect(rows.map(textOf)).toEqual(['甲😀', 'A', '乙', 'B']);
  expect(rows.map(row => row.processGroupId)).toEqual(['first', undefined, 'second', undefined]);
  expect(rows[0].msg_id).toBe(rows[2].msg_id);
  expect(textOf(thought)).toBe('甲😀乙');
});
test('partial pages retain backend group anchors and missing metadata stays unclassified', () => {
  const tail = source('thinking', 'abcdef', [segment('tail', 8, 'reasoning', 3, 6, 'earlier-page-anchor')]);
  const unknown = { ...tail, id: 'unclassified', content: { content: 'raw', status: 'done' } } as TMessage;
  const rows = projectProcessSegments([tail, unknown], new Set());
  expect(rows[0].processGroupId).toBe('earlier-page-anchor');
  expect(textOf(rows[0])).toBe('def');
  expect(rows[1].processGroupId).toBeUndefined();
});
test('a withdrawn source with explicit empty metadata stays hidden, including an obsolete result receipt', () => {
  const old = source('text', 'Withdrawn', []);
  expect(projectProcessSegments([old], new Set(['text']))).toEqual([]);
});
test('only a live final source segment may extend into transient body bytes', () => {
  const message = source('thinking', 'ABCD', [{ ...segment('live', 2, 'reasoning', 1, 2), state: 'running' }]);
  expect(textOf(projectProcessSegments([message], new Set())[0])).toBe('BCD');
  const empty = source('thinking', 'AB', [{ ...segment('budget-phase', 2, 'reasoning', 2, 2), state: 'running' }]);
  expect(projectProcessSegments([empty], new Set())[0].processSegmentState).toBe('running');
});

test('an interrupted tool observation stops display activity without inventing cancellation', () => {
  const tool = { id: 'tool-source', type: 'tool_call', conversation_id: 'session', turn_id: 'turn',
    content: { call_id: 'unresolved', name: 'browser_navigate', status: 'running', args: { url: 'https://example.com' },
      timeline: { through_seq: 20, segments: [{ segment_id: 'tool-start', group_id: 'group', first_seq: 3, last_seq: 20,
        kind: 'tool', call_id: 'unresolved', state: 'interrupted' }] } } } as unknown as TMessage;
  const [row] = projectProcessSegments([tool], new Set());
  expect(row.processSegmentState).not.toBe('running');
  expect(row.processSegmentState).not.toBe('canceled');
  expect(tool.type === 'tool_call' && tool.content.status).toBe('running');
  expect(row.content.timeline?.segments[0].state).toBe('interrupted');
});
