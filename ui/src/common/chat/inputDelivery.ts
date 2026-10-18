/** Read-only progress of one accepted input, ordered by canonical facts. */
export type InputDelivery = {
  state: 'pending' | 'blocked' | 'claimed' | 'discarded';
  reason?: string;
  /** Missing only on an optimistic local echo. */
  event_seq?: number;
};

export function normalizeInputDelivery(raw: unknown): InputDelivery | undefined {
  if (!raw || typeof raw !== 'object' || Array.isArray(raw)) return undefined;
  const value = raw as Record<string, unknown>;
  if (typeof value.state !== 'string' || !['pending', 'blocked', 'claimed', 'discarded'].includes(value.state)) return undefined;
  if (value.event_seq !== undefined &&
      (typeof value.event_seq !== 'number' || !Number.isSafeInteger(value.event_seq) || value.event_seq <= 0)) return undefined;
  return {
    state: value.state as InputDelivery['state'],
    ...(typeof value.reason === 'string' && /^[A-Z][A-Z0-9_]{0,127}$/.test(value.reason) ? { reason: value.reason } : {}),
    ...(typeof value.event_seq === 'number' ? { event_seq: value.event_seq } : {}),
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
