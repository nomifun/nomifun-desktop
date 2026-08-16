/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { parseMessageId, type ConversationId, type MessageId } from '@/common/types/ids';
import { ipcBridge } from '@/common';
import { transformMessage, transformUserCreatedEvent } from '@/common/chat/chatLib';
import { isToolGroupStatusActive, normalizeToolGroupStatus } from '@/common/chat/toolGroupStatus';
import { extractResponseTextChunk, optionalDisplayText, toDisplayText } from '@/common/chat/displayText';
import type { IResponseMessage } from '@/common/adapter/ipcBridge';
import type { TChatConversation, TokenUsageData } from '@/common/config/storage';
import { uuid } from '@/common/utils';
import { useAddOrUpdateMessage } from '@/renderer/pages/conversation/Messages/hooks';
import { getConversationOrNull } from '@/renderer/pages/conversation/utils/conversationCache';
import {
  isCompleteMessageProjection,
  isConversationProcessing,
} from '@/renderer/pages/conversation/utils/conversationRuntime';
import { useCallback, useEffect, useMemo, useReducer, useRef, useState } from 'react';
import type { ThoughtData } from '../thoughtTypes';
import {
  AUTHORITATIVE_RUNTIME_RESYNC_DELAYS_MS,
  reconcileConversationAuthoritativeRuntime,
  reconcileConversationTurnAfterAcceptedReplay,
  reconcileConversationTurnAfterStreamTerminal,
  TERMINAL_RECONCILE_DELAYS_MS,
} from '../reconcileConversationTurnAfterStreamTerminal';
import {
  classifyAuthoritativeTurnCompletion,
  classifyAuthoritativeTurnStart,
  isAuthoritativeCompletionRuntimeIdle,
  resolveVerifiedAuthoritativeTurnStart,
} from '../authoritativeTurnLifecyclePolicy';
import { processLocalCronResponse } from './localCronCommands';
import {
  getNomiHydrationLifecycleFence,
  shouldApplyNomiStreamEventToTurn,
} from './nomiLifecycleFence';
import {
  NomiMessageBufferStore,
  isNomiTextReplacement,
  rememberBoundedNomiCronId,
  rememberBoundedNomiProcessedVersion,
} from './nomiMessageBuffer';
import {
  createNomiPostProcessState,
  discardNomiPostProcessTerminal,
  forgetNomiPostProcessPending,
  getNomiPostProcessInFlightWaiters,
  isNomiBackendFinalTextAuthoritative,
  isNomiInFlightPostProcessCurrent,
  isNomiPostProcessBufferAssociated,
  isNomiPostProcessRequestCurrent,
  markNomiPostProcessWaitingForInFlight,
  promoteNomiPostProcessObservationsForBuffer,
  rememberNomiPostProcessObservation,
  rememberNomiPostProcessPending,
  shouldHandleNomiTerminalPostProcess,
  tryRememberNomiInFlightPostProcess,
  type NomiInFlightPostProcess,
  type NomiPostProcessState,
  type NomiTerminalPostProcessRequest,
} from './nomiPostProcessState';
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
  confirmingDescription?: string;
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
  const confirmingTool = tools.find((tool) => tool.status === 'Confirming');
  const executingTool = tools.find((tool) => tool.status === 'Executing');

  return {
    tools,
    hasActive,
    hasAny: tools.length > 0,
    confirmingDescription: confirmingTool
      ? optionalDisplayText(confirmingTool.description) || optionalDisplayText(confirmingTool.name) || 'Tool execution'
      : undefined,
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
    onConfigChanged?: (capabilities: Record<string, unknown>) => void;
    readOnly?: boolean;
  }
) => {
  const onError = options?.onError;
  const onConfigChanged = options?.onConfigChanged;
  const readOnly = options?.readOnly === true;
  const onConfigChangedRef = useRef(onConfigChanged);
  const conversationIdRef = useRef(conversation_id);
  conversationIdRef.current = conversation_id;
  const addOrUpdateMessage = useAddOrUpdateMessage();
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
  // Current active message ID to filter out events from old requests (prevents aborted request events from interfering with new ones)
  const activeMsgIdRef = useRef<string | null>(null);
  const rootTurnIdRef = useRef<MessageId | null>(null);
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
  const postProcessGenerationRef = useRef(0);
  const mountedRef = useRef(true);
  const lastSettledTurnIdRef = useRef<MessageId | null>(null);
  const turnSettledRef = useRef(true);
  const messageBufferRef = useRef(new NomiMessageBufferStore());
  const postProcessStateRef = useRef<NomiPostProcessState>(createNomiPostProcessState());
  const backendTerminalIdsRef = useRef<Set<string>>(new Set());
  const backendTerminalTurnIdsRef = useRef<Set<string>>(new Set());

  const invalidatePostProcessing = useCallback((clearBuffer = false) => {
    postProcessGenerationRef.current += 1;
    postProcessStateRef.current = createNomiPostProcessState();
    backendTerminalIdsRef.current = new Set();
    backendTerminalTurnIdsRef.current = new Set();
    lastSettledTurnIdRef.current = null;
    if (clearBuffer) messageBufferRef.current.clearAll();
  }, []);

  useEffect(() => {
    mountedRef.current = true;
    return () => {
      mountedRef.current = false;
      turnLifecycleGenerationRef.current += 1;
      turnReconcileSequenceRef.current += 1;
      invalidatePostProcessing(true);
    };
  }, [invalidatePostProcessing]);

  // Mirror the reducer state into a ref so the (non-resubscribing) stream
  // closure can read the current turn state without being a dependency.
  const turnStateRef = useRef(turnState);
  useEffect(() => {
    turnStateRef.current = turnState;
  }, [turnState]);

  useEffect(() => {
    onConfigChangedRef.current = onConfigChanged;
  }, [onConfigChanged]);

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
  const setActiveMsgId = useCallback((msgId: string | null) => {
    activeMsgIdRef.current = msgId;
  }, []);

  const dispatchTurnIfOpen = useCallback((event: NomiTurnEvent) => {
    if (turnClosedRef.current && !awaitingBackendTurnRef.current) return;
    dispatchTurn(event);
  }, []);

  const settleCompletedTurn = useCallback(() => {
    if (turnSettledRef.current && !rootTurnIdRef.current && !awaitingBackendTurnRef.current) {
      return;
    }
    turnLifecycleGenerationRef.current += 1;
    turnCompletionGenerationRef.current += 1;
    turnReconcileSequenceRef.current += 1;
    if (rootTurnIdRef.current) {
      lastSettledTurnIdRef.current = rootTurnIdRef.current;
    }
    rootTurnIdRef.current = null;
    awaitingBackendTurnRef.current = false;
    turnClosedRef.current = true;
    rejectUnannouncedStartRef.current = false;
    verifyUnannouncedStartRuntimeRef.current = true;
    activeMsgIdRef.current = null;
    turnSettledRef.current = true;
    dispatchTurn({ type: 'finish' });
    setThought({ subject: '', description: '' });
    // A compatibility finish can race its final text fragment. Do not clear the
    // bounded buffer or pending fallback at the ordinary idle boundary; stop,
    // switch, unmount, or the next turn explicitly invalidates them.
  }, []);

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
      invalidatePostProcessing(true);
      turnStartGenerationRef.current += 1;
    }
    rootTurnIdRef.current = activeTurnId;
    awaitingBackendTurnRef.current = false;
    turnClosedRef.current = false;
    rejectUnannouncedStartRef.current = false;
    verifyUnannouncedStartRuntimeRef.current = false;
    turnSettledRef.current = false;
    setStopNotice(null);
    if (shouldRaiseRunning) dispatchTurn({ type: 'hydrate', isRunning: true });
    setHasHydratedRunningState(true);
  }, [invalidatePostProcessing]);

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
        delaysMs: immediate
          ? AUTHORITATIVE_RUNTIME_RESYNC_DELAYS_MS
          : TERMINAL_RECONCILE_DELAYS_MS,
        retryForever: true,
        logLabel: 'Nomi runtime',
      });
    },
    [adoptAuthoritativeProcessing, conversation_id, settleCompletedTurn]
  );

  const reconcileAfterStreamTerminal = useCallback(() => {
    startAuthoritativeRuntimeReconciliation();
  }, [startAuthoritativeRuntimeReconciliation]);

  const markTurnAccepted = useCallback(
    () => {
      if (!awaitingBackendTurnRef.current || rejectUnannouncedStartRef.current) return;
      if (!verifyUnannouncedStartRuntimeRef.current) turnLifecycleGenerationRef.current += 1;
      rootTurnIdRef.current = null;
      awaitingBackendTurnRef.current = false;
      turnSettledRef.current = false;
      startAuthoritativeRuntimeReconciliation();
    },
    [startAuthoritativeRuntimeReconciliation]
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
      invalidatePostProcessing(true);
      rootTurnIdRef.current = null;
      awaitingBackendTurnRef.current = false;
      turnClosedRef.current = true;
      turnSettledRef.current = true;
      rejectUnannouncedStartRef.current = false;
      verifyUnannouncedStartRuntimeRef.current = true;
      activeMsgIdRef.current = null;
      dispatchTurn({ type: 'hydrate', isRunning: false, settleIdle: true });

      const generation = turnLifecycleGenerationRef.current;
      const sequence = turnReconcileSequenceRef.current;
      let observedProcessing = false;
      void reconcileConversationTurnAfterAcceptedReplay(
        conversation_id,
        () =>
          mountedRef.current &&
          turnLifecycleGenerationRef.current === generation &&
          turnReconcileSequenceRef.current === sequence,
        (conversation) => {
          if (observedProcessing) return;
          observedProcessing = true;
          adoptAuthoritativeProcessing(conversation);
        },
        settleCompletedTurn
      );
    },
    [adoptAuthoritativeProcessing, conversation_id, invalidatePostProcessing, settleCompletedTurn]
  );

  const isCurrentPostProcessScope = useCallback((request: NomiTerminalPostProcessRequest): boolean => {
    return isNomiPostProcessRequestCurrent(request, {
      mounted: mountedRef.current,
      conversationId: conversationIdRef.current,
      generation: postProcessGenerationRef.current,
      turnStartGeneration: turnStartGenerationRef.current,
      rootTurnId: rootTurnIdRef.current,
      lastSettledTurnId: lastSettledTurnIdRef.current,
      cancelledTurnIds: cancelledTurnIdsRef.current,
      backendTerminalIds: backendTerminalIdsRef.current,
      backendTerminalTurnIds: backendTerminalTurnIdsRef.current,
    });
  }, []);

  const isCurrentPostProcess = useCallback(
    (request: NomiInFlightPostProcess): boolean => {
      if (!isCurrentPostProcessScope(request)) return false;
      const current = messageBufferRef.current.get(request.targetMessageId);
      return isNomiInFlightPostProcessCurrent(
        postProcessStateRef.current,
        request,
        {
          mounted: mountedRef.current,
          conversationId: conversationIdRef.current,
          generation: postProcessGenerationRef.current,
          turnStartGeneration: turnStartGenerationRef.current,
          rootTurnId: rootTurnIdRef.current,
          lastSettledTurnId: lastSettledTurnIdRef.current,
          cancelledTurnIds: cancelledTurnIdsRef.current,
          backendTerminalIds: backendTerminalIdsRef.current,
          backendTerminalTurnIds: backendTerminalTurnIdsRef.current,
        },
        {
          version: current?.version,
          turnId: current?.turnId,
        }
      );
    },
    [isCurrentPostProcessScope]
  );

  const isPostProcessBufferAssociated = useCallback(
    (messageId: string | undefined, turnId?: string): boolean => {
      return isNomiPostProcessBufferAssociated(
        postProcessStateRef.current,
        conversationIdRef.current,
        messageId,
        turnId
      );
    },
    []
  );

  const isTerminalPostProcessEligible = useCallback(
    (
      message: Pick<
        IResponseMessage,
        | 'msg_id'
        | 'turn_id'
        | 'final_text_msg_id'
        | 'final_text_authoritative'
        | 'type'
        | 'data'
      >
    ): boolean => {
      return shouldHandleNomiTerminalPostProcess(
        {
          type: message.type,
          data: message.data,
          msgId: message.msg_id,
          turnId: message.turn_id,
          finalTextMsgId: message.final_text_msg_id,
          finalTextAuthoritative: message.final_text_authoritative,
        },
        {
          rootTurnId: rootTurnIdRef.current,
          lastSettledTurnId: lastSettledTurnIdRef.current,
          hasBuffer: (messageId) => messageBufferRef.current.has(messageId),
          isAssociated: isPostProcessBufferAssociated,
        }
      );
    },
    [isPostProcessBufferAssociated]
  );

  const resolveLegacyPostProcessBuffer = useCallback(
    (
      request: NomiTerminalPostProcessRequest
    ):
      | {
          messageId: MessageId;
          content: string;
          version: number;
          truncated: boolean;
        }
      | undefined => {
      const matchesTurn = (turnId: string | undefined) =>
        !request.turnId || !turnId || turnId === request.turnId;

      if (request.targetMessageId) {
        const buffered = messageBufferRef.current.get(request.targetMessageId);
        if (buffered && matchesTurn(buffered.turnId)) {
          return {
            messageId: request.targetMessageId,
            content: buffered.content,
            version: buffered.version,
            truncated: buffered.truncated,
          };
        }
        return undefined;
      }

      const terminalBuffer = messageBufferRef.current.get(request.terminalId);
      if (terminalBuffer && matchesTurn(terminalBuffer.turnId)) {
        return {
          messageId: request.terminalId,
          content: terminalBuffer.content,
          version: terminalBuffer.version,
          truncated: terminalBuffer.truncated,
        };
      }

      if (request.turnId) {
        const latest = messageBufferRef.current.findLatestForTurn(request.turnId);
        if (latest) {
          return {
            messageId: parseMessageId(latest.messageId),
            content: latest.content,
            version: latest.version,
            truncated: latest.truncated,
          };
        }
      }
      return undefined;
    },
    []
  );

  const startLegacyPostProcess = useCallback(
    (request: NomiTerminalPostProcessRequest): void => {
      if (
        readOnly ||
        !request.terminalId ||
        request.conversationId !== conversationIdRef.current ||
        request.generation !== postProcessGenerationRef.current ||
        request.turnStartGeneration !== turnStartGenerationRef.current ||
        backendTerminalIdsRef.current.has(request.terminalId) ||
        (request.turnId && backendTerminalTurnIdsRef.current.has(request.turnId))
      ) {
        return;
      }

      const postProcessState = postProcessStateRef.current;
      if (postProcessState.inFlight.has(request.terminalId)) return;

      const observed = postProcessState.observed.get(request.terminalId);
      if (observed) {
        const observedBuffer = messageBufferRef.current.get(observed.targetMessageId);
        if (observedBuffer?.version === observed.bufferVersion) {
          return;
        }
        postProcessState.observed.delete(request.terminalId);
        postProcessState.processed.delete(observed.targetMessageId);
      }
      rememberNomiPostProcessPending(postProcessState, request);
      postProcessState.waitingForInFlight.delete(request.terminalId);
      const resolved = resolveLegacyPostProcessBuffer(request);
      if (!resolved) {
        return;
      }
      if (resolved.truncated || !resolved.content.trim()) {
        // An explicit empty replacement is a real, versioned projection, while
        // a truncated buffer is deliberately unusable: rewriting from it could
        // replace a complete rendered message with only the retained prefix.
        // Keep an exact observation so a later complete replacement can advance
        // the version and wake a fresh attempt, but do not spin on reconnect.
        forgetNomiPostProcessPending(postProcessState, request.terminalId);
        rememberBoundedNomiProcessedVersion(
          postProcessState.processed,
          resolved.messageId,
          resolved.version
        );
        rememberNomiPostProcessObservation(postProcessState, {
          ...request,
          targetMessageId: resolved.messageId,
          allowTurnFallback: false,
          bufferVersion: resolved.version,
        });
        return;
      }

      const duplicateTarget = [...postProcessState.inFlight.values()].some(
        (inFlight) =>
          inFlight.targetMessageId === resolved.messageId &&
          inFlight.turnId === request.turnId
      );
      if (duplicateTarget) {
        markNomiPostProcessWaitingForInFlight(postProcessState, request.terminalId);
        return;
      }

      const processedVersion = postProcessState.processed.get(resolved.messageId);
      if (processedVersion === resolved.version) {
        forgetNomiPostProcessPending(postProcessState, request.terminalId);
        return;
      }
      if (processedVersion !== undefined) {
        postProcessState.processed.delete(resolved.messageId);
      }

      const inFlight: NomiInFlightPostProcess = {
        ...request,
        targetMessageId: resolved.messageId,
        allowTurnFallback: false,
        bufferVersion: resolved.version,
      };
      if (!tryRememberNomiInFlightPostProcess(postProcessState, inFlight)) {
        return;
      }
      forgetNomiPostProcessPending(postProcessState, request.terminalId);
      void (async () => {
        let resultApplied = false;
        let processingFailed = false;
        try {
          const result = await processLocalCronResponse(request.conversationId, resolved.content);
          // Replacement and system responses share one guard. A stop, switch,
          // new turn, or authoritative backend terminal invalidates both.
          if (!isCurrentPostProcess(inFlight)) return;

          if (
            result.displayContent !== undefined &&
            result.displayContent !== resolved.content
          ) {
            addOrUpdateMessage({
              id: uuid(),
              msg_id: resolved.messageId,
              type: 'text',
              position: 'left',
              conversation_id: request.conversationId,
              created_at: Date.now(),
              content: {
                content: result.displayContent,
                replace: true,
              },
            });
          }

          for (const response of result.systemResponses) {
            addOrUpdateMessage(
              {
                id: uuid(),
                type: 'tips',
                position: 'center',
                conversation_id: request.conversationId,
                created_at: Date.now(),
                content: {
                  content: response,
                  type: response.startsWith('❌') ? 'error' : 'success',
                },
              },
              true
            );
          }
          resultApplied = true;
        } catch {
          // Keep the buffer available for a later terminal/reconnect retry.
          processingFailed = true;
        } finally {
          // Invalidation swaps the whole state object. A completion from an old
          // generation must not delete or mark entries owned by the replacement
          // turn even when terminal/message ids happen to match.
          if (
            postProcessStateRef.current !== postProcessState ||
            postProcessState.inFlight.get(request.terminalId) !== inFlight
          ) {
            return;
          }
          postProcessState.inFlight.delete(request.terminalId);
          const pendingBeforeCompletion =
            getNomiPostProcessInFlightWaiters(postProcessState);

          const current = messageBufferRef.current.get(resolved.messageId);
          const hasLateFragment =
            current !== undefined && current.version !== resolved.version;
          const scopeStillCurrent = isCurrentPostProcessScope(inFlight);
          if (!resultApplied && !processingFailed && !hasLateFragment) {
            // The async result became stale because the scope was invalidated
            // (for example, an authoritative backend terminal won the race).
            // Do not resurrect a pending fallback after that invalidation.
            postProcessState.processed.delete(resolved.messageId);
            return;
          }
          if (resultApplied && !hasLateFragment) {
            rememberBoundedNomiProcessedVersion(
              postProcessState.processed,
              resolved.messageId,
              resolved.version
            );
            rememberNomiPostProcessObservation(postProcessState, inFlight);
          } else {
            postProcessState.processed.delete(resolved.messageId);
            rememberNomiPostProcessPending(postProcessState, {
              ...request,
              targetMessageId: resolved.messageId,
              allowTurnFallback: false,
            });
          }

          // Releasing any running slot wakes requests that were held in the
          // bounded pending map because the in-flight cap or a duplicate target
          // was active. A failed current request is deliberately absent from
          // this pre-completion snapshot so it does not enter a hot retry loop.
          // A late fragment is the exception: its advanced version should retry
          // immediately, preserving the existing deterministic replacement
          // path.
          const retryRequests = pendingBeforeCompletion.filter(
            (pending) => postProcessState.pending.get(pending.terminalId) === pending
          );
          if (resultApplied || hasLateFragment) {
            const currentPending = postProcessState.pending.get(request.terminalId);
            if (currentPending && !retryRequests.includes(currentPending)) {
              retryRequests.push(currentPending);
            }
          }
          if (scopeStillCurrent && retryRequests.length > 0) {
            queueMicrotask(() => {
              if (postProcessStateRef.current !== postProcessState) return;
              for (const pending of retryRequests) {
                if (postProcessState.pending.get(pending.terminalId) !== pending) continue;
                startLegacyPostProcess(pending);
              }
            });
          }
        }
      })();
    },
    [
      addOrUpdateMessage,
      isCurrentPostProcess,
      isCurrentPostProcessScope,
      readOnly,
      resolveLegacyPostProcessBuffer,
    ]
  );

  const retryPendingPostProcesses = useCallback(() => {
    for (const request of [...postProcessStateRef.current.pending.values()]) {
      startLegacyPostProcess(request);
    }
  }, [startLegacyPostProcess]);

  const processCompletedAssistantMessage = useCallback(
    (message: IResponseMessage): void => {
      if (message.turn_id) {
        // Preserve the exact owner across the terminal -> authoritative-idle
        // gap. The fallback may still be waiting for a final text fragment
        // after settle clears rootTurnIdRef.
        lastSettledTurnIdRef.current = message.turn_id;
      }
      if (isNomiBackendFinalTextAuthoritative(message.final_text_authoritative)) {
        rememberBoundedNomiCronId(backendTerminalIdsRef.current, message.msg_id);
        if (message.turn_id) {
          rememberBoundedNomiCronId(backendTerminalTurnIdsRef.current, message.turn_id);
        }
        const state = postProcessStateRef.current;
        discardNomiPostProcessTerminal(state, (request) =>
          request.terminalId === message.msg_id ||
          (message.turn_id !== undefined && request.turnId === message.turn_id) ||
          (message.final_text_msg_id !== undefined &&
            request.targetMessageId === message.final_text_msg_id)
        );
        return;
      }

      startLegacyPostProcess({
        conversationId: message.conversation_id,
        terminalId: message.msg_id,
        targetMessageId: message.final_text_msg_id,
        turnId: message.turn_id,
        allowTurnFallback: message.final_text_msg_id === undefined,
        generation: postProcessGenerationRef.current,
        turnStartGeneration: turnStartGenerationRef.current,
      });
    },
    [startLegacyPostProcess]
  );

  useEffect(() => {
    return ipcBridge.conversation.userCreated.on((event) => {
      addOrUpdateMessage(transformUserCreatedEvent(event, conversation_id));
    });
  }, [conversation_id, addOrUpdateMessage]);

  useEffect(() => {
    return ipcBridge.conversation.responseStream.on((message) => {
      if (conversation_id !== message.conversation_id) {
        return;
      }

      // A fresh idle hydration and an exact active turn_id form the authority
      // boundary for lifecycle state. Late output is still renderable history,
      // but it cannot reopen a completed turn or mutate a newer accepted turn.
      // Config changes are session-scoped rather than turn-scoped and therefore
      // remain applicable while the conversation is idle.
      const appliesToTurn =
        message.type === 'config_changed' ||
        shouldApplyNomiStreamEventToTurn({
          eventTurnId: message.turn_id,
          activeTurnId: rootTurnIdRef.current,
          turnClosed: turnClosedRef.current,
          awaitingBackendTurn: awaitingBackendTurnRef.current,
        });
      const isTextStreamMessage =
        !readOnly &&
        (message.type === 'content' || message.type === 'text') &&
        Boolean(message.msg_id);
      const textChunk = isTextStreamMessage ? extractResponseTextChunk(message.data) : '';
      const replacement = isTextStreamMessage && isNomiTextReplacement(message);
      const associatedWithPostProcess =
        isTextStreamMessage &&
        isPostProcessBufferAssociated(message.msg_id, message.turn_id);

      // A terminal frame may win the WebSocket race before the final text
      // fragment. Keep only text that is correlated with a pending terminal;
      // unrelated late output remains projection-only and cannot consume the
      // bounded fallback buffer.
      if (
        isTextStreamMessage &&
        (appliesToTurn || associatedWithPostProcess) &&
        (textChunk || replacement)
      ) {
        messageBufferRef.current[replacement ? 'replace' : 'append'](
          message.msg_id,
          textChunk,
          message.turn_id,
          (bufferedMessageId, bufferedTurnId) =>
            isPostProcessBufferAssociated(bufferedMessageId, bufferedTurnId) ||
            (!turnClosedRef.current &&
              (rootTurnIdRef.current
                ? bufferedTurnId === undefined || rootTurnIdRef.current === bufferedTurnId
                : awaitingBackendTurnRef.current &&
                (!activeMsgIdRef.current || activeMsgIdRef.current === bufferedMessageId)))
        );
        const buffered = messageBufferRef.current.get(message.msg_id);
        if (buffered) {
          promoteNomiPostProcessObservationsForBuffer(
            postProcessStateRef.current,
            conversationIdRef.current,
            message.msg_id,
            buffered.version,
            message.turn_id
          );
        }
        if (associatedWithPostProcess) retryPendingPostProcesses();
      }

      if (!appliesToTurn) {
        if (
          (message.type === 'finish' || message.type === 'error') &&
          isTerminalPostProcessEligible(message)
        ) {
          processCompletedAssistantMessage(message);
        }
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
        case 'turn_completed':
          {
            // Phase 3 observability: the engine emits one turn_completed per turn
            // carrying real aggregate metrics. This is the genuine source of token
            // usage for nomi turns (the finish event has never carried usage) —
            // it updates the send-box metrics chip and persists for rehydration.
            const metrics = message.data as
              | {
                  input_tokens?: number;
                  output_tokens?: number;
                  context_tokens?: number;
                  context_window?: number;
                }
              | undefined;
            if (metrics && typeof metrics === 'object') {
              const inputTokens = metrics.input_tokens || 0;
              const outputTokens = metrics.output_tokens || 0;
              const newTokenUsage: TokenUsageData = {
                total_tokens: inputTokens + outputTokens,
                context_tokens: metrics.context_tokens,
                context_window: metrics.context_window,
              };
              setTokenUsage(newTokenUsage);
              if (!readOnly) {
                void ipcBridge.conversation.update.invoke({
                  conversation_id: conversation_id,
                  updates: {
                    extra: { last_token_usage: newTokenUsage } as TChatConversation['extra'],
                  },
                });
              }
            }
          }
          break;
        case 'finish':
          {
            // Stream completion can precede backend turn-handle release.
            setThought({ subject: '', description: '' });
            if (message.msg_id && isTerminalPostProcessEligible(message)) {
              processCompletedAssistantMessage(message);
            }
            reconcileAfterStreamTerminal();
          }
          break;
        case 'tool_group':
          {
            // Check if any tools are executing or awaiting confirmation
            const toolState = getNomiToolGroupRuntimeState(message.data);
            dispatchTurnIfOpen({ type: 'toolGroup', hasActive: toolState.hasActive, hasAny: toolState.hasAny });

            // If tools are awaiting confirmation, update thought hint
            if (toolState.confirmingDescription) {
              setThought({
                subject: 'Awaiting Confirmation',
                // Prefer the contextual description (file/command/pattern) over the
                // bare tool name so the status reads e.g. "edit src/auth.ts".
                description: toolState.confirmingDescription,
              });
            } else if (toolState.hasActive) {
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
        case 'permission':
          dispatchTurnIfOpen({ type: 'activity' });
          addOrUpdateMessage(transformMessage(message));
          break;
        case 'config_changed':
          onConfigChangedRef.current?.(message.data as Record<string, unknown>);
          break;
        default: {
          if (message.type === 'error') {
            if (isTerminalPostProcessEligible(message)) {
              processCompletedAssistantMessage(message);
            }
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
    isNomiTextReplacement,
    isPostProcessBufferAssociated,
    isTerminalPostProcessEligible,
    onError,
    processCompletedAssistantMessage,
    readOnly,
    reconcileAfterStreamTerminal,
    retryPendingPostProcesses,
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
        if (rootTurnIdRef.current !== event.turn_id || turnSettledRef.current) {
          invalidatePostProcessing(true);
        }
        turnStartGenerationRef.current += 1;
        turnLifecycleGenerationRef.current += 1;
        rootTurnIdRef.current = event.turn_id;
        awaitingBackendTurnRef.current = false;
        turnClosedRef.current = false;
        rejectUnannouncedStartRef.current = false;
        verifyUnannouncedStartRuntimeRef.current = false;
        turnSettledRef.current = false;
        setStopNotice(null);
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
  }, [conversation_id, invalidatePostProcessing, startAuthoritativeRuntimeReconciliation]);

  useEffect(() => {
    return ipcBridge.conversation.reconnected.on(() => {
      startAuthoritativeRuntimeReconciliation({ immediate: true });
      retryPendingPostProcesses();
    });
  }, [retryPendingPostProcesses, startAuthoritativeRuntimeReconciliation]);

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
    invalidatePostProcessing(true);
    turnSettledRef.current = true;
    const hydrationGeneration = turnLifecycleGenerationRef.current;
    setThought({ subject: '', description: '' });
    setStopNotice(null);
    setTokenUsage(null);
    setHasHydratedRunningState(false);
    rootTurnIdRef.current = null;
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
        rootTurnIdRef.current = null;
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
      delaysMs: AUTHORITATIVE_RUNTIME_RESYNC_DELAYS_MS,
      retryForever: true,
      announceSettled: false,
      logLabel: 'Nomi hydration',
    });
    return () => {
      cancelled = true;
    };
  }, [
    adoptAuthoritativeProcessing,
    conversation_id,
    invalidatePostProcessing,
    startAuthoritativeRuntimeReconciliation,
  ]);

  const resetState = useCallback(() => {
    turnLifecycleGenerationRef.current += 1;
    invalidatePostProcessing(true);
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
    dispatchTurn({ type: 'reset' });
    setThought({ subject: '', description: '' });
    // Clear active message ID to prevent filtering events from new messages after stop
    activeMsgIdRef.current = null;
  }, [invalidatePostProcessing]);

  // External setter used by the send box to raise the spinner on submit.
  const setWaitingResponse = useCallback((value: boolean) => {
    turnLifecycleGenerationRef.current += 1;
    invalidatePostProcessing(true);
    if (value) {
      turnStartGenerationRef.current += 1;
      rootTurnIdRef.current = null;
      awaitingBackendTurnRef.current = true;
      turnClosedRef.current = false;
      rejectUnannouncedStartRef.current = false;
      verifyUnannouncedStartRuntimeRef.current = true;
      turnSettledRef.current = false;
      setStopNotice(null);
    } else {
      rootTurnIdRef.current = null;
      awaitingBackendTurnRef.current = false;
      turnClosedRef.current = true;
      rejectUnannouncedStartRef.current = false;
      verifyUnannouncedStartRuntimeRef.current = true;
      turnSettledRef.current = true;
    }
    dispatchTurn({ type: 'setWaiting', value });
  }, [invalidatePostProcessing]);

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
    turnLifecycleGenerationRef.current += 1;
    invalidatePostProcessing(true);
    rootTurnIdRef.current = null;
    awaitingBackendTurnRef.current = false;
    turnClosedRef.current = true;
    rejectUnannouncedStartRef.current = false;
    turnSettledRef.current = true;
    dispatchTurn({ type: 'reset' });
  }, [invalidatePostProcessing]);

  const getTurnStartGeneration = useCallback(() => turnStartGenerationRef.current, []);
  const getTurnCompletionGeneration = useCallback(() => turnCompletionGenerationRef.current, []);

  return {
    thought,
    setThought,
    running,
    hasHydratedRunningState,
    stopNotice,
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
