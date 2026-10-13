/** Canonical message projections for the on-demand permission browser harness. */
import type { IMessageText, IMessageToolCall, TMessage } from '../../src/common/chat/chatLib';
import { parseConversationId, parseMessageId } from '../../src/common/types/ids';

export const taskPermissionConversationId = parseConversationId('0190f5fe-7c00-7a00-8000-000000003401');
export const taskPermissionTime = Date.UTC(2026, 9, 13, 6, 20, 0);
export const taskPermissionColdRootId = parseMessageId('0190f5fe-7c00-7a00-8000-000000009000');
const messageId = (index: number) => parseMessageId(`0190f5fe-7c00-7a00-8000-${String(index).padStart(12, '0')}`);

export const taskPermissionUser = (turn: number, text = '帮我操作电脑，整理下载文件夹'): IMessageText => ({
  id: `permission-user-${turn}`, msg_id: messageId(turn * 10), message_id: messageId(turn * 10),
  conversation_id: taskPermissionConversationId, type: 'text', position: 'right', status: 'finish',
  created_at: taskPermissionTime + turn * 10_000, content: { content: text },
});

export const taskPermissionCall = (turn: number, attempt = 1): IMessageToolCall => ({
  id: `permission-call-${turn}-${attempt}`, msg_id: messageId(turn * 10 + attempt), message_id: messageId(turn * 10 + attempt),
  turn_id: messageId(turn * 10), conversation_id: taskPermissionConversationId,
  type: 'tool_call', position: 'left', status: 'error', created_at: taskPermissionTime + turn * 10_000 + attempt,
  content: { call_id: `computer-input-${turn}-${attempt}`, name: 'computer/input', capability_id: 'computer',
    action_id: 'computer/input', args: { action: 'key', key: 'META+A' }, status: 'error',
    error: 'System permission is required before this operation. This operation was not executed.',
    error_code: 'SYSTEM_PERMISSION_REQUIRED', permission_required: {
      schema: 'system-permission-required.v1', capability: 'computer', action_id: 'computer/input',
      operation: 'key', permissions: ['accessibility'], execution: 'not_executed',
    } },
});

export function taskPermissionMessages(turn: number, incomplete = false): TMessage[] {
  const rows: TMessage[] = [taskPermissionUser(turn), taskPermissionCall(turn)];
  if (incomplete) {
    rows.push({ id: `permission-incomplete-${turn}`, msg_id: messageId(turn * 10 + 4),
      message_id: messageId(turn * 10 + 4), turn_id: messageId(turn * 10),
      conversation_id: taskPermissionConversationId, type: 'tips', position: 'center', status: 'error',
      created_at: taskPermissionTime + turn * 10_000 + 4,
      content: { content: 'The task has blocked work.', type: 'error', error: {
        message: 'Computer input is blocked because accessibility permission is missing.',
        code: 'NOMIFUN_TASK_INCOMPLETE', taskIncompleteReason: 'blocked_work', retryable: false,
      } } });
  }
  rows.push({ id: `permission-summary-${turn}`, msg_id: messageId(turn * 10 + 5),
    message_id: messageId(turn * 10 + 5), turn_id: messageId(turn * 10),
    conversation_id: taskPermissionConversationId, type: 'agent_status', position: 'left', status: 'finish',
    created_at: taskPermissionTime + turn * 10_000 + 5,
    content: { backend: 'nomi', status: 'prepared', turn_summary: true,
      turn_state: incomplete ? 'failed' : 'completed',
      started_at_ms: taskPermissionTime + turn * 10_000, finished_at_ms: taskPermissionTime + turn * 10_000 + 5 } });
  return rows;
}

export const taskPermissionOrdinaryMessages: TMessage[] = [
  taskPermissionUser(1, '今天适合做哪些整理工作？'),
  { id: 'ordinary-answer', msg_id: messageId(19), message_id: messageId(19), turn_id: messageId(10),
    conversation_id: taskPermissionConversationId, type: 'text', position: 'left', status: 'finish',
    created_at: taskPermissionTime + 10_001, content: { content: '可以先整理下载文件夹，再给常用资料分门别类。需要时我可以帮你制定整理步骤。' } },
];

/** Permission is one blocker; the task also lacks the user's sorting rules. */
export const taskPermissionMixedMessages = (turn: number): TMessage[] => taskPermissionMessages(turn, true).map(row =>
  row.type !== 'tips' ? row : { ...row, content: { ...row.content, error: { ...row.content.error!,
    message: 'The task still has independently blocked work.',
    detail: '尚未执行电脑输入；此外，仍缺少下载文件夹的整理规则与源素材。授权本身不能补齐这些信息。',
  } } });

export const taskPermissionBrowserMessages: TMessage[] = [
  taskPermissionUser(2, '打开内置浏览器，查看页面标题。'),
  { id: 'browser-success', msg_id: messageId(29), message_id: messageId(29), turn_id: messageId(20),
    conversation_id: taskPermissionConversationId, type: 'tool_call', position: 'left', status: 'finish',
    created_at: taskPermissionTime + 20_001,
    content: { call_id: 'browser-observe-2', name: 'browser/observe', capability_id: 'browser', action_id: 'browser/observe',
      status: 'completed', output: 'NomiFun · 欢迎' } },
];
