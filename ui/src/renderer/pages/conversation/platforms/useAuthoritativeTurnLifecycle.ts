import { ipcBridge } from '@/common';
import type { TChatConversation } from '@/common/config/storage';
import type { ConversationId, MessageId } from '@/common/types/ids';
import { getConversationOrNull } from '@/renderer/pages/conversation/utils/conversationCache';
import { isConversationProcessing } from '@/renderer/pages/conversation/utils/conversationRuntime';
import { useCallback, useEffect, useRef } from 'react';
import {
  AUTHORITATIVE_RUNTIME_RESYNC_DELAYS_MS,
  reconcileConversationTurnAfterAcceptedReplay,
  reconcileConversationAuthoritativeRuntime,
  reconcileConversationTurnAfterStreamTerminal,
  TERMINAL_RECONCILE_DELAYS_MS,
} from './reconcileConversationTurnAfterStreamTerminal';
import {
  classifyAuthoritativeTurnCompletion,
  classifyAuthoritativeTurnStart,
  isAuthoritativeCompletionRuntimeIdle,
  resolveVerifiedAuthoritativeTurnStart,
} from './authoritativeTurnLifecyclePolicy';

type AuthoritativeTurnLifecycleOptions = {
  onTurnStarted?: () => void;
  onTurnCompleted: () => void;
  onAuthoritativeRuntime?: (
    isRunning: boolean,
    conversation: TChatConversation | null
  ) => void;
};

export type AuthoritativeHydrationFence = {
  closed: boolean;
  verifyUnannouncedStartRuntime: boolean;
};

export const getAuthoritativeHydrationFence = (
  isRunning: boolean
): AuthoritativeHydrationFence => ({
  closed: !isRunning,
  verifyUnannouncedStartRuntime: true,
});

export const shouldAcceptAuthoritativeStreamActivity = ({
  closed,
  awaitingBackendTurn,
  activeTurnId,
  eventTurnId,
}: {
  closed: boolean;
  awaitingBackendTurn: boolean;
  activeTurnId?: MessageId | null;
  eventTurnId?: MessageId;
}): boolean => {
  if (closed && !awaitingBackendTurn) return false;
  if (activeTurnId || eventTurnId) {
    return Boolean(activeTurnId && eventTurnId && activeTurnId === eventTurnId);
  }
  return awaitingBackendTurn;
};

/**
 * Owns the generation/correlation bookkeeping shared by the simple composer
 * surfaces. Stream `finish`/`error` events close message rendering only; the UI
 * busy state is lowered exclusively by the conversation-scoped turn.completed
 * event after the backend has released its turn handle.
 */
export const useAuthoritativeTurnLifecycle = (
  conversationId: ConversationId,
  {
    onTurnStarted,
    onTurnCompleted,
    onAuthoritativeRuntime,
  }: AuthoritativeTurnLifecycleOptions
) => {
  const onTurnStartedRef = useRef(onTurnStarted);
  const onTurnCompletedRef = useRef(onTurnCompleted);
  const onAuthoritativeRuntimeRef = useRef(onAuthoritativeRuntime);
  const rootTurnIdRef = useRef<MessageId | null>(null);
  const awaitingBackendTurnRef = useRef(false);
  const closedRef = useRef(true);
  const generationRef = useRef(0);
  const turnStartGenerationRef = useRef(0);
  const turnCompletionGenerationRef = useRef(0);
  const reconcileSequenceRef = useRef(0);
  const cancelledTurnIdsRef = useRef(new Set<MessageId>());
  const rejectUnannouncedStartRef = useRef(false);
  const verifyUnannouncedStartRuntimeRef = useRef(true);
  const mountedRef = useRef(true);

  useEffect(() => {
    mountedRef.current = true;
    return () => {
      mountedRef.current = false;
      generationRef.current += 1;
      reconcileSequenceRef.current += 1;
    };
  }, []);

  useEffect(() => {
    onTurnStartedRef.current = onTurnStarted;
    onTurnCompletedRef.current = onTurnCompleted;
    onAuthoritativeRuntimeRef.current = onAuthoritativeRuntime;
  }, [onAuthoritativeRuntime, onTurnCompleted, onTurnStarted]);

  useEffect(() => {
    rootTurnIdRef.current = null;
    awaitingBackendTurnRef.current = false;
    // A remounted/switching view starts behind an idle fence until its fresh
    // runtime snapshot proves that a turn is actually active. Otherwise a
    // delayed stream frame or turn.started from the already-completed turn can
    // advance the generation first and make the later idle snapshot look stale.
    closedRef.current = true;
    generationRef.current += 1;
    reconcileSequenceRef.current += 1;
    cancelledTurnIdsRef.current.clear();
    rejectUnannouncedStartRef.current = false;
    verifyUnannouncedStartRuntimeRef.current = true;
  }, [conversationId]);

  const beginLocalTurn = useCallback(() => {
    turnStartGenerationRef.current += 1;
    generationRef.current += 1;
    rootTurnIdRef.current = null;
    awaitingBackendTurnRef.current = true;
    closedRef.current = false;
    rejectUnannouncedStartRef.current = false;
    // Local intent opens rendering, but it does not identify the backend turn.
    // Even the next start event must prove exact active_turn_id ownership.
    verifyUnannouncedStartRuntimeRef.current = true;
    reconcileSequenceRef.current += 1;
  }, []);

  const cancelLocalTurn = useCallback(() => {
    generationRef.current += 1;
    rootTurnIdRef.current = null;
    awaitingBackendTurnRef.current = false;
    closedRef.current = true;
    rejectUnannouncedStartRef.current = false;
    verifyUnannouncedStartRuntimeRef.current = true;
    reconcileSequenceRef.current += 1;
  }, []);

  const stopOptimistically = useCallback(() => {
    generationRef.current += 1;
    const rootTurnId = rootTurnIdRef.current;
    if (rootTurnId) {
      const cancelled = cancelledTurnIdsRef.current;
      cancelled.add(rootTurnId);
      if (cancelled.size > 32) {
        const oldest = cancelled.values().next().value;
        if (oldest) cancelled.delete(oldest);
      }
    }
    awaitingBackendTurnRef.current = false;
    closedRef.current = true;
    rejectUnannouncedStartRef.current = true;
    verifyUnannouncedStartRuntimeRef.current = rootTurnId === null;
    reconcileSequenceRef.current += 1;
  }, []);

  const acceptsStreamActivity = useCallback(
    (eventTurnId?: MessageId) =>
      shouldAcceptAuthoritativeStreamActivity({
        closed: closedRef.current,
        awaitingBackendTurn: awaitingBackendTurnRef.current,
        activeTurnId: rootTurnIdRef.current,
        eventTurnId,
      }),
    []
  );

  const settle = useCallback((expectedGeneration: number, conversation: TChatConversation | null = null) => {
    if (!mountedRef.current || generationRef.current !== expectedGeneration) return;
    generationRef.current += 1;
    turnCompletionGenerationRef.current += 1;
    reconcileSequenceRef.current += 1;
    rootTurnIdRef.current = null;
    awaitingBackendTurnRef.current = false;
    closedRef.current = true;
    rejectUnannouncedStartRef.current = false;
    verifyUnannouncedStartRuntimeRef.current = true;
    onTurnCompletedRef.current();
    onAuthoritativeRuntimeRef.current?.(false, conversation);
  }, []);

  const confirmStopped = useCallback(() => {
    generationRef.current += 1;
    reconcileSequenceRef.current += 1;
    rootTurnIdRef.current = null;
    awaitingBackendTurnRef.current = false;
    closedRef.current = true;
    rejectUnannouncedStartRef.current = false;
    verifyUnannouncedStartRuntimeRef.current = true;
    onTurnCompletedRef.current();
  }, []);

  /**
   * Apply the fresh runtime snapshot owned by the current hydration generation.
   * Idle remains closed; a proven running snapshot may accept stream activity,
   * while its still-unknown turn id keeps turn.started behind runtime
   * verification.
   */
  const hydrateAuthoritativeRuntime = useCallback((isRunning: boolean) => {
    const fence = getAuthoritativeHydrationFence(isRunning);
    rootTurnIdRef.current = null;
    awaitingBackendTurnRef.current = false;
    closedRef.current = fence.closed;
    rejectUnannouncedStartRef.current = false;
    verifyUnannouncedStartRuntimeRef.current = fence.verifyUnannouncedStartRuntime;
  }, []);

  const adoptAuthoritativeProcessing = useCallback((conversation: TChatConversation) => {
    const activeTurnId = conversation.runtime?.active_turn_id;
    if (
      !activeTurnId ||
      rejectUnannouncedStartRef.current ||
      cancelledTurnIdsRef.current.has(activeTurnId)
    ) {
      return false;
    }

    const changedTurn = rootTurnIdRef.current !== activeTurnId;
    const shouldRaiseRunning =
      changedTurn || closedRef.current || awaitingBackendTurnRef.current;
    if (changedTurn) turnStartGenerationRef.current += 1;
    rootTurnIdRef.current = activeTurnId;
    awaitingBackendTurnRef.current = false;
    closedRef.current = false;
    rejectUnannouncedStartRef.current = false;
    verifyUnannouncedStartRuntimeRef.current = false;
    if (shouldRaiseRunning) onTurnStartedRef.current?.();
    onAuthoritativeRuntimeRef.current?.(true, conversation);
    return true;
  }, []);

  const reconcileGeneration = useCallback(
    (
      generation: number,
      delaysMs: readonly number[] = TERMINAL_RECONCILE_DELAYS_MS,
      isExpected: () => boolean = () => true,
      announceSettled = true
    ) => {
      const sequence = reconcileSequenceRef.current + 1;
      reconcileSequenceRef.current = sequence;
      let transferredAuthority = false;
      void reconcileConversationAuthoritativeRuntime(conversationId, {
        isCurrent: () =>
          mountedRef.current &&
          generationRef.current === generation &&
          reconcileSequenceRef.current === sequence &&
          (transferredAuthority || isExpected()),
        onIdle: (conversation) => settle(generation, conversation),
        onProcessing: (conversation) => {
          transferredAuthority =
            adoptAuthoritativeProcessing(conversation) || transferredAuthority;
        },
        delaysMs,
        retryForever: true,
        announceSettled,
      });
    },
    [adoptAuthoritativeProcessing, conversationId, settle]
  );

  const resyncAuthoritativeRuntime = useCallback(
    ({
      immediate = false,
      announceSettled = true,
    }: {
      immediate?: boolean;
      announceSettled?: boolean;
    } = {}) => {
      reconcileGeneration(
        generationRef.current,
        immediate
          ? AUTHORITATIVE_RUNTIME_RESYNC_DELAYS_MS
          : TERMINAL_RECONCILE_DELAYS_MS,
        () => true,
        announceSettled
      );
    },
    [reconcileGeneration]
  );

  const restoreAfterStopFailure = useCallback(() => {
    generationRef.current += 1;
    const rootTurnId = rootTurnIdRef.current;
    if (rootTurnId) cancelledTurnIdsRef.current.delete(rootTurnId);
    awaitingBackendTurnRef.current = false;
    closedRef.current = false;
    rejectUnannouncedStartRef.current = false;
    verifyUnannouncedStartRuntimeRef.current = false;
    // A null-turn completion may already be awaiting its own GET when the stop
    // request fails. Restoring advances the lifecycle generation, so start a
    // fresh authoritative reconciliation instead of losing that idle signal.
    reconcileGeneration(generationRef.current);
  }, [reconcileGeneration]);

  const markLocalTurnAccepted = useCallback(
    () => {
      if (!awaitingBackendTurnRef.current || closedRef.current) return;
      // Do not invalidate an in-flight runtime verification inherited from an
      // unknown-root stop. The verified start itself advances the generation.
      if (!verifyUnannouncedStartRuntimeRef.current) generationRef.current += 1;
      reconcileSequenceRef.current += 1;
      // The POST returns the user-message id, while wire turn_id identifies the
      // first assistant turn message. Correlation is established exclusively by
      // turn.started; acceptance only unlocks authoritative runtime recovery.
      rootTurnIdRef.current = null;
      awaitingBackendTurnRef.current = false;
      const generation = generationRef.current;
      // Covers a fast turn whose started/completed or terminal stream events
      // raced the HTTP result. Use the same bounded long-tail window as terminal
      // reconciliation so slow DB/receipt finalization cannot strand busy state.
      reconcileGeneration(generation);
    },
    [reconcileGeneration]
  );

  /**
   * Replace the optimistic submit state with the durable replay receipt.
   *
   * Completed receipts close synchronously. Accepted receipts also start
   * closed and may reopen only after a fresh conversation GET proves the
   * original turn is still processing.
   */
  const reconcilePublicDeliveryReplay = useCallback(
    (completed: boolean) => {
      generationRef.current += 1;
      reconcileSequenceRef.current += 1;
      rootTurnIdRef.current = null;
      awaitingBackendTurnRef.current = false;
      closedRef.current = true;
      rejectUnannouncedStartRef.current = false;
      verifyUnannouncedStartRuntimeRef.current = true;
      onTurnCompletedRef.current();

      if (completed) {
        turnCompletionGenerationRef.current += 1;
        return;
      }

      const generation = generationRef.current;
      const sequence = reconcileSequenceRef.current;
      let observedProcessing = false;
      void reconcileConversationTurnAfterAcceptedReplay(
        conversationId,
        () =>
          mountedRef.current &&
          generationRef.current === generation &&
          reconcileSequenceRef.current === sequence,
        (conversation) => {
          if (observedProcessing) return;
          observedProcessing = adoptAuthoritativeProcessing(conversation);
        },
        () => settle(generation)
      );
    },
    [adoptAuthoritativeProcessing, conversationId, settle]
  );

  useEffect(() => {
    let disposed = false;
    const unsubscribe = ipcBridge.conversation.turnStarted.on((event) => {
      if (event.conversation_id !== conversationId) return;
      const startAction = classifyAuthoritativeTurnStart({
        turnId: event.turn_id,
        activeTurnId: rootTurnIdRef.current,
        cancelledTurnIds: cancelledTurnIdsRef.current,
        rejectUnannouncedStart: rejectUnannouncedStartRef.current,
        awaitingBackendTurn: awaitingBackendTurnRef.current,
        verifyUnannouncedStartRuntime: verifyUnannouncedStartRuntimeRef.current,
      });
      if (startAction === 'ignore') return;

      const acceptStart = () => {
        turnStartGenerationRef.current += 1;
        generationRef.current += 1;
        reconcileSequenceRef.current += 1;
        rootTurnIdRef.current = event.turn_id;
        awaitingBackendTurnRef.current = false;
        closedRef.current = false;
        rejectUnannouncedStartRef.current = false;
        verifyUnannouncedStartRuntimeRef.current = false;
        onTurnStartedRef.current?.();
        // turn.started advances the generation, so explicitly transfer
        // authoritative polling ownership to that new generation.
        reconcileGeneration(generationRef.current);
      };

      if (startAction === 'accept') {
        acceptStart();
        return;
      }

      const generation = generationRef.current;
      void getConversationOrNull(conversationId)
        .then((conversation) => {
          if (
            disposed ||
            generationRef.current !== generation ||
            !verifyUnannouncedStartRuntimeRef.current ||
            resolveVerifiedAuthoritativeTurnStart({
              turnId: event.turn_id,
              runtimeIsProcessing: isConversationProcessing(conversation),
              eventActiveTurnId: event.runtime.active_turn_id,
              runtimeActiveTurnId: conversation?.runtime?.active_turn_id,
            }) !== 'accept'
          ) {
            return;
          }
          acceptStart();
        })
        .catch((error) => {
          if (disposed) return;
          console.warn('[conversation-turn-lifecycle] Failed to verify unannounced turn start:', error);
        });
    });
    return () => {
      disposed = true;
      unsubscribe();
    };
  }, [conversationId, reconcileGeneration]);

  useEffect(() => {
    return ipcBridge.conversation.reconnected.on(() => {
      // The server has no event replay. Snapshot immediately and either adopt
      // the exact active turn or settle a terminal event lost in the gap.
      resyncAuthoritativeRuntime({ immediate: true });
    });
  }, [resyncAuthoritativeRuntime]);

  useEffect(() => {
    const unsubscribe = ipcBridge.conversation.turnCompleted.on((event) => {
      if (
        event.conversation_id !== conversationId ||
        !isAuthoritativeCompletionRuntimeIdle(event.runtime)
      ) {
        return;
      }

      const generation = generationRef.current;
      const rootTurnId = rootTurnIdRef.current;
      const awaitingBackendTurn = awaitingBackendTurnRef.current;
      const action = classifyAuthoritativeTurnCompletion({
        rootTurnId,
        completedTurnId: event.turn_id,
        awaitingBackendTurn,
      });
      if (action === 'settle') {
        settle(generation);
        return;
      }
      if (action === 'ignore') return;

      reconcileGeneration(generation, undefined, () =>
        rootTurnIdRef.current === rootTurnId &&
        awaitingBackendTurnRef.current === awaitingBackendTurn
      );
    });

    return () => {
      unsubscribe();
    };
  }, [conversationId, reconcileGeneration, settle]);

  const reconcileAfterStreamTerminal = useCallback(() => {
    const generation = generationRef.current;
    if (awaitingBackendTurnRef.current) return;
    reconcileGeneration(generation);
  }, [reconcileGeneration]);

  const getTurnStartGeneration = useCallback(() => turnStartGenerationRef.current, []);
  const getTurnCompletionGeneration = useCallback(() => turnCompletionGenerationRef.current, []);
  const getTurnLifecycleGeneration = useCallback(() => generationRef.current, []);

  return {
    beginLocalTurn,
    markLocalTurnAccepted,
    reconcilePublicDeliveryReplay,
    cancelLocalTurn,
    stopOptimistically,
    confirmStopped,
    restoreAfterStopFailure,
    hydrateAuthoritativeRuntime,
    resyncAuthoritativeRuntime,
    acceptsStreamActivity,
    reconcileAfterStreamTerminal,
    getTurnStartGeneration,
    getTurnCompletionGeneration,
    getTurnLifecycleGeneration,
  };
};
