import { expect, test } from 'bun:test';
import type { IMessageToolCall, TMessage } from '@/common/chat/chatLib';
import type { ToolFileSummary } from '@/common/protocolBindings/ToolFileSummary';
import { parseConversationId, parseMessageId } from '@/common/types/ids';
import { compactFileActivityItems, isConfirmedFileActivity, projectFileActivity, type ToolFileActivityItem } from './fileActivityModel';
import type { ProcessSegmentIdentity } from './processSegments';

const conversation = parseConversationId('00000000-07d0-7000-8000-000000000001');
const turn = parseMessageId('00000000-07d0-7000-8000-000000000002');
const id = (n: number) => parseMessageId(`00000000-07d0-7000-8000-${String(n).padStart(12, '0')}`);
const summary = (operation: ToolFileSummary['operation'], files: ToolFileSummary['files'], kind?: ToolFileSummary['kind']): ToolFileSummary =>
  ({ operation, files, ...(kind ? { kind } : {}) });
const call = (n: number, content: Partial<IMessageToolCall['content']> = {}, meta: Partial<IMessageToolCall & ProcessSegmentIdentity> = {}): IMessageToolCall & ProcessSegmentIdentity => ({
  id: `call-${n}`, message_id: id(n + 10), msg_id: id(n + 10), conversation_id: conversation, turn_id: turn,
  type: 'tool_call', created_at: n, processSequence: n * 10, processGroupId: 'canonical-group',
  ...meta,
  content: { call_id: `call-${n}`, name: 'read_file', capability_id: 'workspace.files', action_id: 'workspace.files/read',
    status: 'completed', result_seq: n * 10 + 1, file_summary: summary('read', [{ path: `${n}.txt` }]), ...content },
});

test('only exact confirmed ordinary file actions qualify, independently of aliases', () => {
  expect(isConfirmedFileActivity(call(1, { name: 'model-selected-name' }))).toBe(true);
  for (const operation of ['write', 'patch', 'delete', 'rename'] as const) {
    expect(isConfirmedFileActivity(call(1, { action_id: `workspace.files/${operation}`,
      file_summary: summary(operation, [{ path: ' source ', ...(operation === 'rename' ? { destination: ' target ' } : {}) }]) }))).toBe(true);
  }
  const invalid: Array<Partial<IMessageToolCall['content']>> = [
    { status: 'running' }, { status: 'error' }, { status: 'canceled' }, { status: undefined },
    { result_seq: undefined }, { result_seq: 0 }, { result_seq: -1 }, { result_seq: 1.5 },
    { result_seq: Number.MAX_SAFE_INTEGER + 1 }, { runtime_hold: 'arguments_rejected' },
    { runtime_hold: 'prior_call_failed' }, { error: 'not accepted' }, { error: '' },
    { capability_id: undefined }, { capability_id: 'nomi.mcp.v1.opaque' },
    { action_id: undefined }, { action_id: 'workspace.files/write' },
    { file_summary: undefined }, { file_summary: summary('read', []) },
    { file_summary: summary('read', [{ path: '' }]) },
    { file_summary: summary('read', [{ path: 'folder' }], 'instruction_scope') },
    { file_summary: summary('rename', [{ path: 'source' }]), action_id: 'workspace.files/rename' },
    { file_summary: summary('read', [{ path: 'PRIVATE\nBODY' }]) },
  ];
  for (const change of invalid) expect(isConfirmedFileActivity(call(1, change))).toBe(false);
  expect(isConfirmedFileActivity(call(1, { result_seq: undefined, timeline: { through_seq: 30,
    segments: [{ segment_id: 'segment', group_id: 'group', kind: 'tool', state: 'interrupted', call_id: 'call-1', first_seq: 10, last_seq: 20 }] } }))).toBe(false);
});

test('adjacent confirmed calls and file-only summaries retain all calls, sources and first metadata', () => {
  const first = call(1);
  const second = call(2);
  const third = call(3);
  const wrapper: ToolFileActivityItem = { ...second, turn_id: turn, type: 'tool_summary', messages: [second, third],
    sourceMessageIds: [id(99), second.msg_id!], created_at: 2 };
  const original = JSON.stringify([first, wrapper]);
  const items = compactFileActivityItems([first, wrapper]);
  expect(items).toHaveLength(1);
  const group = items[0] as ToolFileActivityItem;
  expect(group.type).toBe('tool_summary');
  expect(group.id).toBe(first.id);
  expect(group.processSequence).toBe(first.processSequence);
  expect(group.processGroupId).toBe(first.processGroupId);
  expect(group.message_id).toBe(first.message_id);
  expect(group.created_at).toBe(first.created_at);
  expect(group.messages).toEqual([first, second, third]);
  expect(group.messages.every((item, index) => item === [first, second, third][index])).toBe(true);
  expect(group.sourceMessageIds).toEqual([first.message_id!, id(99), second.message_id!, third.message_id!]);
  expect(JSON.stringify([first, wrapper])).toBe(original);
  expect(compactFileActivityItems(items)).toEqual(items);
});

test.each(['text', 'thinking', 'tool_group', 'delegation_record', 'unknown'])('%s is a hard adjacency boundary', type => {
  const barrier = { id: 'barrier', type, content: { content: 'public narration' } } as unknown as TMessage;
  const items = compactFileActivityItems([call(1), barrier, call(2)]);
  expect(items).toHaveLength(3);
  expect(items[1]).toBe(barrier);
});

test('commands, errors, missing summaries and instruction scope are hard boundaries', () => {
  for (const barrier of [call(3, { capability_id: 'workspace.process', action_id: 'workspace.process/start' }),
    call(3, { status: 'error' }), call(3, { file_summary: undefined }),
    call(3, { file_summary: summary('read', [{ path: 'folder' }], 'instruction_scope') })]) {
    const items = compactFileActivityItems([call(1), barrier, call(2)]);
    expect(items).toHaveLength(3);
    expect(items[1]).toBe(barrier);
  }
});

test('a mixed or unknown tool summary cannot conceal a boundary', () => {
  const good = call(2);
  const bad = call(3, { status: 'error' });
  for (const messages of [[good, bad], [], [{ type: 'tool_group', content: [] }]]) {
    const wrapper = { id: 'mixed', type: 'tool_summary', messages };
    const items = compactFileActivityItems<unknown>([call(1), wrapper, call(4)]);
    expect(items).toHaveLength(3);
    expect(items[1]).toBe(wrapper);
  }
});

test('Conversation, accepted Turn and canonical narration groups never merge', () => {
  for (const second of [call(2, {}, { conversation_id: parseConversationId('00000000-07d0-7000-8000-000000000099') }),
    call(2, {}, { turn_id: id(99) }), call(2, {}, { turn_id: undefined }),
    call(2, {}, { processGroupId: 'next-canonical-narration' }), call(2, {}, { processGroupId: undefined })]) {
    expect(compactFileActivityItems([call(1), second])).toHaveLength(2);
  }
  const first = call(1);
  const other = call(2, {}, { turn_id: id(99) });
  const wrapper = { ...first, type: 'tool_summary', messages: [first, other], sourceMessageIds: [] };
  expect(compactFileActivityItems<unknown>([wrapper, call(3)])[0]).toBe(wrapper);
});

test('repeated source keys count once and repeated file reads keep one target in original order', () => {
  const projected = projectFileActivity([
    { key: 'one', summary: summary('read', [{ path: 'one.txt' }, { path: 'two.txt' }]) },
    { key: 'one', summary: summary('read', [{ path: 'must-not-replace.txt' }]) },
    { key: 'two', summary: summary('read', [{ path: 'one.txt' }, { path: 'three.txt' }]) },
  ]);
  expect(projected.entries.map(entry => [entry.id, entry.action, entry.path])).toEqual([
    ['file:one:0', 'read', 'one.txt'], ['file:one:1', 'read', 'two.txt'], ['file:two:1', 'read', 'three.txt'],
  ]);
  expect(projected.counts).toEqual({ read: 3, edit: 0, delete: 0, rename: 0, instructions: 0 });
});

test('multi-file patches sum exact per-call changes without double-counting replayed calls', () => {
  const first = { key: 'patch-a', summary: summary('patch', [{ path: 'a', added_lines: 2, removed_lines: 1 },
    { path: 'b', added_lines: 0, removed_lines: 0 }]) };
  const inputs = [first, first, { key: 'patch-b', summary: summary('patch', [{ path: 'a', added_lines: 3, removed_lines: 4 }]) }];
  const original = JSON.stringify(inputs);
  const result = projectFileActivity(inputs);
  expect(result.counts.edit).toBe(2);
  expect(result.entries.map(entry => [entry.path, entry.addedLines, entry.removedLines])).toEqual([['a', 5, 5], ['b', 0, 0]]);
  expect(JSON.stringify(inputs)).toBe(original);
});

test('writes and patches share edit targets but unknown changes never become zero or partial totals', () => {
  const known = { key: 'patch', summary: summary('patch', [{ path: 'a', added_lines: 2, removed_lines: 1 }]) };
  const unknown = { key: 'write', summary: summary('write', [{ path: 'a' }]) };
  for (const inputs of [[known, unknown], [unknown, known], [known, unknown, { ...known, key: 'later-patch' }]]) {
    const result = projectFileActivity(inputs);
    expect(result.counts.edit).toBe(1);
    expect(result.entries[0].addedLines).toBeUndefined();
    expect(result.entries[0].removedLines).toBeUndefined();
  }
  const partial = projectFileActivity([known, { key: 'partial', summary: summary('patch', [{ path: 'a', added_lines: 2 }]) }]);
  expect(partial.entries[0]).not.toHaveProperty('addedLines');
  expect(partial.entries[0]).not.toHaveProperty('removedLines');
});

test('nonintegral, negative and overflowing edit totals are omitted as a pair', () => {
  for (const added of [-1, 1.5, Number.MAX_SAFE_INTEGER + 1]) {
    expect(projectFileActivity([{ key: 'invalid', summary: summary('patch', [{ path: 'a', added_lines: added, removed_lines: 1 }]) }]).entries[0].addedLines).toBeUndefined();
  }
  const overflow = projectFileActivity([
    { key: 'first', summary: summary('patch', [{ path: 'a', added_lines: Number.MAX_SAFE_INTEGER, removed_lines: 0 }]) },
    { key: 'second', summary: summary('patch', [{ path: 'a', added_lines: 1, removed_lines: 0 }]) },
  ]);
  expect(overflow.entries[0].addedLines).toBeUndefined();
  expect(overflow.entries[0].removedLines).toBeUndefined();
});

test('paths retain exact whitespace and spelling; instructions do not inflate read counts', () => {
  const result = projectFileActivity([
    { key: 'scope', summary: summary('read', [{ path: ' folder ' }], 'instruction_scope') },
    { key: 'read', summary: summary('read', [{ path: ' folder ' }, { path: 'folder' }, { path: './folder' }, { path: '中文 文件.txt' }]) },
  ]);
  expect(result.entries.map(entry => [entry.action, entry.path])).toEqual([
    ['instructions', ' folder '], ['read', ' folder '], ['read', 'folder'], ['read', './folder'], ['read', '中文 文件.txt'],
  ]);
  expect(result.counts.read).toBe(4);
  expect(result.counts.instructions).toBe(1);
});

test('rename destinations and distinct operation facts remain explicit', () => {
  const result = projectFileActivity([
    { key: 'rename-a', summary: summary('rename', [{ path: ' same source ', destination: 'target-a' }]) },
    { key: 'rename-b', summary: summary('rename', [{ path: ' same source ', destination: 'target-a' }, { path: ' same source ', destination: 'target-b' }]) },
    { key: 'delete', summary: summary('delete', [{ path: ' same source ' }]) },
    { key: 'write', summary: summary('write', [{ path: ' same source ' }]) },
  ]);
  expect(result.counts).toEqual({ read: 0, edit: 1, delete: 1, rename: 2, instructions: 0 });
  expect(result.entries.map(entry => [entry.action, entry.path, entry.destination])).toEqual([
    ['rename', ' same source ', 'target-a'], ['rename', ' same source ', 'target-b'],
    ['delete', ' same source ', undefined], ['edit', ' same source ', undefined],
  ]);
});
