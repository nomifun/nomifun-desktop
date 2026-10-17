import type { AgentTurnOutcome } from '../protocolBindings/AgentTurnOutcome';
import { parseMessageId } from '../types/ids';

/** A pause can resume in the same Turn; a committed terminal cannot become paused again. */
export function mergeAgentTurnOutcome(existing: AgentTurnOutcome | undefined, incoming: AgentTurnOutcome | undefined): AgentTurnOutcome | undefined {
  if (!incoming) return existing;
  return existing && existing.kind !== 'paused' && incoming.kind === 'paused' ? existing : incoming;
}

/** The host derives this receipt from canonical facts; text and row order do not author it. */
export function normalizeAgentTurnOutcome(value: unknown): AgentTurnOutcome | undefined {
  if (!value || typeof value !== 'object' || Array.isArray(value)) return;
  const row = value as Record<string, unknown>;
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
  if (row.kind === 'silent' || row.kind === 'unavailable') {
    return { kind: row.kind };
  }
}
