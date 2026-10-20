import { normalizeSystemPermissionRequired, type SystemPermissionRequired, type TMessage } from '@/common/chat/chatLib';
import type { SystemPermissionStatus } from '@/common/adapter/ipcBridge';
import type { MessageId } from '@/common/types/ids';
import { computerPermissionKindsForOperation, permissionEntryIsReady, systemPermissionEntry } from '@/renderer/hooks/system/systemPermissionModel';

export type SystemPermissionNotice = {
  id: string;
  turn_id: MessageId;
  request: SystemPermissionRequired;
  /** Identity of actual unresolved refusals; different from dismissal scope. */
  refusal_id: string;
  sourceMessageIds: MessageId[];
  created_at: number;
};

/** Display only a typed host refusal, never a task guess or a parsed error string. */
export const collectSystemPermissionNotices = (messages: TMessage[], currentTurnId?: MessageId): SystemPermissionNotice[] => {
  const notices = new Map<string, SystemPermissionNotice>();
  // The caller supplies the latest accepted root from the shared canonical
  // Turn association. Request message IDs and backend Turn IDs can differ.
  if (!currentTurnId) return [];
  const successfulChecks = messages.flatMap(message => {
    if (message.turn_id !== currentTurnId || message.hidden || (message.type !== 'tool_call' && message.type !== 'tool_group')) return [];
    const calls = message.type === 'tool_call' ? [message.content] : message.content;
    return calls.flatMap(call => {
      if ((call.status !== 'completed' && call.status !== 'Success') || !call.action_id ||
        !Number.isSafeInteger(call.result_seq) || call.result_seq! < 0) return [];
      const permissions = computerPermissionKindsForOperation(call.action_id, call.operation, true);
      return permissions.length ? [{ permissions, result_seq: call.result_seq! }] : [];
    });
  });
  for (const message of messages) {
    if (message.turn_id !== currentTurnId || message.hidden || (message.type !== 'tool_call' && message.type !== 'tool_group')) continue;
    const calls = message.type === 'tool_call' ? [message.content] : message.content;
    for (const call of calls) {
      if (call.error_code !== 'SYSTEM_PERMISSION_REQUIRED' || (call.status !== 'error' && call.status !== 'Error')) continue;
      const request = normalizeSystemPermissionRequired(call.permission_required, call.action_id);
      if (!request) continue;
      // Only an exactly ordered, later successful action can resolve an old
      // refusal. Timestamps, prose and a successful action before revocation
      // provide no such proof. Missing sequence stays subject to live checks.
      if (Number.isSafeInteger(call.result_seq) && call.result_seq! >= 0 &&
        successfulChecks.some(success => success.result_seq > call.result_seq! &&
          request.permissions.every(kind => success.permissions.includes(kind)))) continue;
      const id = `turn-permission-${message.turn_id}-${request.permissions.join('-')}`;
      const sourceId = message.message_id ?? message.msg_id;
      const previous = notices.get(id);
      const refusalIdentity = `${call.result_seq ?? 'unsequenced'}:${call.call_id}`;
      if (previous) {
        if (sourceId && !previous.sourceMessageIds.includes(sourceId)) previous.sourceMessageIds.push(sourceId);
        previous.refusal_id = [...new Set([...previous.refusal_id.split('|'), refusalIdentity])].sort().join('|');
        continue;
      }
      notices.set(id, { id, turn_id: message.turn_id, request, refusal_id: refusalIdentity,
        sourceMessageIds: sourceId ? [sourceId] : [], created_at: message.created_at ?? 0 });
    }
  }
  return [...notices.values()];
};

export const systemPermissionNoticeState = (request: SystemPermissionRequired, status: SystemPermissionStatus | null) => {
  if (!status) return { kind: 'checking' as const, missing: request.permissions };
  const entries = request.permissions.map(kind => systemPermissionEntry(status, kind));
  if (entries.some(entry => !entry || entry.state === 'unknown')) {
    return { kind: 'check_failed' as const, missing: [] };
  }
  const missing = request.permissions.filter((_, index) => !permissionEntryIsReady(entries[index]));
  return { kind: missing.length ? 'missing' as const : 'ready' as const, missing };
};

/** Keep task blockers visible beside permission recovery, in a compact row. */
export const shouldCompactPermissionBlockedSummary = (
  message: TMessage,
  notices: SystemPermissionNotice[]
): boolean => {
  return message.type === 'tips' && message.content.type === 'error' && Boolean(message.turn_id) &&
    message.content.error?.code === 'NOMIFUN_TASK_INCOMPLETE' && message.content.error.taskIncompleteReason === 'blocked_work' &&
    !message.content.error.providerDiagnostic && !message.content.idmm_notice && !message.content.recovery &&
    !message.content.execution_pause && notices.some(notice => notice.turn_id === message.turn_id);
};
