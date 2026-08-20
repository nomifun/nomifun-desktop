/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { extractResponseTextChunk, toDisplayText } from '@/common/chat/displayText';
import type {
  IConversationTurnCompletedEvent,
  IConversationTurnStartedEvent,
  IResponseMessage,
} from '@/common/adapter/ipcBridge';

import type {
  CreativeStudioAgentChatPort,
  CreativeStudioAgentTurnEvent,
  CreativeStudioAgentTurnRequest,
} from '../chatPort';
import { serializeCreativeStudioAgentHistory } from './history';
import { createNomiCreativeStudioAgentTransport } from './nomiTransport';
import type {
  NomiCreativeStudioAgentPortOptions,
  NomiCreativeStudioAgentSessionBinding,
  NomiCreativeStudioAgentSessionResolution,
  NomiCreativeStudioAgentTransport,
} from './types';

const DEFAULT_TURN_START_TIMEOUT_MS = 30_000;
const DEFAULT_RECOVERY_POLL_MS = 250;

type RuntimeEvent =
  | { kind: 'response'; value: IResponseMessage }
  | { kind: 'turn-started'; value: IConversationTurnStartedEvent }
  | { kind: 'turn-completed'; value: IConversationTurnCompletedEvent }
  | { kind: 'reconnected' };

type QueueResult =
  | { kind: 'event'; value: RuntimeEvent }
  | { kind: 'aborted' }
  | { kind: 'timeout' };

class RuntimeEventQueue {
  private readonly buffered: RuntimeEvent[] = [];
  private waiter: ((event: RuntimeEvent) => void) | null = null;

  push(event: RuntimeEvent): void {
    const waiter = this.waiter;
    if (waiter) {
      this.waiter = null;
      waiter(event);
      return;
    }
    this.buffered.push(event);
  }

  next(signal: AbortSignal, timeoutMs?: number): Promise<QueueResult> {
    const buffered = this.buffered.shift();
    if (buffered) return Promise.resolve({ kind: 'event', value: buffered });
    if (signal.aborted) return Promise.resolve({ kind: 'aborted' });

    return new Promise((resolve) => {
      let timer: ReturnType<typeof setTimeout> | undefined;
      const settle = (result: QueueResult) => {
        if (timer) clearTimeout(timer);
        signal.removeEventListener('abort', onAbort);
        this.waiter = null;
        resolve(result);
      };
      const onAbort = () => settle({ kind: 'aborted' });
      this.waiter = (event) => settle({ kind: 'event', value: event });
      signal.addEventListener('abort', onAbort, { once: true });
      if (timeoutMs !== undefined) {
        timer = setTimeout(() => settle({ kind: 'timeout' }), timeoutMs);
      }
    });
  }
}

export class NomiCreativeStudioAgentBindingError extends Error {
  constructor(message: string) {
    super(message);
    this.name = 'NomiCreativeStudioAgentBindingError';
  }
}

export class NomiCreativeStudioAgentRuntimeError extends Error {
  readonly code: string;

  constructor(code: string, message: string) {
    super(message);
    this.name = 'NomiCreativeStudioAgentRuntimeError';
    this.code = code;
  }
}

const sameModel = (
  left: NomiCreativeStudioAgentSessionBinding['model'],
  right: NomiCreativeStudioAgentSessionBinding['model']
): boolean => left.providerId === right.providerId && left.model === right.model;

const validateBinding = (
  request: CreativeStudioAgentTurnRequest,
  binding: NomiCreativeStudioAgentSessionBinding,
  authoritativeHistoryKey: string
): void => {
  if (binding.ownership !== 'creative-studio-exclusive') {
    throw new NomiCreativeStudioAgentBindingError('Nomi conversation is not Creative Studio exclusive');
  }
  if (binding.projectId !== request.projectId || binding.sessionId !== request.sessionId) {
    throw new NomiCreativeStudioAgentBindingError('Nomi conversation project/session binding mismatch');
  }
  if (!sameModel(binding.model, request.model)) {
    throw new NomiCreativeStudioAgentBindingError('Nomi conversation model binding mismatch');
  }
  if (binding.historyKey !== authoritativeHistoryKey) {
    throw new NomiCreativeStudioAgentBindingError('Nomi conversation history projection mismatch');
  }
};

const completedEvents = (
  resolution: NomiCreativeStudioAgentSessionResolution
): readonly CreativeStudioAgentTurnEvent[] => {
  const last = resolution.history.at(-1);
  if (!last || last.role !== 'assistant' || last.status !== 'complete') {
    throw new NomiCreativeStudioAgentRuntimeError(
      'AUTHORITATIVE_HISTORY_INCOMPLETE',
      'NomiFun did not return a completed assistant message for the recovered turn'
    );
  }
  return [
    { type: 'history-reconciled', history: resolution.history },
    { type: 'completed', assistantMessageId: last.id },
  ];
};

const activityFromResponse = (event: IResponseMessage): string | null => {
  if (event.type === 'thought') {
    if (event.data && typeof event.data === 'object' && !Array.isArray(event.data)) {
      const record = event.data as Record<string, unknown>;
      const subject = toDisplayText(record.subject).trim();
      const description = toDisplayText(record.description).trim();
      return [subject, description].filter(Boolean).join(' · ') || 'Agent 正在思考';
    }
    return toDisplayText(event.data).trim() || 'Agent 正在思考';
  }
  if (event.type === 'start') return 'Agent 已开始执行';
  if (event.type === 'permission') return 'Agent 正在等待确认';
  if (event.type === 'finish') return 'Agent 正在完成本轮任务';
  if (event.type === 'tool_group') {
    const tools = Array.isArray(event.data) ? event.data : [];
    const active = tools.find(
      (tool): tool is Record<string, unknown> =>
        Boolean(tool) &&
        typeof tool === 'object' &&
        !Array.isArray(tool) &&
        ['Executing', 'Confirming', 'Pending'].includes(String((tool as Record<string, unknown>).status))
    );
    if (active) {
      return (
        toDisplayText(active.description).trim() ||
        toDisplayText(active.name).trim() ||
        'Agent 正在执行工具'
      );
    }
    return 'Agent 正在处理工具结果';
  }
  return null;
};

const appendOnlyDelta = (
  previous: string,
  next: string,
  replace: boolean
): { text: string; delta: string } => {
  if (!replace) return { text: previous + next, delta: next };
  if (!next.startsWith(previous)) {
    throw new NomiCreativeStudioAgentRuntimeError(
      'NON_APPEND_REPLACEMENT',
      'Nomi response replaced prior text non-monotonically; the Creative Studio delta port cannot represent it safely'
    );
  }
  return { text: next, delta: next.slice(previous.length) };
};

const abortError = (): Error => {
  const error = new Error('Nomi Creative Studio Agent turn stopped');
  error.name = 'AbortError';
  return error;
};

const waitForRecoveryPoll = (signal: AbortSignal, delayMs: number): Promise<void> =>
  new Promise((resolve, reject) => {
    if (signal.aborted) {
      reject(abortError());
      return;
    }
    const timer = setTimeout(() => {
      signal.removeEventListener('abort', onAbort);
      resolve();
    }, delayMs);
    const onAbort = () => {
      clearTimeout(timer);
      signal.removeEventListener('abort', onAbort);
      reject(abortError());
    };
    signal.addEventListener('abort', onAbort, { once: true });
  });

const waitForReceiptOrAbort = <T>(
  operation: Promise<T>,
  signal: AbortSignal
): Promise<{ kind: 'receipt'; value: T } | { kind: 'aborted' }> =>
  new Promise((resolve, reject) => {
    let settled = false;
    const finish = (result: { kind: 'receipt'; value: T } | { kind: 'aborted' }) => {
      if (settled) return;
      settled = true;
      signal.removeEventListener('abort', onAbort);
      resolve(result);
    };
    const onAbort = () => finish({ kind: 'aborted' });
    if (signal.aborted) {
      finish({ kind: 'aborted' });
      void operation.catch(() => undefined);
      return;
    }
    signal.addEventListener('abort', onAbort, { once: true });
    void operation.then((value) => finish({ kind: 'receipt', value })).catch((error) => {
      if (settled) return;
      settled = true;
      signal.removeEventListener('abort', onAbort);
      reject(error);
    });
  });

const errorText = (event: IResponseMessage): string =>
  extractResponseTextChunk(event.data).trim() ||
  toDisplayText(event.data).trim() ||
  'NomiFun Agent returned an error event without details';

async function stopAfterAbort(
  transport: NomiCreativeStudioAgentTransport,
  conversationId: NomiCreativeStudioAgentSessionBinding['conversationId']
): Promise<never> {
  await transport.stopAndConfirm(conversationId);
  throw abortError();
}

async function verifyStartedTurn(
  transport: NomiCreativeStudioAgentTransport,
  binding: NomiCreativeStudioAgentSessionBinding,
  event: IConversationTurnStartedEvent
): Promise<boolean> {
  if (
    event.conversation_id !== binding.conversationId ||
    event.runtime.active_turn_id !== event.turn_id
  ) {
    return false;
  }
  const snapshot = await transport.inspect(binding.conversationId);
  return (
    snapshot.conversationId === binding.conversationId &&
    snapshot.authority === 'processing' &&
    snapshot.activeTurnId === event.turn_id
  );
}

/**
 * Adapt the real NomiFun conversation transport to the small Creative Studio
 * turn stream. Session binding is deliberately injected: today's conversation
 * API has no reusable project/session mapping endpoint and reconstructing one
 * from route state would be unsafe.
 */
export function createNomiCreativeStudioAgentChatPort(
  options: NomiCreativeStudioAgentPortOptions
): CreativeStudioAgentChatPort {
  const transport = options.transport ?? createNomiCreativeStudioAgentTransport();
  const turnStartTimeoutMs = options.turnStartTimeoutMs ?? DEFAULT_TURN_START_TIMEOUT_MS;
  const recoveryPollMs = Math.max(25, options.recoveryPollMs ?? DEFAULT_RECOVERY_POLL_MS);

  return {
    async *runTurn(request): AsyncIterable<CreativeStudioAgentTurnEvent> {
      const requestHistoryKey = serializeCreativeStudioAgentHistory(request.history);
      const resolveAuthoritative = async (): Promise<NomiCreativeStudioAgentSessionResolution> => {
        const resolution = await options.resolveSession({
          projectId: request.projectId,
          sessionId: request.sessionId,
          model: request.model,
          pendingTurnIdempotencyKey: request.idempotencyKey,
          signal: request.signal,
        });
        if (request.signal.aborted) throw abortError();
        validateBinding(
          request,
          resolution.binding,
          serializeCreativeStudioAgentHistory(resolution.history)
        );
        if (
          resolution.history.length < request.history.length ||
          serializeCreativeStudioAgentHistory(
            resolution.history.slice(0, request.history.length)
          ) !== requestHistoryKey
        ) {
          throw new NomiCreativeStudioAgentBindingError(
            'Nomi conversation no longer preserves the loaded project history prefix'
          );
        }
        const recoveredCount = resolution.history.length - request.history.length;
        if (recoveredCount !== 0 && recoveredCount !== 2) {
          throw new NomiCreativeStudioAgentBindingError(
            'Nomi conversation returned an invalid pending-turn history projection'
          );
        }
        return resolution;
      };
      const reconcileCompleted = async (): Promise<NomiCreativeStudioAgentSessionResolution> => {
        const deadline = Date.now() + turnStartTimeoutMs;
        for (;;) {
          const resolution = await resolveAuthoritative();
          if (resolution.history.length === request.history.length + 2) return resolution;
          if (resolution.history.length !== request.history.length) {
            throw new NomiCreativeStudioAgentRuntimeError(
              'AUTHORITATIVE_HISTORY_INVALID',
              'NomiFun returned an invalid completed-turn history projection'
            );
          }
          if (Date.now() >= deadline) {
            throw new NomiCreativeStudioAgentRuntimeError(
              'HISTORY_RECONCILIATION_TIMEOUT',
              'NomiFun reached a terminal state before its completed message pair became durable'
            );
          }
          await waitForRecoveryPoll(request.signal, recoveryPollMs);
        }
      };

      const initialResolution = await resolveAuthoritative();
      if (initialResolution.history.length === request.history.length + 2) {
        for (const event of completedEvents(initialResolution)) yield event;
        return;
      }
      const binding = initialResolution.binding;

      const beforeSend = await transport.inspect(binding.conversationId);
      if (beforeSend.conversationId !== binding.conversationId) {
        throw new NomiCreativeStudioAgentBindingError(
          'Nomi conversation identity changed during resolution'
        );
      }
      if (!sameModel(beforeSend.model, request.model)) {
        throw new NomiCreativeStudioAgentBindingError(
          'Nomi conversation selected model changed before send'
        );
      }
      if (beforeSend.authority === 'unknown') {
        throw new NomiCreativeStudioAgentRuntimeError(
          'CONVERSATION_AUTHORITY_UNKNOWN',
          'Bound Nomi conversation has no authoritative runtime state'
        );
      }
      if (request.signal.aborted) throw abortError();

      const queue = new RuntimeEventQueue();
      const unsubscribe = [
        transport.onResponse((value) => queue.push({ kind: 'response', value })),
        transport.onTurnStarted((value) => queue.push({ kind: 'turn-started', value })),
        transport.onTurnCompleted((value) => queue.push({ kind: 'turn-completed', value })),
        transport.onReconnected(() => queue.push({ kind: 'reconnected' })),
      ];

      let activeTurnId: IConversationTurnStartedEvent['turn_id'] | undefined;
      const streamedByMessageId = new Map<string, string>();
      let emittedAssistantText = '';
      let admittedNonTerminalTurn = false;

      try {
        const receiptPromise = transport.sendMessage({
          conversationId: binding.conversationId,
          prompt: request.prompt,
          idempotencyKey: request.idempotencyKey,
        });
        const receipt = await waitForReceiptOrAbort(receiptPromise, request.signal);

        if (receipt.kind === 'aborted') {
          void receiptPromise.catch(() => undefined);
          return await stopAfterAbort(transport, binding.conversationId);
        }

        if (receipt.value.completed) {
          if (receipt.value.result_ok !== true) {
            yield {
              type: 'failed',
              code: 'DURABLE_REPLAY_FAILED',
              message:
                receipt.value.result_error?.trim() ||
                'Durable NomiFun replay completed without a successful result',
              retryable: false,
            };
            return;
          }
          const resolution = await reconcileCompleted();
          for (const event of completedEvents(resolution)) yield event;
          return;
        }
        admittedNonTerminalTurn = true;

        if (!receipt.value.replayed && beforeSend.authority !== 'idle') {
          throw new NomiCreativeStudioAgentRuntimeError(
            'CONVERSATION_NOT_IDLE',
            'A fresh Agent turn was admitted while its exclusive conversation was already running'
          );
        }

        if (receipt.value.replayed) {
          const afterSend = await transport.inspect(binding.conversationId);
          if (!sameModel(afterSend.model, request.model)) {
            throw new NomiCreativeStudioAgentBindingError(
              'Nomi conversation selected model changed after replay admission'
            );
          }
          if (afterSend.authority === 'processing' && afterSend.activeTurnId) {
            // WebSocket events are not replayed. An accepted replay may already
            // own a live turn, so adopt only the exact active_turn_id from a
            // fresh authoritative GET. A fresh admission still has to observe
            // and verify its own turn.started event below.
            activeTurnId = afterSend.activeTurnId;
          } else if (afterSend.authority === 'idle') {
            const resolution = await reconcileCompleted();
            for (const event of completedEvents(resolution)) yield event;
            return;
          } else {
            throw new NomiCreativeStudioAgentRuntimeError(
              'REPLAY_RUNTIME_UNRESOLVED',
              'Accepted NomiFun replay has no authoritative active turn or terminal receipt'
            );
          }
        }

        for (;;) {
          const queued = await queue.next(
            request.signal,
            activeTurnId ? recoveryPollMs : turnStartTimeoutMs
          );
          if (queued.kind === 'aborted') {
            return await stopAfterAbort(transport, binding.conversationId);
          }
          if (queued.kind === 'timeout') {
            if (activeTurnId) {
              const snapshot = await transport.inspect(binding.conversationId);
              if (!sameModel(snapshot.model, request.model)) {
                throw new NomiCreativeStudioAgentBindingError(
                  'Nomi conversation selected model changed during recovery'
                );
              }
              if (
                snapshot.authority === 'processing' &&
                snapshot.activeTurnId === activeTurnId
              ) {
                continue;
              }
              if (snapshot.authority === 'idle') {
                const resolution = await reconcileCompleted();
                for (const event of completedEvents(resolution)) yield event;
                return;
              }
              throw new NomiCreativeStudioAgentRuntimeError(
                'REPLAY_RUNTIME_UNRESOLVED',
                'NomiFun replay lost its authoritative active turn before durable completion'
              );
            }
            throw new NomiCreativeStudioAgentRuntimeError(
              'TURN_START_TIMEOUT',
              'NomiFun accepted the message but no authoritative turn start was observed'
            );
          }

          const runtimeEvent = queued.value;
          if (runtimeEvent.kind === 'reconnected') {
            const snapshot = await transport.inspect(binding.conversationId);
            if (
              snapshot.authority === 'processing' &&
              snapshot.activeTurnId &&
              (!activeTurnId || snapshot.activeTurnId === activeTurnId)
            ) {
              activeTurnId = snapshot.activeTurnId;
              yield { type: 'activity', label: '连接已恢复，Agent 仍在运行' };
              continue;
            }
            if (snapshot.authority === 'idle' && activeTurnId) {
              const resolution = await reconcileCompleted();
              for (const event of completedEvents(resolution)) yield event;
              return;
            }
            throw new NomiCreativeStudioAgentRuntimeError(
              'RECONNECTED_TERMINAL_UNKNOWN',
              'NomiFun stream reconnected after the turn ended without a correlated terminal event'
            );
          }

          if (runtimeEvent.kind === 'turn-started') {
            if (!activeTurnId && (await verifyStartedTurn(transport, binding, runtimeEvent.value))) {
              activeTurnId = runtimeEvent.value.turn_id;
              yield {
                type: 'activity',
                label:
                  runtimeEvent.value.detail.trim() ||
                  runtimeEvent.value.phase ||
                  'Agent 已开始执行',
              };
            }
            continue;
          }

          if (runtimeEvent.kind === 'response') {
            const event = runtimeEvent.value;
            if (
              event.conversation_id !== binding.conversationId ||
              !activeTurnId ||
              event.turn_id !== activeTurnId
            ) {
              continue;
            }

            if (event.type === 'error') {
              yield {
                type: 'failed',
                code: 'NOMI_STREAM_ERROR',
                message: errorText(event),
                retryable: true,
              };
              return;
            }

            if (event.type === 'content' || event.type === 'text') {
              const chunk = extractResponseTextChunk(event.data);
              if (chunk) {
                const previous = streamedByMessageId.get(event.msg_id) ?? '';
                const appended = appendOnlyDelta(previous, chunk, event.replace === true);
                streamedByMessageId.set(event.msg_id, appended.text);
                if (appended.delta) {
                  emittedAssistantText += appended.delta;
                  yield { type: 'assistant-delta', delta: appended.delta };
                }
              }
            }

            const activity = activityFromResponse(event);
            if (activity) yield { type: 'activity', label: activity };
            continue;
          }

          const event = runtimeEvent.value;
          if (
            event.conversation_id !== binding.conversationId ||
            !activeTurnId ||
            event.turn_id !== activeTurnId ||
            event.runtime.is_processing ||
            event.runtime.active_turn_id != null
          ) {
            continue;
          }

          if (event.state === 'error' || event.state === 'stopped') {
            yield {
              type: 'failed',
              code: event.state === 'stopped' ? 'TURN_STOPPED_EXTERNALLY' : 'TURN_FAILED',
              message: event.detail.trim() || `NomiFun Agent turn ended with state ${event.state}`,
              retryable: event.state === 'error',
            };
            return;
          }

          if (event.status !== 'finished' || event.last_message.status === 'error') {
            yield {
              type: 'failed',
              code: 'TERMINAL_NOT_SUCCESSFUL',
              message:
                event.detail.trim() ||
                'NomiFun turn reached idle without an authoritative successful terminal status',
              retryable: false,
            };
            return;
          }

          const finalText = extractResponseTextChunk(event.last_message.content);
          if (finalText) {
            // The terminal row may have a different durable message id from a
            // streamed fragment. Reconcile against the text already emitted to
            // the small port, not against ids, so no duplicate reply is made.
            const appended = appendOnlyDelta(emittedAssistantText, finalText, true);
            if (appended.delta) {
              emittedAssistantText += appended.delta;
              yield { type: 'assistant-delta', delta: appended.delta };
            }
          }
          const resolution = await reconcileCompleted();
          for (const completedEvent of completedEvents(resolution)) yield completedEvent;
          return;
        }
      } catch (error) {
        // A local correlation/protocol failure after REST admission must not
        // leave an exclusive Creative Studio conversation executing invisibly.
        // Abort already owns the same stop-confirm path and must not run twice.
        if (admittedNonTerminalTurn && !request.signal.aborted) {
          await transport.stopAndConfirm(binding.conversationId);
        }
        throw error;
      } finally {
        for (const dispose of unsubscribe) dispose();
      }
    },
  };
}
