import type { ConversationId } from '@/common/types/ids';
import type { TChatConversation } from '@/common/config/storage';
import { getConversationOrNull } from '@/renderer/pages/conversation/utils/conversationCache';
import { getConversationRuntimeAuthority } from '@/renderer/pages/conversation/utils/conversationRuntime';
import { emitter } from '@/renderer/utils/emitter';

export const TERMINAL_RECONCILE_DELAYS_MS = [120, 400, 1_200, 3_000, 8_000, 16_000] as const;
export const AUTHORITATIVE_RUNTIME_RESYNC_DELAYS_MS = [
  0,
  ...TERMINAL_RECONCILE_DELAYS_MS,
] as const;
export const TERMINAL_RECONCILE_QUERY_TIMEOUT_MS = 3_000;

export const terminalReconcileDelayForAttempt = (
  attempt: number,
  delaysMs: readonly number[] = TERMINAL_RECONCILE_DELAYS_MS
): number => {
  const schedule = delaysMs.length > 0 ? delaysMs : TERMINAL_RECONCILE_DELAYS_MS;
  const boundedAttempt = Math.min(Math.max(0, Math.trunc(attempt)), schedule.length - 1);
  return schedule[boundedAttempt]!;
};

const getConversationWithTimeout = async (
  conversationId: ConversationId,
  getConversation: typeof getConversationOrNull,
  timeoutMs: number
) => {
  let timeout: ReturnType<typeof setTimeout> | undefined;
  try {
    return await Promise.race([
      getConversation(conversationId),
      new Promise<never>((_, reject) => {
        timeout = setTimeout(
          () => reject(new Error(`Conversation runtime reconciliation timed out after ${timeoutMs}ms`)),
          timeoutMs
        );
      }),
    ]);
  } finally {
    if (timeout) clearTimeout(timeout);
  }
};

type AuthoritativeRuntimeReconciliationOptions = {
  isCurrent: () => boolean;
  onIdle: (conversation: TChatConversation | null) => void;
  onProcessing?: (conversation: TChatConversation) => void;
  onUnknown?: (conversation: TChatConversation) => void;
  delaysMs?: readonly number[];
  getConversation?: typeof getConversationOrNull;
  queryTimeoutMs?: number;
  retryForever?: boolean;
  announceSettled?: boolean;
  logLabel?: string;
};

/**
 * Repeatedly snapshots the durable conversation runtime until it is
 * authoritatively idle or this lifecycle generation is superseded.
 *
 * The callback receives the full processing snapshot (including the exact
 * `active_turn_id`) so a renderer can recover correlation after a WebSocket
 * delivery gap. Network errors, query timeouts, and incomplete runtime
 * projections remain inside this bounded-backoff loop instead of escaping as
 * unhandled promise rejections.
 */
export const reconcileConversationAuthoritativeRuntime = async (
  conversationId: ConversationId,
  {
    isCurrent,
    onIdle,
    onProcessing,
    onUnknown,
    delaysMs = AUTHORITATIVE_RUNTIME_RESYNC_DELAYS_MS,
    getConversation = getConversationOrNull,
    queryTimeoutMs = TERMINAL_RECONCILE_QUERY_TIMEOUT_MS,
    retryForever = delaysMs === AUTHORITATIVE_RUNTIME_RESYNC_DELAYS_MS,
    announceSettled = true,
    logLabel = 'authoritative runtime',
  }: AuthoritativeRuntimeReconciliationOptions
): Promise<boolean> => {
  let attempt = 0;
  while (retryForever || attempt < delaysMs.length) {
    if (!isCurrent()) return false;
    const delayMs = terminalReconcileDelayForAttempt(attempt, delaysMs);
    attempt += 1;
    if (delayMs > 0) {
      await new Promise<void>((resolve) => setTimeout(resolve, delayMs));
    }
    if (!isCurrent()) return false;

    try {
      const conversation = await getConversationWithTimeout(
        conversationId,
        getConversation,
        queryTimeoutMs
      );
      if (!isCurrent()) return false;
      const runtimeAuthority = getConversationRuntimeAuthority(conversation);
      if (runtimeAuthority === 'processing') {
        if (conversation) onProcessing?.(conversation);
        continue;
      }
      if (runtimeAuthority === 'unknown') {
        if (conversation) onUnknown?.(conversation);
        continue;
      }

      onIdle(conversation);
      if (announceSettled) {
        // Durable projections also need a reload because the WebSocket terminal
        // frames may have been lost together with the lifecycle event.
        emitter.emit('conversation.turn.settled', conversationId);
      }
      return true;
    } catch (error) {
      console.warn(`[conversation-turn-lifecycle] Failed to reconcile ${logLabel}:`, error);
    }
  }
  return false;
};

/**
 * Reliability reconciliation for a lost turn.completed event. A stream terminal is
 * only a trigger for these reads; it never directly lowers the busy state.
 */
export const reconcileConversationTurnAfterStreamTerminal = async (
  conversationId: ConversationId,
  isCurrent: () => boolean,
  onIdle: () => void,
  delaysMs: readonly number[] = TERMINAL_RECONCILE_DELAYS_MS,
  getConversation: typeof getConversationOrNull = getConversationOrNull,
  queryTimeoutMs = TERMINAL_RECONCILE_QUERY_TIMEOUT_MS,
  retryForever = delaysMs === TERMINAL_RECONCILE_DELAYS_MS,
  onProcessing?: (conversation: TChatConversation) => void
): Promise<boolean> =>
  reconcileConversationAuthoritativeRuntime(conversationId, {
    isCurrent,
    onIdle: () => onIdle(),
    onProcessing,
    delaysMs,
    getConversation,
    queryTimeoutMs,
    retryForever,
    logLabel: 'terminal stream',
  });

/**
 * Reconcile an accepted replay without declaring a new local turn.
 *
 * Only a fresh runtime GET may reopen rendering/processing through
 * `onProcessing`; an idle snapshot settles immediately. Reads retry forever in
 * production so a lost response cannot strand either state.
 */
export const reconcileConversationTurnAfterAcceptedReplay = (
  conversationId: ConversationId,
  isCurrent: () => boolean,
  onProcessing: (conversation: TChatConversation) => void,
  onIdle: () => void,
  delaysMs: readonly number[] = TERMINAL_RECONCILE_DELAYS_MS,
  getConversation: typeof getConversationOrNull = getConversationOrNull,
  queryTimeoutMs = TERMINAL_RECONCILE_QUERY_TIMEOUT_MS,
  retryForever = delaysMs === TERMINAL_RECONCILE_DELAYS_MS
): Promise<boolean> =>
  reconcileConversationTurnAfterStreamTerminal(
    conversationId,
    isCurrent,
    onIdle,
    delaysMs,
    getConversation,
    queryTimeoutMs,
    retryForever,
    onProcessing
  );
