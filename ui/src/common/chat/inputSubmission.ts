import { parseMessageId, type MessageId } from '@/common/types/ids';
import type { SessionInputDeliveryIntent } from '@/common/protocolBindings/SessionInputDeliveryIntent';

export type { SessionInputDeliveryIntent } from '@/common/protocolBindings/SessionInputDeliveryIntent';

export const QUEUED_INPUT: SessionInputDeliveryIntent = Object.freeze({ kind: 'queue' });

export function normalizeDeliveryIntent(value: unknown): SessionInputDeliveryIntent | undefined {
  if (!value || typeof value !== 'object' || Array.isArray(value)) return undefined;
  const raw = value as Record<string, unknown>;
  if (raw.kind === 'queue' && Object.keys(raw).length === 1) return QUEUED_INPUT;
  if (raw.kind !== 'prefer_current' || Object.keys(raw).length !== 2) return undefined;
  try { return { kind: 'prefer_current', turn_id: parseMessageId(raw.turn_id) }; } catch { return undefined; }
}

/** Guiding expresses one exact observed target; the Host owns admission. */
export function guidedInput(observedTurn: MessageId): SessionInputDeliveryIntent {
  return Object.freeze({ kind: 'prefer_current', turn_id: observedTurn });
}
