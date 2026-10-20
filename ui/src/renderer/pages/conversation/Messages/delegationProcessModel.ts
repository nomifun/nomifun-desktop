import type { DelegationRecord } from '@/common/chat/delegationRecords';
import type { IMessageToolCall, IMessageToolGroup } from '@/common/chat/chatLib';
import type { ProcessSegmentIdentity } from './processSegments';

export type DelegationProcessItem = ProcessSegmentIdentity & { type: 'delegation_record'; id: string; records: DelegationRecord[] };

/** Sequence comparison is valid only within its owning stream. */
export const compareDelegationRecords = (a: DelegationRecord, b: DelegationRecord): number =>
  a.anchor.seq - b.anchor.seq || a.execution_id.localeCompare(b.execution_id)
  || a.execution_event_sequence - b.execution_event_sequence || a.step_id.localeCompare(b.step_id) || a.id.localeCompare(b.id);

const sharesDelegationBatch = (a: DelegationRecord, b: DelegationRecord): boolean =>
  a.kind === b.kind && a.execution_id === b.execution_id && a.source_scoped_order === b.source_scoped_order
  && a.source.agent_session_id === b.source.agent_session_id && a.source.tool_operation_id === b.source.tool_operation_id
  && a.source.source_message_id === b.source.source_message_id && a.source.turn_operation_id === b.source.turn_operation_id
  && a.source.tool_call_id === b.source.tool_call_id && a.source.effect_id === b.source.effect_id;

const isDelegationItem = <T extends ProcessSegmentIdentity>(item: T | DelegationProcessItem): item is DelegationProcessItem =>
  'type' in item && item.type === 'delegation_record';

/** Compact contiguous records after weaving. Every other item is a boundary. */
const compactDelegationBatches = <T extends ProcessSegmentIdentity>(items: Array<T | DelegationProcessItem>): Array<T | DelegationProcessItem> => {
  const compacted: Array<T | DelegationProcessItem> = [];
  for (const item of items) {
    const previous = compacted.at(-1);
    if (previous && isDelegationItem(previous) && isDelegationItem(item)
      && sharesDelegationBatch(previous.records[0], item.records[0])) previous.records.push(...item.records);
    else compacted.push(isDelegationItem(item) ? { ...item, records: [...item.records] } : item);
  }
  return compacted;
};

/** Only the successful originating call is replaced. Rejections retain their own tool row. */
export function withoutRecordedDelegationCalls(messages: Array<IMessageToolCall | IMessageToolGroup>, records: DelegationRecord[]) {
  const calls = new Set(records.map(record => record.source.tool_call_id));
  return messages.flatMap((message): Array<IMessageToolCall | IMessageToolGroup> => {
    if (message.type === 'tool_call') return message.content.status === 'completed' && calls.has(message.content.call_id) ? [] : [message];
    const content = message.content.filter(call => call.status !== 'Success' || !calls.has(call.call_id));
    return content.length ? [{ ...message, content }] : [];
  });
}

/** Weave source-anchored records without changing canonical content ranges or groups. */
export function insertDelegationRecords<T extends ProcessSegmentIdentity>(items: T[], records: DelegationRecord[]): Array<T | DelegationProcessItem> {
  const result: Array<T | DelegationProcessItem> = [...items];
  const ordered = [...records].sort(compareDelegationRecords);
  for (const record of ordered.filter(record => !record.source_scoped_order)) {
    const entry: DelegationProcessItem = { type: 'delegation_record', id: `delegation:${record.id}`, records: [record], processSequence: record.anchor.seq };
    const next = result.findIndex(item => item.processSequence !== undefined && item.processSequence > record.anchor.seq);
    result.splice(next < 0 ? result.length : next, 0, entry);
  }
  // Historical source-only references cannot masquerade as an asynchronous Session position.
  for (const record of ordered.filter(record => record.source_scoped_order)) {
    result.push({ type: 'delegation_record', id: `delegation:${record.id}`, records: [record] });
  }
  return compactDelegationBatches(result);
}
