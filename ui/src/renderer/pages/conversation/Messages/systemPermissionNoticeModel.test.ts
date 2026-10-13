import { expect, test } from 'bun:test';
import type { IMessageToolCall, SystemPermissionRequired, TMessage } from '@/common/chat/chatLib';
import { normalizeToolCallContent, normalizeToolGroupContent } from '@/common/chat/chatLib';
import { parseConversationId, parseMessageId } from '@/common/types/ids';
import { collectSystemPermissionNotices as collect, shouldCompactPermissionBlockedSummary } from './systemPermissionNoticeModel';
import { assignTurnIdsFromUserRequests } from './turnDisclosureModel';
const collectSystemPermissionNotices = (messages: TMessage[]) => {
  const associations = assignTurnIdsFromUserRequests(messages.map(message => ({ id: message.id,
    role: message.type === 'text' && message.position === 'right' ? 'user' as const : 'process' as const,
    turnId: message.type === 'text' && message.position === 'right' ? message.msg_id : message.turn_id,
    createdAt: message.created_at ?? 0 })));
  return collect(messages, associations.findLast(entry => entry.role === 'user')?.turnId);
};

const conversation = parseConversationId('01900000-0000-7000-8000-000000000001');
const turn = parseMessageId('01900000-0000-7000-8000-000000000002');
const nextTurn = parseMessageId('01900000-0000-7000-8000-000000000003');
export const request: SystemPermissionRequired = { schema: 'system-permission-required.v1', capability: 'computer',
  action_id: 'computer/input', operation: 'left_click', permissions: ['accessibility'], execution: 'not_executed' };
const root: TMessage = { id: 'root', type: 'text', position: 'right', conversation_id: conversation,
  msg_id: turn, created_at: 1, content: { content: 'Do the task.' } };
const call: IMessageToolCall = { id: 'call', type: 'tool_call', conversation_id: conversation,
  turn_id: turn, msg_id: parseMessageId('01900000-0000-7000-8000-000000000004'), created_at: 2,
  content: { call_id: 'computer-call', name: 'computer_input', action_id: 'computer/input', status: 'error',
    error_code: 'SYSTEM_PERMISSION_REQUIRED', permission_required: request } };

test('requires exact host metadata instead of task words or English errors', () => {
  expect(collectSystemPermissionNotices([root])).toEqual([]);
  expect(collectSystemPermissionNotices([root, { ...call, content: { ...call.content, error_code: undefined,
    error: 'SYSTEM_PERMISSION_REQUIRED Accessibility permission is missing' } }])).toEqual([]);
  expect(collectSystemPermissionNotices([root, { ...call, content: { ...call.content,
    permission_required: { ...request, execution: 'executed' } as unknown as SystemPermissionRequired } }])).toEqual([]);
  expect(collectSystemPermissionNotices([root, { ...call, turn_id: undefined }])).toEqual([]);
  expect(collectSystemPermissionNotices([root, { ...call, content: { ...call.content, status: 'completed' } }])).toEqual([]);
});

test('deduplicates by exact Turn and missing set, including tool_group', () => {
  const duplicate: TMessage = { ...call, id: 'group', type: 'tool_group', content: [{
    call_id: 'retry', name: 'computer_input', action_id: 'computer/input', description: '', status: 'Error',
    error_code: 'SYSTEM_PERMISSION_REQUIRED', permission_required: request }] };
  const notices = collectSystemPermissionNotices([root, call, duplicate]);
  expect(notices).toHaveLength(1);
  expect(notices[0]?.turn_id).toBe(turn);
  expect(notices[0]?.request.permissions).toEqual(['accessibility']);
});

test('a new accepted chat root retires old task prompts even if the old tool arrives late', () => {
  const next: TMessage = { ...root, id: 'new-root', msg_id: nextTurn, created_at: 3 };
  expect(collectSystemPermissionNotices([root, next, { ...call, created_at: 100 }])).toEqual([]);
  expect(collectSystemPermissionNotices([root, next, { ...call, turn_id: nextTurn }])).toHaveLength(1);
  expect(collectSystemPermissionNotices([call])).toEqual([]);
});

test('normalization preserves host refusal and rejects malformed structured metadata', () => {
  expect(normalizeToolCallContent(call.content).permission_required).toEqual(request);
  expect(normalizeToolCallContent({ ...call.content, permission_required: { ...request, permissions: ['microphone'] } }).permission_required).toBeUndefined();
  expect(normalizeToolCallContent({ ...call.content, action_id: 'computer/observe' }).permission_required).toBeUndefined();
  expect(normalizeToolCallContent({ ...call.content, permission_required: { ...request, operation: 'gibberish' } }).permission_required).toBeUndefined();
  expect(normalizeToolCallContent({ ...call.content, permission_required: { ...request, permissions: ['screen_recording'] } }).permission_required).toBeUndefined();
  const group = normalizeToolGroupContent([{ call_id: 'group', name: 'computer_input', status: 'Error', description: '',
    action_id: 'computer/input', error_code: 'SYSTEM_PERMISSION_REQUIRED', permission_required: request }]);
  expect(group[0]?.permission_required).toEqual(request);
  expect(group[0]?.action_id).toBe('computer/input');
});

test('only a later canonical successful action with matching prerequisites resolves a refusal', () => {
  const failure: IMessageToolCall = { ...call, content: { ...call.content, result_seq: 10 } };
  const success: IMessageToolCall = { ...call, id: 'success', content: { call_id: 'new-call', name: 'computer_input',
    action_id: 'computer/input', args: { action: 'left_click' }, status: 'completed', result_seq: 11 } };
  expect(collectSystemPermissionNotices([root, failure, success])).toEqual([]);
  expect(collectSystemPermissionNotices([root, failure, { ...success, content: { ...success.content, result_seq: 9 } }])).toHaveLength(1);
  expect(collectSystemPermissionNotices([root, failure, { ...success, content: { ...success.content, result_seq: undefined } }])).toHaveLength(1);
  expect(collectSystemPermissionNotices([root, failure, { ...success, content: { ...success.content, action_id: 'computer/observe' } }])).toHaveLength(1);
  expect(collectSystemPermissionNotices([root, failure, { ...success, content: { ...success.content, action_id: 'workspace.files/read' } }])).toHaveLength(1);
  expect(collectSystemPermissionNotices([root, failure, { ...success, content: { ...success.content, args: undefined } }])).toHaveLength(1);
  expect(collectSystemPermissionNotices([root, failure, { ...success, turn_id: nextTurn }])).toHaveLength(1);
  expect(collectSystemPermissionNotices([root, success, failure])).toEqual([]);
});

test('a successful permission-free observe operation does not resolve a screenshot refusal', () => {
  const screenshot: IMessageToolCall = { ...call, content: { ...call.content, result_seq: 10,
    action_id: 'computer/observe', permission_required: { ...request, action_id: 'computer/observe',
      operation: 'screenshot', permissions: ['screen_recording'] } } };
  const success: IMessageToolCall = { ...call, id: 'success', content: { call_id: 'new-call', name: 'computer_observe',
    action_id: 'computer/observe', args: { action: 'wait' }, status: 'completed', result_seq: 11 } };
  for (const action of ['wait', 'cursor_position', 'list_windows']) {
    expect(collectSystemPermissionNotices([root, screenshot, { ...success,
      content: { ...success.content, args: { action } } }])).toHaveLength(1);
  }
  expect(collectSystemPermissionNotices([root, screenshot, { ...success,
    content: { ...success.content, args: { action: 'screenshot' } } }])).toEqual([]);
});

test('new refusal identities retain a stable dismissal scope', () => {
  const first = collectSystemPermissionNotices([root, call])[0]!;
  const newer = { ...call, id: 'new-failure', content: { ...call.content, call_id: 'new-call', result_seq: 20 } };
  const repeated = collectSystemPermissionNotices([root, call, newer])[0]!;
  expect(repeated.id).toBe(first.id);
  expect(repeated.refusal_id).not.toBe(first.refusal_id);
});

test('only the matching generic blocked summary becomes compact while its task reason stays intact', () => {
  const tip: TMessage = { ...call, id: 'blocked-summary', type: 'tips', content: { type: 'error', content: '', error: {
    message: 'Task has blocked work.', detail: 'Blocked criterion: Provide source artwork.',
    code: 'NOMIFUN_TASK_INCOMPLETE', taskIncompleteReason: 'blocked_work' } } };
  const notices = collectSystemPermissionNotices([root, call]);
  expect(shouldCompactPermissionBlockedSummary(tip, notices)).toBe(true);
  expect(tip.content.error?.detail).toBe('Blocked criterion: Provide source artwork.');
  expect(shouldCompactPermissionBlockedSummary({ ...tip, content: { ...tip.content,
    error: { ...tip.content.error!, taskIncompleteReason: 'step_limit' } } }, notices)).toBe(false);
  expect(shouldCompactPermissionBlockedSummary({ ...tip, content: { ...tip.content,
    error: { ...tip.content.error!, code: 'USER_LLM_PROVIDER_AUTH_FAILED' } } }, notices)).toBe(false);
  expect(shouldCompactPermissionBlockedSummary(tip, [])).toBe(false);
});
