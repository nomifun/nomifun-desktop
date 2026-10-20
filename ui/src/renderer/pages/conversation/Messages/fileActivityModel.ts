import type { IMessageToolCall } from '@/common/chat/chatLib';
import { normalizeToolFileSummary } from '@/common/chat/fileSummary';
import type { ToolFileSummary } from '@/common/protocolBindings/ToolFileSummary';
import type { ConversationId, MessageId } from '@/common/types/ids';
import { isToolObservationUnconfirmed, type ProcessSegmentIdentity } from './processSegments';

export type ToolFileActivityItem = ProcessSegmentIdentity & {
  type: 'tool_summary';
  id: string;
  msg_id?: MessageId;
  message_id?: MessageId;
  conversation_id: ConversationId;
  turn_id: MessageId;
  messages: IMessageToolCall[];
  sourceMessageIds: MessageId[];
  created_at: number;
};

export type FileActivityAction = 'read' | 'edit' | 'delete' | 'rename' | 'instructions';
export type FileActivityEntry = {
  id: string;
  action: FileActivityAction;
  path: string;
  destination?: string;
  addedLines?: number;
  removedLines?: number;
};
export type FileActivityProjection = {
  entries: FileActivityEntry[];
  counts: { read: number; edit: number; delete: number; rename: number; instructions: number };
};

/** Group only owner-confirmed ordinary file operations. Instruction scope is
 * metadata, and failed/proposed calls keep their individual status rows. */
export function isConfirmedFileActivity(message: IMessageToolCall): boolean {
  const content = message.content;
  if (message.type !== 'tool_call' || !content || content.status !== 'completed'
    || !Number.isSafeInteger(content.result_seq) || content.result_seq! <= 0
    || content.runtime_hold !== undefined || content.error !== undefined
    || isToolObservationUnconfirmed(message, content.call_id)) return false;
  const summary = normalizeToolFileSummary(content.file_summary, content.capability_id, content.action_id, true);
  return summary !== undefined && summary.kind === undefined;
}

const record = (value: unknown): value is Record<string, unknown> => value !== null
  && typeof value === 'object' && !Array.isArray(value);
const toolCall = (value: unknown): value is IMessageToolCall => record(value)
  && value.type === 'tool_call' && typeof value.id === 'string' && typeof value.conversation_id === 'string'
  && record(value.content) && typeof value.content.call_id === 'string' && typeof value.content.name === 'string';
const confirmedCall = (value: unknown): value is IMessageToolCall => toolCall(value) && isConfirmedFileActivity(value);

type FileRun = {
  calls: IMessageToolCall[];
  conversation: ConversationId;
  turn: MessageId;
  group: unknown;
  sourceMessageIds: MessageId[];
};

function fileRun(item: unknown): FileRun | undefined {
  if (!record(item)) return;
  const calls = confirmedCall(item) ? [item]
    : item.type === 'tool_summary' && Array.isArray(item.messages) && item.messages.length
      && item.messages.every(confirmedCall) ? item.messages : undefined;
  if (!calls) return;
  const first = calls[0];
  if (!first.conversation_id || !first.turn_id || calls.some(call =>
    call.conversation_id !== first.conversation_id || call.turn_id !== first.turn_id)) return;
  if ((item.conversation_id !== undefined && item.conversation_id !== first.conversation_id)
    || (item.turn_id !== undefined && item.turn_id !== first.turn_id)) return;
  const sourceMessageIds = [
    ...(Array.isArray(item.sourceMessageIds) ? item.sourceMessageIds.filter((id): id is MessageId => typeof id === 'string') : []),
    ...calls.flatMap(call => [call.message_id, call.msg_id].filter((id): id is MessageId => id !== undefined)),
  ];
  return { calls, conversation: first.conversation_id, turn: first.turn_id, group: item.processGroupId,
    sourceMessageIds: [...new Set(sourceMessageIds)] };
}

/** A transient view over original calls. Run this after delegation weaving:
 * every other item and every canonical narration group is a hard boundary. */
export function compactFileActivityItems<T>(items: T[]): Array<T | ToolFileActivityItem> {
  const result: Array<T | ToolFileActivityItem> = [];
  let previous: { run: FileRun; item: ToolFileActivityItem } | undefined;
  for (const item of items) {
    const run = fileRun(item);
    if (!run) { result.push(item); previous = undefined; continue; }
    if (previous && previous.run.conversation === run.conversation && previous.run.turn === run.turn
      && previous.run.group === run.group) {
      previous.item.messages.push(...run.calls);
      previous.item.sourceMessageIds = [...new Set([...previous.item.sourceMessageIds, ...run.sourceMessageIds])];
      continue;
    }
    const source = item as Record<string, unknown>;
    const projected: ToolFileActivityItem = { ...source, type: 'tool_summary', id: source.id as string,
      conversation_id: run.conversation, turn_id: run.turn, messages: [...run.calls],
      sourceMessageIds: run.sourceMessageIds, created_at: typeof source.created_at === 'number' ? source.created_at : 0 };
    result.push(projected);
    previous = { run, item: projected };
  }
  return result;
}

const lineCount = (value: number | undefined): value is number => Number.isSafeInteger(value) && value! >= 0;

/** Count unique displayed targets, keeping each invocation only once. Edit
 * totals describe these calls, never a current workspace diff. */
export function projectFileActivity(inputs: Array<{ key: string; summary: ToolFileSummary }>): FileActivityProjection {
  const entries: FileActivityEntry[] = [];
  const counts: FileActivityProjection['counts'] = { read: 0, edit: 0, delete: 0, rename: 0, instructions: 0 };
  const seenCalls = new Set<string>();
  const byTarget = new Map<string, FileActivityEntry>();
  const completeEdits = new Set<string>();
  for (const { key, summary } of inputs) {
    if (seenCalls.has(key)) continue;
    seenCalls.add(key);
    const action: FileActivityAction = summary.kind === 'instruction_scope' ? 'instructions'
      : summary.operation === 'write' || summary.operation === 'patch' ? 'edit' : summary.operation;
    summary.files.forEach((file, index) => {
      const target = JSON.stringify([action, file.path, action === 'rename' ? file.destination : undefined]);
      let entry = byTarget.get(target);
      const complete = lineCount(file.added_lines) && lineCount(file.removed_lines);
      if (!entry) {
        entry = { id: `file:${key}:${index}`, action, path: file.path,
          ...(action === 'rename' && file.destination !== undefined ? { destination: file.destination } : {}) };
        if (action === 'edit' && complete) {
          entry.addedLines = file.added_lines;
          entry.removedLines = file.removed_lines;
          completeEdits.add(entry.id);
        }
        entries.push(entry);
        byTarget.set(target, entry);
        counts[action] += 1;
      } else if (action === 'edit') {
        const added = (entry.addedLines ?? 0) + (file.added_lines ?? 0);
        const removed = (entry.removedLines ?? 0) + (file.removed_lines ?? 0);
        if (completeEdits.has(entry.id) && complete && lineCount(added) && lineCount(removed)) {
          entry.addedLines = added;
          entry.removedLines = removed;
        } else {
          completeEdits.delete(entry.id);
          delete entry.addedLines;
          delete entry.removedLines;
        }
      }
    });
  }
  return { entries, counts };
}
