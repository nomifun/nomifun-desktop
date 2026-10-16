import type { MessageTimeline } from '../protocolBindings/MessageTimeline';
import type { MessageTimelineUpdate } from '../protocolBindings/MessageTimelineUpdate';
import type { TMessage } from './chatLib';

const record = (value: unknown): value is Record<string, unknown> =>
  !!value && typeof value === 'object' && !Array.isArray(value);
const sequence = (value: unknown): value is number => Number.isSafeInteger(value) && (value as number) >= 0;

/** Validate display references only. These are never execution or permission receipts. */
export function normalizeMessageTimeline(value: unknown): MessageTimeline | undefined {
  if (!record(value) || !sequence(value.through_seq) || !Array.isArray(value.segments)) return undefined;
  const ids = new Set<string>();
  for (const segment of value.segments) {
    if (!record(segment) || typeof segment.segment_id !== 'string' || !segment.segment_id
      || typeof segment.group_id !== 'string' || !segment.group_id
      || !sequence(segment.first_seq) || !sequence(segment.last_seq)
      || segment.first_seq > segment.last_seq || segment.last_seq > value.through_seq
      || !['reasoning', 'narration', 'tool'].includes(String(segment.kind))
      || !['running', 'completed', 'failed', 'cancelled', 'interrupted'].includes(String(segment.state))
      || ids.has(segment.segment_id)) return undefined;
    ids.add(segment.segment_id);
    if (segment.kind === 'tool') {
      if (typeof segment.call_id !== 'string' || !segment.call_id) return undefined;
    } else if (!sequence(segment.start_utf16) || !sequence(segment.end_utf16) || segment.start_utf16 > segment.end_utf16) return undefined;
  }
  return value as unknown as MessageTimeline;
}

export function preferMessageTimeline(a?: MessageTimeline, b?: MessageTimeline): MessageTimeline | undefined {
  return !a ? b : !b || a.through_seq >= b.through_seq ? a : b;
}

export type TimelineNotification = {
  type: 'canonical_timeline'; conversation_id: string; turn_id: string; updates: unknown;
};

export function applyCanonicalTimeline(list: TMessage[], notification: TimelineNotification): TMessage[] {
  if (!Array.isArray(notification.updates)) return list;
  const updates = notification.updates.filter(record).flatMap(update => {
    const timeline = normalizeMessageTimeline(update.timeline);
    return timeline && typeof update.source_message_id === 'string'
      ? [{ ...update, timeline } as unknown as MessageTimelineUpdate] : [];
  });
  let changed = false;
  const next = list.map(message => {
    if (message.conversation_id !== notification.conversation_id || message.turn_id !== notification.turn_id
      || !['text', 'thinking', 'tool_call'].includes(message.type)) return message;
    const update = updates.find(item => message.type === 'tool_call'
      ? item.call_id === message.content.call_id
      : item.source_message_id === (message.message_id ?? message.msg_id));
    if (!update || preferMessageTimeline(message.content.timeline, update.timeline) !== update.timeline) return message;
    changed = true;
    return { ...message, content: { ...message.content, timeline: update.timeline } } as TMessage;
  });
  return changed ? next : list;
}
