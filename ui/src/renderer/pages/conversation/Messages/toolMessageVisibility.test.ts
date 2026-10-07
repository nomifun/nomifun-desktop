import { expect, test } from 'bun:test';
import type { IMessageToolCall } from '@/common/chat/chatLib';
import { isInternalInstructionToolCall, isTaskPlanControlReceipt } from './toolMessageVisibility';

test('internal preflight reads are hidden and model-selected reads stay visible', () => {
  const call = { type: 'tool_call', content: { call_id: 'agent-instructions:100', name: 'read_file' } } as IMessageToolCall;
  expect(isInternalInstructionToolCall(call)).toBe(true);
  expect(isInternalInstructionToolCall({ ...call, content: { ...call.content, call_id: 'model-call-1' } })).toBe(false);
});

test('progress receipts hide only successful declarations and preserve real errors', () => {
  const call = { type: 'tool_call', content: { call_id: 'p1', name: 'update_plan', args: {}, status: 'completed' } } as IMessageToolCall;
  expect(isTaskPlanControlReceipt(call)).toBe(true);
  expect(isTaskPlanControlReceipt({ ...call, content: { ...call.content, status: 'error' } })).toBe(false);
  expect(isTaskPlanControlReceipt({ ...call, content: { ...call.content, name: 'write_file' } })).toBe(false);
});
