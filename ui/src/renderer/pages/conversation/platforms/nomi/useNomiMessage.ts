/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import type { ConversationId, MessageId } from '@/common/types/ids';
import { ipcBridge } from '@/common';
import { transformMessage, transformUserCreatedEvent } from '@/common/chat/chatLib';
import { isToolGroupStatusActive, normalizeToolGroupStatus } from '@/common/chat/toolGroupStatus';
import { optionalDisplayText, toDisplayText } from '@/common/chat/displayText';
import type { IResponseMessage } from '@/common/adapter/ipcBridge';
import type { TChatConversation, TokenUsageData } from '@/common/config/storage';
import { mergeFetchedMessagesForConversation, normalizeDbMessage, useAddOrUpdateMessage, useUpdateMessageList } from '@/renderer/pages/conversation/Messages/hooks';
import { getConversationOrNull } from '@/renderer/pages/conversation/utils/conversationCache';
import {
  isCompleteMessageProjection,
  isConversationProcessing,
  getConversationPauseNotice,
  type ConversationPauseNotice,
} from '@/renderer/pages/conversation/utils/conversationRuntime';
import { useCallback, useEffect, useMemo, useReducer, useRef, useState } from 'react';
import type { ThoughtData } from '../thoughtTypes';
import {
  AUTHORITATIVE_RUNTIME_RESYNC_DELAYS_MS,
  reconcileConversationAuthoritativeRuntime,
  reconcileConversationTurnAfterStreamTerminal,
  TERMINAL_RECONCILE_DELAYS_MS,
} from '../reconcileConversationTurnAfterStreamTerminal';
import {
  classifyAuthoritativeTurnCompletion,
  classifyAuthoritativeTurnStart,
  isAuthoritativeCompletionRuntimeIdle,
  resolveVerifiedAuthoritativeTurnStart,
} from '../authoritativeTurnLifecyclePolicy';
import {
  getNomiHydrationLifecycleFence,
  shouldApplyNomiStreamEventToTurn,
} from './nomiLifecycleFence';
import { initialNomiTurnState, isTurnRunning, nomiTurnReducer, type NomiTurnEvent } from './nomiTurnState';

type NomiToolGroupRuntimeTool = {
  status: ReturnType<typeof normalizeToolGroupStatus>;
  name?: string;
  description?: string;
};

export const getNomiToolGroupRuntimeState = (data: unknown): {
  tools: NomiToolGroupRuntimeTool[];
  hasActive: boolean;
  hasAny: boolean;
  executingDescription?: string;
} => {
  const tools = Array.isArray(data)
    ? data
        .filter((item): item is Record<string, unknown> => !!item && typeof item === 'object' && !Array.isArray(item))
        .map((tool) => ({
          status: normalizeToolGroupStatus(tool.status),
          ...(tool.name != null ? { name: toDisplayText(tool.name) } : {}),
          ...(tool.description != null ? { description: toDisplayText(tool.description) } : {}),
        }))
    : [];
  const hasActive = tools.some((tool) => isToolGroupStatusActive(tool.status));
  const executingTool = tools.find((tool) => tool.status === 'Executing');

  return {
    tools,
    hasActive,
    hasAny: tools.length > 0,
    executingDescription: executingTool
      ? optionalDisplayText(executingTool.description) || optionalDisplayText(executingTool.name) || 'Tool'
      : undefined,
  };
};

const normalizeThoughtData = (data: unknown): ThoughtData => {
  if (!data || typeof data !== 'object' || Array.isArray(data)) {
    return { subject: '', description: toDisplayText(data) };
  }
  const record = data as Record<string, unknown>;
  return {
    subject: record.subject != null ? toDisplayText(record.subject) : '',
    description: record.description != null ? toDisplayText(record.description) : '',
  };
};

export const useNomiMessage = (
  conversation_id: ConversationId,
  options?: {
    onError?: (message: IResponseMessage) => void;
  }
) => {
  const onError = options?.onError;
  const addOrUpdateMessage = useAddOrUpdateMessage();
  const updateMessageList = useUpdateMessageList();
  // Single source of truth for the turn's activity state (design §3.2): a pure
  // reducer over lifecycle events replaces three hand-synced booleans.
  const [turnState, dispatchTurn] = useReducer(nomiTurnReducer, initialNomiTurnState);
  const [hasHydratedRunningState, setHasHydratedRunningState] = useState(false);
  const [thought, setThought] = useState<ThoughtData>({
    description: '',
    subject: '',
  });
  const [tokenUsage, setTokenUsage] = useState<TokenUsageData | null>(null);
  // Set when the user stops the active turn; MessageList pins the tail
  // disclosure to this moment ("you stopped after {duration}"). Session-local.
  const [stopNotice, setStopNotice] = useState<{ stoppedAt: number } | null>(null);
  const [pauseNotice, setPauseNotice] = useState<ConversationPauseNotice | null>(null);
  // Current active message ID to filter out events from old requests (prevents aborted request events from interfering with new ones)
  const activeMsgIdRef = useRef<MessageId | null>(null);
  const rootTurnIdRef = useRef<MessageId | null>(null);
  const [activeTurnId, setActiveTurnId] = useState<MessageId | null>(null);
  const [activeRequestMessageId, setActiveRequestMessageId] = useState<MessageId | null>(null);
  // Publish the same verified identity used by stream fencing to the renderer.
  // Ref-only identity cannot notify the timeline when hydration/start replaces
  // its provisional request boundary or a delayed row splits the active Turn.
  const setRootTurnId = useCallback((turnId: MessageId | null) => {
    rootTurnIdRef.current = turnId;
    setActiveTurnId(turnId);
  }, []);
  const awaitingBackendTurnRef = useRef(false);
  const turnClosedRef = useRef(false);
  const cancelledTurnIdsRef = useRef(new Set<MessageId>());
  const rejectUnannouncedStartRef = useRef(false);
  // Mount behind exact runtime verification so a synchronously replayed old
  // turn.started event cannot win before the hydration effect installs its
  // Finished/idle fence.
  const verifyUnannouncedStartRuntimeRef = useRef(true);
  const turnLifecycleGenerationRef = useRef(0);
  const turnStartGenerationRef = useRef(0);
  const turnCompletionGenerationRef = useRef(0);
  const turnReconcileSequenceRef = useRef(0);
  const mountedRef = useRef(true);
  const turnSettledRef = useRef(true);
  useEffect(() => {
    mountedRef.current = true;
    return () => {
      mountedRef.current = false;
      turnLifecycleGenerationRef.current += 1;
      turnReconcileSequenceRef.current += 1;
    };
  }, []);

  // Mirror the reducer state into a ref so the (non-resubscribing) stream
  // closure can read the current turn state without being a dependency.
  const turnStateRef = useRef(turnState);
  useEffect(() => {
    turnStateRef.current = turnState;
  }, [turnState]);

  // Throttle thought updates to reduce render frequency
  const thoughtThrottleRef = useRef<{
    lastUpdate: number;
    pending: ThoughtData | null;
    timer: ReturnType<typeof setTimeout> | null;
  }>({ lastUpdate: 0, pending: null, timer: null });

  const throttledSetThought = useMemo(() => {
    const THROTTLE_MS = 50; // 50ms throttle interval
    return (data: ThoughtData) => {
      const now = Date.now();
      const ref = thoughtThrottleRef.current;

      if (now - ref.lastUpdate >= THROTTLE_MS) {
        ref.lastUpdate = now;
        ref.pending = null;
        if (ref.timer) {
          clearTimeout(ref.timer);
          ref.timer = null;
        }
        setThought(data);
      } else {
        ref.pending = data;
        if (!ref.timer) {
          ref.timer = setTimeout(
            () => {
              ref.lastUpdate = Date.now();
              ref.timer = null;
              if (ref.pending) {
                setThought(ref.pending);
                ref.pending = null;
              }
            },
            THROTTLE_MS - (now - ref.lastUpdate)
          );
        }
      }
    };
  }, []);

  // Cleanup throttle timer
  useEffect(() => {
    return () => {
      if (thoughtThrottleRef.current.timer) {
        clearTimeout(thoughtThrottleRef.current.timer);
      }
    };
  }, []);

  // Combined running state: waiting for response OR stream is running OR tools are active
  const running = isTurnRunning(turnState);

  // Set current active message ID
  const setActiveMsgId = useCallback((msgId: MessageId | null) => {
    activeMsgIdRef.current = msgId;
    setActiveRequestMessageId(msgId);
  }, []);

  const dispatchTurnIfOpen = useCallback((event: NomiTurnEvent) => {
    if (turnClosedRef.current && !awaitingBackendTurnRef.current) return;
    dispatchTurn(event);
  }, []);

  const settleCompletedTurn = useCallback(() => {
    setPauseNotice(null);
    if (turnSettledRef.current && !rootTurnIdRef.current && !awaitingBackendTurnRef.current) {
      return;
    }
    turnLifecycleGenerationRef.current += 1;
    turnCompletionGenerationRef.current += 1;
    turnReconcileSequenceRef.current += 1;
    setRootTurnId(null);
    awaitingBackendTurnRef.current = false;
    turnClosedRef.current = true;
    rejectUnannouncedStartRef.current = false;
    verifyUnannouncedStartRuntimeRef.current = true;
    setActiveMsgId(null);
    turnSettledRef.current = true;
    dispatchTurn({ type: 'finish' });
    setThought({ subject: '', description: '' });
  }, [setActiveMsgId, setRootTurnId]);

  const adoptAuthoritativePause = useCallback((conversation: TChatConversation) => {
    const notice = getConversationPauseNotice(conversation);
    if (!notice || rejectUnannouncedStartRef.current || cancelledTurnIdsRef.current.has(notice.turnId)) return;
    turnLifecycleGenerationRef.current += 1;
    turnReconcileSequenceRef.current += 1;
    setRootTurnId(notice.turnId);
    awaitingBackendTurnRef.current = false;
    turnClosedRef.current = true;
    turnSettledRef.current = false;
    verifyUnannouncedStartRuntimeRef.current = true;
    setActiveMsgId(null);
    dispatchTurn({ type: 'reset' });
    setThought({ subject: '', description: '' });
    setPauseNotice(notice);
    setHasHydratedRunningState(true);
  }, [setActiveMsgId, setRootTurnId]);

  const adoptAuthoritativeProcessing = useCallback((conversation: TChatConversation) => {
    const activeTurnId = conversation.runtime?.active_turn_id;
    if (
      !activeTurnId ||
      rejectUnannouncedStartRef.current ||
      cancelledTurnIdsRef.current.has(activeTurnId)
    ) {
      return;
    }

    const changedTurn = rootTurnIdRef.current !== activeTurnId;
    const shouldRaiseRunning =
      changedTurn ||
      turnClosedRef.current ||
      awaitingBackendTurnRef.current ||
      !isTurnRunning(turnStateRef.current);
    if (changedTurn) {
      turnStartGenerationRef.current += 1;
      if (rootTurnIdRef.current !== null) setActiveMsgId(null);
    }
    setRootTurnId(activeTurnId);
    awaitingBackendTurnRef.current = false;
    turnClosedRef.current = false;
    rejectUnannouncedStartRef.current = false;
    verifyUnannouncedStartRuntimeRef.current = false;
    turnSettledRef.current = false;
    setStopNotice(null);
    setPauseNotice(null);
    if (shouldRaiseRunning) dispatchTurn({ type: 'hydrate', isRunning: true });
    setHasHydratedRunningState(true);
  }, [setActiveMsgId, setRootTurnId]);

  const startAuthoritativeRuntimeReconciliation = useCallback(
    ({ immediate = false }: { immediate?: boolean } = {}) => {
      const generation = turnLifecycleGenerationRef.current;
      const sequence = turnReconcileSequenceRef.current + 1;
      turnReconcileSequenceRef.current = sequence;
      void reconcileConversationAuthoritativeRuntime(conversation_id, {
        isCurrent: () =>
          mountedRef.current &&
          turnLifecycleGenerationRef.current === generation &&
          turnReconcileSequenceRef.current === sequence,
        onIdle: settleCompletedTurn,
        onProcessing: adoptAuthoritativeProcessing,
        onPaused: adoptAuthoritativePause,
        delaysMs: immediate
          ? AUTHORITATIVE_RUNTIME_RESYNC_DELAYS_MS
          : TERMINAL_RECONCILE_DELAYS_MS,
        retryForever: true,
        logLabel: 'Nomi runtime',
      });
    },
    [adoptAuthoritativePause, adoptAuthoritativeProcessing, conversation_id, settleCompletedTurn]
  );

  const reconcileAfterStreamTerminal = useCallback(() => {
    startAuthoritativeRuntimeReconciliation();
  }, [startAuthoritativeRuntimeReconciliation]);

  const markTurnAccepted = useCallback(
    () => {
      if (!awaitingBackendTurnRef.current || rejectUnannouncedStartRef.current) return;
      if (!verifyUnannouncedStartRuntimeRef.current) turnLifecycleGenerationRef.current += 1;
      setRootTurnId(null);
      awaitingBackendTurnRef.current = false;
      turnSettledRef.current = false;
      startAuthoritativeRuntimeReconciliation();
    },
    [setRootTurnId, startAuthoritativeRuntimeReconciliation]
  );

  const reconcilePublicDeliveryReplay = useCallback(
    (completed: boolean) => {
      if (completed) {
        settleCompletedTurn();
        return;
      }

      // Discard the optimistic local submit. Only a fresh runtime snapshot may
      // reopen this already-accepted delivery.
      turnLifecycleGenerationRef.current += 1;
      turnReconcileSequenceRef.current += 1;
      setRootTurnId(null);
      awaitingBackendTurnRef.current = false;
      turnClosedRef.current = true;
      turnSettledRef.current = true;
      rejectUnannouncedStartRef.current = false;
      verifyUnannouncedStartRuntimeRef.current = true;
      setActiveMsgId(null);
      dispatchTurn({ type: 'hydrate', isRunning: false, settleIdle: true });

      const generation = turnLifecycleGenerationRef.current;
      const sequence = turnReconcileSequenceRef.current;
      let observedProcessing = false;
      void reconcileConversationAuthoritativeRuntime(conversation_id, {
        isCurrent: () =>
          mountedRef.current &&
          turnLifecycleGenerationRef.current === generation &&
          turnReconcileSequenceRef.current === sequence,
        onProcessing: (conversation) => {
          if (observedProcessing) return;
          observedProcessing = true;
          adoptAuthoritativeProcessing(conversation);
        },
        onPaused: adoptAuthoritativePause,
        onIdle: settleCompletedTurn,
        logLabel: 'accepted delivery replay',
      });
    },
    [adoptAuthoritativePause, adoptAuthoritativeProcessing, conversation_id, setActiveMsgId, setRootTurnId, settleCompletedTurn]
  );

  useEffect(() => {
    return ipcBridge.conversation.userCreated.on((event) => {
      addOrUpdateMessage(transformUserCreatedEvent(event, conversation_id));
    });
  }, [conversation_id, addOrUpdateMessage]);

  useEffect(() => {
    let disposed = false;
    const off = ipcBridge.conversation.messageAnnotated.on((event) => {
      if (event.conversation_id !== conversation_id) return;
      void ipcBridge.database.getConversationMessage.invoke(event).then((message) => {
        if (disposed || message.conversation_id !== conversation_id
          || (message.message_id ?? message.msg_id) !== event.message_id) return;
        const canonical = normalizeDbMessage(message);
        if (canonical.type === 'text' && message.type === 'text') {
          // Existing camera annotations carry typed presentation facts that
          // the history normalizer does not reconstruct. Keep those facts.
          canonical.content = { ...canonical.content,
            ...(message.content.interaction ? { interaction: message.content.interaction } : {}),
            ...(message.content.observations ? { observations: message.content.observations } : {}),
          };
        }
        // Single-row annotations can arrive after newer live messages. Merge
        // with canonical ordering without replacing a longer active stream.
        updateMessageList(current => disposed ? current
          : mergeFetchedMessagesForConversation(current, [canonical], conversation_id));
      }).catch((error) => console.error('[Companion] Failed to refresh observation:', error));
    });
    return () => { disposed = true; off(); };
  }, [conversation_id, updateMessageList]);

  useEffect(() => {
    return ipcBridge.conversation.responseStream.on((message) => {
      if (conversation_id !== message.conversation_id) {
        return;
      }

      // A fresh idle hydration and an exact active turn_id form the authority
      // boundary for lifecycle state. Late output is still renderable history,
      // but it cannot reopen a completed turn or mutate a newer accepted turn.
      const appliesToTurn =
        shouldApplyNomiStreamEventToTurn({
          eventTurnId: message.turn_id,
          activeTurnId: rootTurnIdRef.current,
          turnClosed: turnClosedRef.current,
          awaitingBackendTurn: awaitingBackendTurnRef.current,
        });
      if (!appliesToTurn) {
        addOrUpdateMessage(transformMessage(message));
        return;
      }

      // Filter out events not belonging to current active request (prevents aborted events from interfering)
      // Note: only filter out thought and start messages, other messages must be rendered
      if (activeMsgIdRef.current && message.msg_id && message.msg_id !== activeMsgIdRef.current) {
        if (message.type === 'thought') {
          return;
        }
      }

      switch (message.type) {
        case 'thought':
          dispatchTurnIfOpen({ type: 'activity' });
          throttledSetThought(normalizeThoughtData(message.data));
          break;
        case 'start':
          dispatchTurnIfOpen({ type: 'activity' });
          // Don't reset waitingResponse here - let tool completion flow handle it
          break;
        case 'output_discarded':
          // The backend follows this control frame with authoritative
          // replace/hidden updates for the exact superseded segments.
          dispatchTurnIfOpen({ type: 'activity' });
          setThought({ subject: '', description: '' });
          break;
        case 'turn_metrics':
          {
            // Non-authoritative runtime telemetry. Lifecycle completion remains
            // owned exclusively by the conversation-scoped `turn.completed`
            // event; this frame only updates the metrics chip and rehydration
            // snapshot.
            const metrics = message.data as
              | {
                  input_tokens?: number;
                  output_tokens?: number;
                  reasoning_tokens?: number;
                  context_tokens?: number;
                  context_window?: number;
                }
              | undefined;
            if (metrics && typeof metrics === 'object') {
              const validTokenCount = (value: unknown): number | undefined =>
                typeof value === 'number' && Number.isFinite(value) && value >= 0 ? value : undefined;
              const inputTokens = validTokenCount(metrics.input_tokens);
              const outputTokens = validTokenCount(metrics.output_tokens);
              const reasoningTokens = validTokenCount(metrics.reasoning_tokens);
              const newTokenUsage: TokenUsageData = {
                total_tokens: (inputTokens ?? 0) + (outputTokens ?? 0),
                ...(inputTokens !== undefined ? { input_tokens: inputTokens } : {}),
                ...(outputTokens !== undefined ? { output_tokens: outputTokens } : {}),
                ...(reasoningTokens !== undefined ? { reasoning_tokens: reasoningTokens } : {}),
                context_tokens: validTokenCount(metrics.context_tokens),
                context_window: validTokenCount(metrics.context_window),
              };
              setTokenUsage(newTokenUsage);
            }
          }
          break;
        case 'finish':
          {
            // Stream completion can precede backend turn-handle release.
            setThought({ subject: '', description: '' });
            reconcileAfterStreamTerminal();
          }
          break;
        case 'task_plan_changed':
          // Progress is re-read by useConversationTaskPlan. It is neither a
          // transcript item nor lifecycle authority for the busy state.
          break;
        case 'system':
          // Non-transcript System events never grant Turn lifecycle authority.
          break;
        case 'tool_group':
          {
            // Check whether any tools are executing.
            const toolState = getNomiToolGroupRuntimeState(message.data);
            dispatchTurnIfOpen({ type: 'toolGroup', hasActive: toolState.hasActive, hasAny: toolState.hasAny });

            if (toolState.hasActive) {
              if (toolState.executingDescription) {
                setThought({
                  subject: 'Executing',
                  description: toolState.executingDescription,
                });
              }
            } else if (!turnStateRef.current.streamRunning) {
              // All tools completed and stream stopped, clear thought
              setThought({ subject: '', description: '' });
            }

            // Continue passing message to message list update
            addOrUpdateMessage(transformMessage(message));
          }
          break;
        default: {
          if (message.type === 'error') {
            setThought({ subject: '', description: '' });
            onError?.(message as IResponseMessage);
            reconcileAfterStreamTerminal();
          } else if (message.type === 'content') {
            // A terminal Agent Execution report is a self-contained projection,
            // not a new model stream. Render it without re-raising the send-box
            // busy state; ordinary stream content still marks the turn active.
            dispatchTurnIfOpen({
              type: 'content',
              streamComplete: isCompleteMessageProjection(message),
            });
          } else {
            // Any other non-error output: keep the turn marked running (handles
            // events that arrive after a premature finish).
            dispatchTurnIfOpen({ type: 'activity' });
          }
          // Backend handles persistence, Frontend only updates UI
          addOrUpdateMessage(transformMessage(message));
          break;
        }
      }
    });
    // Note: turn state is read via turnStateRef to avoid re-subscription
  }, [
    addOrUpdateMessage,
    conversation_id,
    dispatchTurnIfOpen,
    onError,
    reconcileAfterStreamTerminal,
  ]);

  useEffect(() => {
    let disposed = false;
    const unsubscribe = ipcBridge.conversation.turnStarted.on((event) => {
      if (event.conversation_id !== conversation_id) return;
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
        turnLifecycleGenerationRef.current += 1;
        if (rootTurnIdRef.current && rootTurnIdRef.current !== event.turn_id) setActiveMsgId(null);
        setRootTurnId(event.turn_id);
        awaitingBackendTurnRef.current = false;
        turnClosedRef.current = false;
        rejectUnannouncedStartRef.current = false;
        verifyUnannouncedStartRuntimeRef.current = false;
        turnSettledRef.current = false;
        setStopNotice(null);
        setPauseNotice(null);
        dispatchTurn({ type: 'activity' });
        setHasHydratedRunningState(true);
        // Accepting turn.started advances the lifecycle generation. Transfer
        // the authoritative poll to that generation so a later delivery gap
        // cannot lose the terminal runtime transition.
        startAuthoritativeRuntimeReconciliation();
      };

      if (startAction === 'accept') {
        acceptStart();
        return;
      }

      const generation = turnLifecycleGenerationRef.current;
      void getConversationOrNull(conversation_id)
        .then((conversation) => {
          if (
            disposed ||
            turnLifecycleGenerationRef.current !== generation ||
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
          console.warn('[useNomiMessage] Failed to verify unannounced turn start:', error);
        });
    });
    return () => {
      disposed = true;
      unsubscribe();
    };
  }, [conversation_id, setActiveMsgId, setRootTurnId, startAuthoritativeRuntimeReconciliation]);

  useEffect(() => {
    return ipcBridge.conversation.reconnected.on(() => {
      startAuthoritativeRuntimeReconciliation({ immediate: true });
    });
  }, [startAuthoritativeRuntimeReconciliation]);

  useEffect(() => ipcBridge.conversation.turnPaused.on((event) => {
    if (event.conversation_id !== conversation_id) return;
    if (rootTurnIdRef.current && rootTurnIdRef.current !== event.turn_id) return;
    startAuthoritativeRuntimeReconciliation({ immediate: true });
  }), [conversation_id, startAuthoritativeRuntimeReconciliation]);

  useEffect(() => {
    let disposed = false;

    const unsubscribe = ipcBridge.conversation.turnCompleted.on((event) => {
      if (
        event.conversation_id !== conversation_id ||
        !isAuthoritativeCompletionRuntimeIdle(event.runtime)
      ) {
        return;
      }

      const rootTurnId = rootTurnIdRef.current;
      const awaitingBackendTurn = awaitingBackendTurnRef.current;
      const action = classifyAuthoritativeTurnCompletion({
        rootTurnId,
        completedTurnId: event.turn_id,
        awaitingBackendTurn,
      });
      if (action === 'settle') {
        settleCompletedTurn();
        return;
      }
      if (action === 'ignore') return;

      const observedRootTurnId = rootTurnId;
      const observedAwaitingBackendTurn = awaitingBackendTurn;
      const generation = turnLifecycleGenerationRef.current;
      const sequence = turnReconcileSequenceRef.current + 1;
      turnReconcileSequenceRef.current = sequence;
      void reconcileConversationTurnAfterStreamTerminal(
        conversation_id,
        () =>
          !disposed &&
          mountedRef.current &&
          turnLifecycleGenerationRef.current === generation &&
          turnReconcileSequenceRef.current === sequence &&
          rootTurnIdRef.current === observedRootTurnId &&
          awaitingBackendTurnRef.current === observedAwaitingBackendTurn,
        settleCompletedTurn
      );
    });

    return () => {
      disposed = true;
      unsubscribe();
    };
  }, [conversation_id, settleCompletedTurn]);

  useEffect(() => {
    let cancelled = false;

    // Clear turn state on conversation switch so a previous conversation's
    // running state cannot bleed into this one. Lifecycle generations prevent
    // the initial snapshot from settling a local submit or accepted start that
    // races the async query.
    dispatchTurn({ type: 'reset' });
    turnLifecycleGenerationRef.current += 1;
    turnSettledRef.current = true;
    const hydrationGeneration = turnLifecycleGenerationRef.current;
    setThought({ subject: '', description: '' });
    setStopNotice(null);
    setPauseNotice(null);
    setTokenUsage(null);
    setHasHydratedRunningState(false);
    setRootTurnId(null);
    setActiveMsgId(null);
    awaitingBackendTurnRef.current = false;
    // Start behind the same idle fence before the async snapshot resolves.
    // Otherwise a delayed turn.started could advance the generation first and
    // cause the later authoritative idle response to be discarded as stale.
    const pendingHydrationFence = getNomiHydrationLifecycleFence(false);
    turnClosedRef.current = pendingHydrationFence.turnClosed;
    cancelledTurnIdsRef.current.clear();
    rejectUnannouncedStartRef.current = false;
    verifyUnannouncedStartRuntimeRef.current =
      pendingHydrationFence.verifyUnannouncedStartRuntime;

    // Check actual conversation status from backend before resetting all running states
    // to avoid flicker when switching to a running conversation
    const hydrationSequence = turnReconcileSequenceRef.current + 1;
    turnReconcileSequenceRef.current = hydrationSequence;

    const restoreTokenUsage = (res: TChatConversation | null) => {
      if (res?.type !== 'nomi' || !res.extra?.last_token_usage) return;
      const { last_token_usage } = res.extra;
      if (last_token_usage.total_tokens > 0) setTokenUsage(last_token_usage);
    };

    // A failed/unknown snapshot is not idle authority. Keep hydration closed
    // and retry with capped backoff; the shared helper catches transport errors
    // so this fire-and-forget effect cannot create an unhandled rejection.
    void reconcileConversationAuthoritativeRuntime(conversation_id, {
      isCurrent: () =>
        !cancelled &&
        mountedRef.current &&
        turnLifecycleGenerationRef.current === hydrationGeneration &&
        turnReconcileSequenceRef.current === hydrationSequence,
      onIdle: (res) => {
        const fence = getNomiHydrationLifecycleFence(false);
        setRootTurnId(null);
        awaitingBackendTurnRef.current = false;
        turnClosedRef.current = fence.turnClosed;
        verifyUnannouncedStartRuntimeRef.current = fence.verifyUnannouncedStartRuntime;
        dispatchTurn({ type: 'hydrate', isRunning: false, settleIdle: true });
        restoreTokenUsage(res);
        setHasHydratedRunningState(true);
      },
      onProcessing: (res) => {
        restoreTokenUsage(res);
        adoptAuthoritativeProcessing(res);
        // Hydration only needs the first complete authority snapshot. Move the
        // continuing poll to the ordinary lifecycle owner/sequence.
        startAuthoritativeRuntimeReconciliation();
      },
      onPaused: (res) => {
        restoreTokenUsage(res);
        adoptAuthoritativePause(res);
      },
      delaysMs: AUTHORITATIVE_RUNTIME_RESYNC_DELAYS_MS,
      retryForever: true,
      announceSettled: false,
      logLabel: 'Nomi hydration',
    });
    return () => {
      cancelled = true;
    };
  }, [
    adoptAuthoritativePause,
    adoptAuthoritativeProcessing,
    conversation_id,
    setActiveMsgId,
    setRootTurnId,
    startAuthoritativeRuntimeReconciliation,
  ]);

  const resetState = useCallback(() => {
    turnLifecycleGenerationRef.current += 1;
    turnSettledRef.current = true;
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
    turnClosedRef.current = true;
    rejectUnannouncedStartRef.current = true;
    verifyUnannouncedStartRuntimeRef.current = rootTurnId === null;
    setStopNotice({ stoppedAt: Date.now() });
    setPauseNotice(null);
    dispatchTurn({ type: 'reset' });
    setThought({ subject: '', description: '' });
    // Clear active message ID to prevent filtering events from new messages after stop
    setActiveMsgId(null);
  }, [setActiveMsgId]);

  // External setter used by the send box to raise the spinner on submit.
  const setWaitingResponse = useCallback((value: boolean) => {
    turnLifecycleGenerationRef.current += 1;
    if (value) {
      turnStartGenerationRef.current += 1;
      setRootTurnId(null);
      awaitingBackendTurnRef.current = true;
      turnClosedRef.current = false;
      rejectUnannouncedStartRef.current = false;
      verifyUnannouncedStartRuntimeRef.current = true;
      turnSettledRef.current = false;
      setStopNotice(null);
      setPauseNotice(null);
    } else {
      setRootTurnId(null);
      awaitingBackendTurnRef.current = false;
      turnClosedRef.current = true;
      rejectUnannouncedStartRef.current = false;
      verifyUnannouncedStartRuntimeRef.current = true;
      turnSettledRef.current = true;
    }
    dispatchTurn({ type: 'setWaiting', value });
  }, [setRootTurnId]);

  const restoreRunningAfterStopFailure = useCallback(() => {
    turnLifecycleGenerationRef.current += 1;
    setStopNotice(null);
    const rootTurnId = rootTurnIdRef.current;
    if (rootTurnId) cancelledTurnIdsRef.current.delete(rootTurnId);
    awaitingBackendTurnRef.current = false;
    turnClosedRef.current = false;
    rejectUnannouncedStartRef.current = false;
    verifyUnannouncedStartRuntimeRef.current = false;
    turnSettledRef.current = false;
    dispatchTurn({ type: 'hydrate', isRunning: true });
    startAuthoritativeRuntimeReconciliation();
  }, [startAuthoritativeRuntimeReconciliation]);

  const confirmStopped = useCallback(() => {
    setPauseNotice(null);
    turnLifecycleGenerationRef.current += 1;
    setRootTurnId(null);
    awaitingBackendTurnRef.current = false;
    turnClosedRef.current = true;
    rejectUnannouncedStartRef.current = false;
    turnSettledRef.current = true;
    dispatchTurn({ type: 'reset' });
  }, [setRootTurnId]);

  const getTurnStartGeneration = useCallback(() => turnStartGenerationRef.current, []);
  const getTurnCompletionGeneration = useCallback(() => turnCompletionGenerationRef.current, []);

  return {
    thought,
    setThought,
    running,
    activeTurnId: running ? activeTurnId ?? undefined : undefined,
    activeRequestMessageId: running ? activeRequestMessageId ?? undefined : undefined,
    hasHydratedRunningState,
    stopNotice,
    pauseNotice,
    tokenUsage,
    setActiveMsgId,
    markTurnAccepted,
    reconcilePublicDeliveryReplay,
    reconcileAfterStreamTerminal,
    setWaitingResponse,
    resetState,
    confirmStopped,
    restoreRunningAfterStopFailure,
    getTurnStartGeneration,
    getTurnCompletionGeneration,
  };
};

export type NomiMessageRuntime = ReturnType<typeof useNomiMessage>;
