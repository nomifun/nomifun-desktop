import type { IResponseMessage } from '@/common/adapter/ipcBridge';
import type { TChatConversation } from '@/common/config/storage';

export type ConversationRuntimeAuthority = 'idle' | 'processing' | 'unknown';

export const getConversationPauseNotice = (conversation?: Pick<TChatConversation, 'runtime' | 'status' | 'extra'> | null) => {
  const runtime = conversation?.runtime;
  if (conversation?.status !== 'running' || conversation.extra?.execution_phase !== 'paused'
    || runtime?.state !== 'idle' || runtime.is_processing !== false
    || runtime.can_send_message !== false || !runtime.active_turn_id) return null;
  const pause = conversation.extra.execution_pause;
  return {
    turnId: runtime.active_turn_id,
    reason: typeof pause?.reason === 'string' && /^[A-Z][A-Z0-9_]{0,127}$/.test(pause.reason) ? pause.reason : undefined,
    cleanupProven: pause?.cleanup_proven === true,
    pausedAt: typeof pause?.paused_at_ms === 'number' && Number.isFinite(pause.paused_at_ms) && pause.paused_at_ms > 0
      ? pause.paused_at_ms : undefined,
  };
};

export type ConversationPauseNotice = NonNullable<ReturnType<typeof getConversationPauseNotice>>;

export const getConversationRuntimeAuthority = (
  conversation?: Pick<TChatConversation, 'runtime' | 'status'> | null
): ConversationRuntimeAuthority => {
  if (!conversation) return 'idle';

  // The durable aggregate terminal status dominates stale process projections:
  // a Finished conversation can never be raised back to Running by UI state.
  if (conversation.status === 'finished') return 'idle';

  if (conversation.status === 'running') {
    return conversation.runtime?.is_processing === true &&
      conversation.runtime.active_turn_id != null
      ? 'processing'
      : 'unknown';
  }

  if (conversation.status === 'pending') {
    return conversation.runtime?.is_processing === true ||
      conversation.runtime?.active_turn_id != null
      ? 'unknown'
      : 'idle';
  }

  // Legacy/malformed snapshots are never authority to start or settle a turn.
  return 'unknown';
};

export const isConversationProcessing = (conversation?: Pick<TChatConversation, 'runtime' | 'status'> | null) => {
  return getConversationRuntimeAuthority(conversation) === 'processing';
};

/** A complete projection is delivered over `message.stream` for realtime
 * rendering, but it does not own a model turn and intentionally has no later
 * `finish` / `turn.completed` event. */
export const isCompleteMessageProjection = (
  message?: Pick<IResponseMessage, 'stream_complete'> | null
): boolean => message?.stream_complete === true;
