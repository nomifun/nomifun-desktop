import type { TMessage } from '@/common/chat/chatLib';
import type { TurnDisclosureProcessState } from './turnDisclosureModel';

import type { MessageTimeline } from '@/common/protocolBindings/MessageTimeline';
export type ProcessSegmentIdentity = {
  processGroupId?: string;
  processSequence?: number;
  processSegmentState?: TurnDisclosureProcessState;
};
export type ProcessSegmentMessage = TMessage & ProcessSegmentIdentity;

/** References are authored from canonical content events. Missing metadata never
 * grants a renderer permission to guess phase boundaries from neighbouring rows. */
export function projectProcessSegments(messages: TMessage[], publicSources: ReadonlySet<string>): ProcessSegmentMessage[] {
  return messages.flatMap((message): ProcessSegmentMessage[] => {
    const content = message.content as unknown;
    const timeline = content && !Array.isArray(content) && typeof content === 'object'
      ? (content as { timeline?: MessageTimeline }).timeline : undefined;
    if (message.type === 'text' && message.position === 'right') return [message];
    if (timeline?.segments.length === 0) return [];
    if (message.type === 'text' && publicSources.has(message.message_id ?? message.msg_id ?? '')) return [message];
    if (!timeline) return [message];
    return timeline.segments.flatMap((segment, index): ProcessSegmentMessage[] => {
      if (!segment.segment_id || !segment.group_id || !Number.isSafeInteger(segment.first_seq)) return [];
      const state: TurnDisclosureProcessState = segment.state === 'cancelled' ? 'canceled'
        : segment.state === 'interrupted' ? 'completed' : segment.state;
      const identity = { id: segment.segment_id, processGroupId: segment.kind === 'narration' ? undefined : segment.group_id,
        processSequence: segment.first_seq, processSegmentState: state };
      if (segment.kind === 'tool') {
        if (message.type !== 'tool_call' || (segment.call_id && segment.call_id !== message.content.call_id)) return [];
        return [{ ...message, ...identity }];
      }
      if (message.type !== 'text' && message.type !== 'thinking') return [];
      if ((segment.kind === 'reasoning') !== (message.type === 'thinking')) return [];
      if (!Number.isSafeInteger(segment.start_utf16) || !Number.isSafeInteger(segment.end_utf16)) return [];
      const start = segment.start_utf16!;
      const end = segment.state === 'running' && index === timeline.segments.length - 1 ? Math.max(segment.end_utf16!, message.content.content.length) : segment.end_utf16!;
      if (start < 0 || end < start || start > message.content.content.length) return [];
      const text = message.content.content.slice(start, end);
      if (!text.trim() && segment.kind !== 'reasoning') return [];
      return [{ ...message, ...identity, content: { ...message.content, content: text,
        ...(message.type === 'thinking' ? { status: state === 'running' ? 'thinking' : 'done' } : {}) } } as ProcessSegmentMessage];
    });
  });
}

/** Sort only canonical runs. Unknown rows keep their own place and never create
 * a synthetic group or an ordering authority for the known references. */
export function orderProcessSegments<T extends ProcessSegmentIdentity>(items: T[]): T[] {
  const ordered: T[] = [];
  let run: T[] = [];
  const flush = () => { ordered.push(...run.sort((a, b) => a.processSequence! - b.processSequence!)); run = []; };
  for (const item of items) {
    if (item.processSequence !== undefined) run.push(item);
    else { flush(); ordered.push(item); }
  }
  flush();
  return ordered;
}

/** Missing result evidence ends the display observation, without asserting a
 * tool outcome or authorizing another invocation. */
export function isToolObservationUnconfirmed(message: TMessage, callId: string): boolean {
  return message.type === 'tool_call' && message.content.call_id === callId &&
    message.content.timeline?.segments.some(segment => segment.kind === 'tool' &&
      segment.call_id === callId && segment.state === 'interrupted') === true;
}
