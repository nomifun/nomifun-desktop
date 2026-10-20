import { expect, test } from 'bun:test';
import type { TMessage } from '@/common/chat/chatLib';
import { orderProcessSegments, projectProcessSegments } from './processSegments';
const textOf = (message: TMessage) => message.type === 'text' || message.type === 'thinking' ? message.content.content : '';
const source = (type: 'thinking' | 'text', content: string, segments: unknown[]): TMessage => ({ id: type, msg_id: type,
  turn_id: 'turn', conversation_id: 'session', type, position: 'left', content: { content, status: 'done', timeline: { through_seq: 20, segments } } }) as unknown as TMessage;
const segment = (id: string, seq: number, kind: 'reasoning' | 'narration', start: number, end: number, group = 'first') => ({
  segment_id: id, group_id: group, first_seq: seq, last_seq: seq, kind, state: 'completed', start_utf16: start, end_utf16: end,
});
test('canonical reasoning and narration each retain their complete source body through interleaving', () => {
  const thought = source('thinking', '甲😀乙', [{ ...segment('r1', 4, 'reasoning', 0, 4), last_seq: 6 }]);
  const text = '我将安装 **CLI**。';
  const narration = source('text', text, [{ ...segment('t1', 2, 'narration', 0, text.length), last_seq: 8 }]);
  const rows = orderProcessSegments(projectProcessSegments([thought, narration], new Set()));
  expect(rows.map(row => row.id)).toEqual(['t1', 'r1']);
  expect(rows.map(textOf)).toEqual([text, '甲😀乙']);
  expect(rows.map(row => row.processGroupId)).toEqual([undefined, 'first']);
  expect(textOf(thought)).toBe('甲😀乙');
});

test.each(['Let me make the call now.', 'Inspect the handler.', '检查😀处理器，现在。'])(
  'one canonical reasoning range retains the whole sentence across distinct narration sources: %s', content => {
    const thought = source('thinking', content, [{ ...segment('reasoning', 2, 'reasoning', 0, content.length), last_seq: 10 }]);
    const first = source('text', '好的', [segment('first-output', 4, 'narration', 0, 2)]);
    const later = { ...source('text', '继续正文', [segment('later-output', 8, 'narration', 0, 4)]), id: 'later-text', msg_id: 'later-text' } as TMessage;
    const rows = orderProcessSegments(projectProcessSegments([thought, first, later], new Set()));
    expect(rows.map(row => row.id)).toEqual(['reasoning', 'first-output', 'later-output']);
    expect(rows.map(textOf)).toEqual([content, '好的', '继续正文']);
    expect(rows.filter(row => row.type === 'thinking')).toHaveLength(1);
    expect(textOf(thought)).toBe(content);
  }
);

test('distinct canonical reasoning ranges remain distinct even when their source and process anchor match', () => {
  const thought = source('thinking', 'First.Second.', [segment('boundary-first', 2, 'reasoning', 0, 6),
    segment('boundary-second', 4, 'reasoning', 6, 13)]);
  const rows = projectProcessSegments([thought], new Set());
  expect(rows.map(row => row.id)).toEqual(['boundary-first', 'boundary-second']);
  expect(rows.map(textOf)).toEqual(['First.', 'Second.']);
  expect(rows.map(row => row.msg_id)).toEqual([thought.msg_id, thought.msg_id]);
});

test.each(['now.', '.', '。'])('a genuinely short canonical reasoning source %j remains visible', content => {
  const thought = source('thinking', content, [segment('short-source', 2, 'reasoning', 0, content.length)]);
  expect(projectProcessSegments([thought], new Set()).map(textOf)).toEqual([content]);
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
  expect(projectProcessSegments([empty], new Set())).toEqual([]);
});

test.each(['', ' \n ', '[Private reasoning omitted]', '[Private reasoning omitted from replay]'])(
  'unreadable reasoning %j cannot introduce an empty process group', text => {
    const thought = source('thinking', text, [segment('r1', 3, 'reasoning', 0, text.length)]);
    const narration = source('text', 'CLI', [segment('t1', 2, 'narration', 0, 3)]);
    const rows = orderProcessSegments(projectProcessSegments([thought, narration], new Set()));
    expect(rows.map(row => row.id)).toEqual(['t1']);
    expect(rows.map(textOf)).toEqual(['CLI']);
    expect(projectProcessSegments([{ ...thought, content: { content: text, status: 'done' } } as TMessage], new Set())).toEqual([]);
  }
);

test('distinct canonical narration ranges preserve a true tool boundary within one source', () => {
  const narration = source('text', 'Before.After.', [segment('before-tool', 2, 'narration', 0, 7),
    segment('after-tool', 6, 'narration', 7, 13, 'after-tool')]);
  const tool = { id: 'tool', type: 'tool_call', conversation_id: 'session', turn_id: 'turn', content: {
    call_id: 'call', name: 'read_file', status: 'completed', timeline: { through_seq: 20, segments: [{
      segment_id: 'tool-start', group_id: 'before-tool', first_seq: 4, last_seq: 5, kind: 'tool',
      call_id: 'call', state: 'completed',
    }] },
  } } as unknown as TMessage;
  const rows = orderProcessSegments(projectProcessSegments([narration, tool], new Set()));
  expect(rows.map(row => row.id)).toEqual(['before-tool', 'tool-start', 'after-tool']);
  expect(rows.filter(row => row.type === 'text').map(textOf)).toEqual(['Before.', 'After.']);
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
