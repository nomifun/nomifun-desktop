import { expect, test } from 'bun:test';
import type { IMessageToolCall } from '@/common/chat/chatLib';
import type { DelegationRecord } from '@/common/chat/delegationRecords';
import { parseConversationId, parseExecutionAttemptId, parseExecutionId, parseMessageId } from '@/common/types/ids';
import { makeDelegationRecord, leadConversationId, makeStep } from '../../../../../test/fixtures/conversationDelegation';
import { compareDelegationRecords, insertDelegationRecords, withoutRecordedDelegationCalls } from './delegationProcessModel';

test('canonical anchors weave flat records; historical source-only completions remain at their source tail', () => {
  const items = [{ id: 'before', processSequence: 5, processGroupId: 'group' }, { id: 'after', processSequence: 15 }];
  const start = makeDelegationRecord();
  const done = makeDelegationRecord({ id: 'done', kind: 'completed', source_scoped_order: true, execution_event_sequence: 2 });
  const result = insertDelegationRecords(items, [done, start]);
  expect(result.map(row => row.id)).toEqual(['before', 'delegation:delegated-record', 'after', 'delegation:done']);
  expect(items[0].processGroupId).toBe('group');
  const later = makeDelegationRecord({ id: 'later', anchor: { ...start.anchor, seq: 20 } });
  const mixed = insertDelegationRecords(items, [done, later, start]);
  expect(mixed.at(-1)?.id).toBe('delegation:done');
  expect(mixed.at(-1)?.processSequence).toBeUndefined();
});

test('only the exact successful source call is replaced; unrelated and failed calls remain visible', () => {
  const call = (id: string, status: 'completed' | 'error'): IMessageToolCall => ({ id, conversation_id: leadConversationId, type: 'tool_call', content: { call_id: id, name: 'delegate_task', status } });
  const source = makeDelegationRecord();
  expect(withoutRecordedDelegationCalls([call('delegate-call', 'completed'), call('other', 'completed')], [source]).map(row => row.id)).toEqual(['other']);
  expect(withoutRecordedDelegationCalls([call('delegate-call', 'error')], [source])).toHaveLength(1);
});

test('independent execution counters never order one another at a shared Session anchor', () => {
  const first = makeDelegationRecord({ id: 'first', execution_id: parseExecutionId('00000000-07d0-7000-8000-000000000001'), execution_event_sequence: 900 });
  const second = makeDelegationRecord({ id: 'second', execution_id: parseExecutionId('00000000-07d0-7000-8000-000000000002'), execution_event_sequence: 1 });
  expect([second, first].sort(compareDelegationRecords).map(record => record.id)).toEqual(['first', 'second']);
  expect(insertDelegationRecords([], [second, first]).map(record => record.id)).toEqual(['delegation:first', 'delegation:second']);
});

test('a five-task delegation and two terminal facts compact to two adjacent semantic runs without losing facts', () => {
  const start = Array.from({ length: 5 }, (_, index) => makeDelegationRecord({ id: `start-${index}`, title: `Task ${index}`,
    step_id: makeStep(index).step_id, execution_event_sequence: index + 1 }));
  const complete = start.slice(0, 2).map((record, index) => ({ ...record, id: `end-${index}`, kind: 'completed' as const,
    execution_event_sequence: index + 10, anchor: { ...record.anchor, seq: 30 } }));
  const facts = [complete[1], ...[...start].reverse(), complete[0]];
  const original = JSON.stringify(facts);
  const projected = insertDelegationRecords<never>([], facts);
  expect(projected).toHaveLength(2);
  expect(projected.map(item => [item.id, item.processSequence, item.records.length])).toEqual([
    ['delegation:start-0', 10, 5], ['delegation:end-0', 30, 2],
  ]);
  expect(projected[0].records.map(record => record.id)).toEqual(start.map(record => record.id));
  expect(projected[1].records.map(record => record.id)).toEqual(complete.map(record => record.id));
  expect(projected[0].records[0] === start[0]).toBe(true);
  expect(JSON.stringify(facts)).toBe(original);
  const increment = insertDelegationRecords<never>([], [...facts, makeDelegationRecord({ id: 'start-5', execution_event_sequence: 6 })]);
  expect(increment[0].id).toBe(projected[0].id);
  expect(increment[0].records).toHaveLength(6);
});

test.each(['text', 'tool_call', 'tool_summary', 'thinking'])('%s remains a hard boundary between otherwise matching delegation records', type => {
  const first = makeDelegationRecord({ id: 'first' });
  const second = makeDelegationRecord({ id: 'second', execution_event_sequence: 2, anchor: { ...first.anchor, seq: 12 } });
  const barrier = { id: 'barrier', type, processSequence: 11, processGroupId: 'original-group' };
  const projected = insertDelegationRecords([barrier], [second, first]);
  expect(projected.map(item => item.id)).toEqual(['delegation:first', 'barrier', 'delegation:second']);
  expect(projected[1] === barrier).toBe(true);
  expect(barrier.processGroupId).toBe('original-group');
});

const source = makeDelegationRecord().source;
const differentOrigins: Array<{ name: string; overrides: Partial<DelegationRecord> }> = [
  { name: 'kind', overrides: { kind: 'completed' } },
  { name: 'Execution', overrides: { execution_id: parseExecutionId('00000000-07d0-7000-8000-000000000099') } },
  { name: 'source Session', overrides: { source: { ...source, agent_session_id: parseConversationId('00000000-07d0-7000-8000-000000000099') } } },
  { name: 'accepted request', overrides: { source: { ...source, source_message_id: parseMessageId('00000000-07d0-7000-8000-000000000099') } } },
  { name: 'turn operation', overrides: { source: { ...source, turn_operation_id: 'different-turn' } } },
  { name: 'tool operation', overrides: { source: { ...source, tool_operation_id: 'different-operation' } } },
  { name: 'tool call', overrides: { source: { ...source, tool_call_id: 'different-call' } } },
  { name: 'effect', overrides: { source: { ...source, effect_id: 'different-effect' } } },
  { name: 'ordering authority', overrides: { source_scoped_order: true } },
];
test.each(differentOrigins)('a different $name cannot share a batch', ({ overrides }) => {
  const projected = insertDelegationRecords<never>([], [makeDelegationRecord({ id: 'first' }),
    makeDelegationRecord({ id: 'second', execution_event_sequence: 2, ...overrides })]);
  expect(projected).toHaveLength(2);
  expect(projected.map(item => item.records.length)).toEqual([1, 1]);
});

test('different terminal kinds stay explicit while duplicate titles retain both exact historical Attempts', () => {
  const original = makeDelegationRecord({ id: 'old-result', kind: 'completed', title: 'Same title',
    attempt_id: parseExecutionAttemptId('00000000-09c4-7000-8000-000000000030') });
  const next = { ...original, id: 'new-result', execution_event_sequence: 2,
    attempt_id: parseExecutionAttemptId('00000000-09c4-7000-8000-000000000031') };
  const failed = { ...next, id: 'failed-result', kind: 'failed' as const, execution_event_sequence: 3 };
  const skipped = { ...next, id: 'skipped-result', kind: 'skipped' as const, execution_event_sequence: 4 };
  const projected = insertDelegationRecords<never>([], [skipped, next, failed, original]);
  expect(projected.map(item => item.records.map(record => record.kind))).toEqual([['completed', 'completed'], ['failed'], ['skipped']]);
  expect(projected[0].records.map(record => record.attempt_id)).toEqual([original.attempt_id, next.attempt_id]);
});
