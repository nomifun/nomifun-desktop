import type { AgentTurnOutcome } from '../protocolBindings/AgentTurnOutcome';
import { parseMessageId } from '../types/ids';

/** A pause can resume in the same Turn; a committed terminal cannot become paused again. */
export function mergeAgentTurnOutcome(existing: AgentTurnOutcome | undefined, incoming: AgentTurnOutcome | undefined): AgentTurnOutcome | undefined {
  if (!incoming) return existing;
  const transient = (outcome: AgentTurnOutcome) => outcome.kind === 'paused' || outcome.kind === 'waiting';
  if (existing?.kind === 'waiting' && incoming.kind === 'waiting' && existing.wait_seq > incoming.wait_seq) return existing;
  return existing && !transient(existing) && transient(incoming) ? existing : incoming;
}

/** The host derives this receipt from canonical facts; text and row order do not author it. */
export function normalizeAgentTurnOutcome(value: unknown): AgentTurnOutcome | undefined {
  if (!value || typeof value !== 'object' || Array.isArray(value)) return;
  const row = value as Record<string, unknown>;
  if (row.kind === 'waiting') {
    if (Object.keys(row).some(key => !['kind', 'request_id', 'reason', 'wait_seq'].includes(key))
      || typeof row.wait_seq !== 'number' || !Number.isSafeInteger(row.wait_seq) || row.wait_seq <= 0
      || !['user_input', 'external_resource', 'intervention_required'].includes(row.reason as string)
      || row.request_id !== null && (typeof row.request_id !== 'string' || !row.request_id.trim()
        || new TextEncoder().encode(row.request_id).length > 256 || /[\u0000-\u001f\u007f-\u009f]/u.test(row.request_id))) return;
    return { kind: 'waiting', request_id: row.request_id as string | null, wait_seq: row.wait_seq,
      reason: row.reason as Extract<AgentTurnOutcome, { kind: 'waiting' }>['reason'] };
  }
  if (row.kind === 'answered' || row.kind === 'failed' || row.kind === 'cancelled' || row.kind === 'paused') {
    if (Object.keys(row).some(key => !['kind', 'message_ids'].includes(key)) || !Array.isArray(row.message_ids)
      || (row.kind === 'answered' && !row.message_ids.length)) return;
    try {
      const messageIds = row.message_ids.map(parseMessageId);
      if (new Set(messageIds).size !== messageIds.length) return;
      return { kind: row.kind, message_ids: messageIds };
    } catch { return; }
  }
  if (row.kind === 'input_required') {
    if (Object.keys(row).some(key => !['kind', 'request_id'].includes(key)) || typeof row.request_id !== 'string'
      || !row.request_id.trim() || new TextEncoder().encode(row.request_id).length > 256
      || /[\u0000-\u001f\u007f-\u009f]/u.test(row.request_id)) return;
    return { kind: 'input_required', request_id: row.request_id };
  }
  if (Object.keys(row).length !== 1) return;
  if (row.kind === 'unavailable') {
    return { kind: row.kind };
  }
}
