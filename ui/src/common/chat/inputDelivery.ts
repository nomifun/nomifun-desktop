import { parseMessageId, type MessageId } from '@/common/types/ids';
import type { SessionInputView } from '@/common/protocolBindings/SessionInputView';

/** Read-only progress and placement of one accepted input. Local submission
 * echoes have no host observation cursor; they never select execution. */
export type InputDelivery = Pick<SessionInputView, 'state'>
  & Partial<Omit<SessionInputView, 'state' | 'turn_id'>> & {
  turn_id?: MessageId;
};

export function normalizeInputDelivery(raw: unknown): InputDelivery | undefined {
  if (!raw || typeof raw !== 'object' || Array.isArray(raw)) return undefined;
  const value = raw as Record<string, unknown>;
  if (typeof value.state !== 'string' || !['pending', 'blocked', 'claimed', 'discarded', 'cancelled'].includes(value.state)) return undefined;
  if (value.event_seq !== undefined &&
      (typeof value.event_seq !== 'number' || !Number.isSafeInteger(value.event_seq) || value.event_seq <= 0)) return undefined;
  if (value.placement !== undefined && !['queued', 'steering', 'transcript'].includes(value.placement as string)) return undefined;
  if (value.kind !== undefined && !['root', 'steering'].includes(value.kind as string)) return undefined;
  if (value.order_seq !== undefined && (typeof value.order_seq !== 'number' || !Number.isSafeInteger(value.order_seq) || value.order_seq <= 0)) return undefined;
  let turnId: MessageId | undefined;
  if (value.turn_id !== undefined && value.turn_id !== null) {
    try { turnId = parseMessageId(value.turn_id); } catch { return undefined; }
  }
  return {
    state: value.state as InputDelivery['state'],
    ...(typeof value.reason === 'string' && /^[A-Z][A-Z0-9_]{0,127}$/.test(value.reason) ? { reason: value.reason } : {}),
    ...(typeof value.event_seq === 'number' ? { event_seq: value.event_seq } : {}),
    ...(value.placement ? { placement: value.placement as InputDelivery['placement'] } : {}),
    ...(value.kind ? { kind: value.kind as InputDelivery['kind'] } : {}),
    ...(turnId ? { turn_id: turnId } : {}),
    ...(typeof value.order_seq === 'number' ? { order_seq: value.order_seq } : {}),
  };
}

/** State names have no rank: a new claim may follow an earlier blocked input. */
export function preferInputDelivery(incoming: InputDelivery | undefined, existing: InputDelivery | undefined): InputDelivery | undefined {
  if (!incoming) return existing;
  if (!existing) return incoming;
  if (incoming.event_seq !== undefined && existing.event_seq !== undefined) {
    return incoming.event_seq > existing.event_seq ? incoming : existing;
  }
  if (existing.event_seq !== undefined) return existing;
  return incoming;
}
