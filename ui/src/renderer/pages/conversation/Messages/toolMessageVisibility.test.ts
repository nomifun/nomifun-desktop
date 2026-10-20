import { expect, test } from 'bun:test';
import { mergeToolCallContent, type IMessageToolCall, type IMessageToolGroup, type IMessageText } from '@/common/chat/chatLib';
import { isInternalInstructionToolCall, isTaskControlReceipt } from './toolMessageVisibility';

const report = (overrides: Partial<IMessageToolCall['content']> = {}): IMessageToolCall => ({
  type: 'tool_call', content: { call_id: 'report', name: 'report_completion', status: 'completed', result_seq: 10, ...overrides },
} as IMessageToolCall);

test('internal preflight reads are hidden and model-selected reads stay visible', () => {
  const call = report({ call_id: 'agent-instructions:100', name: 'read_file' });
  expect(isInternalInstructionToolCall(call)).toBe(true);
  expect(isInternalInstructionToolCall({ ...call, content: { ...call.content, call_id: 'model-call-1' } })).toBe(false);
});

test('progress receipts hide only successful declarations and preserve real errors', () => {
  const call = report({ name: 'update_plan', result_seq: undefined });
  expect(isTaskControlReceipt(call)).toBe(true);
  expect(isTaskControlReceipt({ ...call, content: { ...call.content, status: 'error' } })).toBe(false);
  expect(isTaskControlReceipt({ ...call, content: { ...call.content, name: 'write_file' } })).toBe(false);
});

test('confirmed native completion bookkeeping is hidden without inspecting report content or changing facts', () => {
  for (const description of ['The final answer', 'Candidate account recorded', 'Blocked work remains']) {
    const call = report({ description });
    const before = JSON.stringify(call);
    expect(isTaskControlReceipt(call)).toBe(true);
    expect(JSON.stringify(call)).toBe(before);
  }
  // A committed result remains authoritative after the observation was interrupted.
  expect(isTaskControlReceipt(report({ timeline: { through_seq: 12, segments: [{ segment_id: 'report', group_id: 'g',
    first_seq: 5, last_seq: 9, kind: 'tool', call_id: 'report', state: 'interrupted' }] } }))).toBe(true);
});

test.each([
  { status: 'running' }, { status: undefined }, { status: 'error', error: 'Report rejected' }, { status: 'canceled' },
  { result_seq: undefined }, { result_seq: 0 }, { result_seq: -1 }, { result_seq: 1.5 }, { result_seq: Number.MAX_SAFE_INTEGER + 1 },
  { runtime_hold: 'arguments_rejected' }, { runtime_hold: 'prior_call_failed' }, { error: 'Later correction' },
] satisfies Partial<IMessageToolCall['content']>[] )('unsettled or rejected completion metadata remains visible: %j', metadata => {
  expect(isTaskControlReceipt(report(metadata))).toBe(false);
});

test('same-name external and partially identified tools remain inspectable, including tool groups', () => {
  for (const identity of [{ capability_id: 'external' }, { action_id: 'external/report' }, { capability_id: '' }, { action_id: '' },
    { capability_id: 'external', action_id: 'external/report' },
    { name: 'mcp__server__report_completion__0123456789abcdefabcd' }, { name: 'plugin__server__report_completion__0123456789abcdefabcd' }]) {
    expect(isTaskControlReceipt(report(identity))).toBe(false);
  }
  const group = { type: 'tool_group', content: [{ call_id: 'report', name: 'report_completion', status: 'Success' }] } as IMessageToolGroup;
  expect(isTaskControlReceipt(group)).toBe(false);
  const text = { type: 'text', content: { content: 'report_completion' } } as IMessageText;
  expect(isTaskControlReceipt(text)).toBe(false);
  expect(isTaskControlReceipt(report({ name: 'inspect', description: 'The final answer' }))).toBe(false);
});

test('live confirmation removes only the receipt and a real later error correction restores it', () => {
  const pending = report({ status: 'running', result_seq: undefined });
  expect(isTaskControlReceipt(pending)).toBe(false);
  const confirmed = { ...pending, content: mergeToolCallContent(pending.content, report().content) };
  expect(isTaskControlReceipt(confirmed)).toBe(true);
  const corrected = { ...confirmed, content: mergeToolCallContent(confirmed.content,
    report({ status: 'error', result_seq: 11, error: 'Completion review rejected' }).content) };
  expect(isTaskControlReceipt(corrected)).toBe(false);
  expect(corrected.content.error).toBe('Completion review rejected');
  const unconfirmed = report({ result_seq: undefined, timeline: { through_seq: 12, segments: [{ segment_id: 'interrupted-report',
    group_id: 'g', first_seq: 5, last_seq: 12, kind: 'tool', call_id: 'report', state: 'interrupted' }] } });
  expect(isTaskControlReceipt(unconfirmed)).toBe(false);
});
