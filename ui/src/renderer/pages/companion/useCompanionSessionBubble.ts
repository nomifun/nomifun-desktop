/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { useCallback, useEffect, useRef, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { ipcBridge } from '@/common';
import { isBackendHttpError } from '@/common/adapter/httpBridge';
import type { IConversationTurnCompletedEvent, IResponseMessage, IUserMessageCreatedEvent } from '@/common/adapter/ipcBridge';
import type { TMessage } from '@/common/chat/chatLib';
import type { CompanionId, ConversationId, MessageId } from '@/common/types/ids';
import { reconcileConversationAuthoritativeRuntime, terminalReconcileDelayForAttempt } from '../conversation/platforms/reconcileConversationTurnAfterStreamTerminal';
import { browserNarrationFor } from './browserNarration';
import { companionErrorKey, streamErrorCode } from './companionError';
import {
  beginCompanionSessionBubble, clearCompanionSessionBubble, emptyCompanionSessionBubble, hydrateCompanionSessionBubble,
  settleCompanionSessionBubble, streamCompanionSessionBubble, type CompanionSessionBubbleState,
} from './companionSessionBubble';

const COMPLETED_BUBBLE_MS = 24_000;

/** Every sending surface observes the same canonical Session and Turn. */
export function useCompanionSessionBubble(companionId: CompanionId | null, companionName?: string) {
  const { t } = useTranslation();
  const tRef = useRef(t);
  tRef.current = t;
  const nameRef = useRef('Nomi');
  nameRef.current = companionName?.trim() || 'Nomi';
  const [state, setState] = useState(() => emptyCompanionSessionBubble(null));
  const stateRef = useRef(state);
  const bindingGeneration = useRef(0);
  const reconciliationGeneration = useRef(0);
  const bindRef = useRef<(receipt?: MessageId) => void>(() => {});
  const reconcileRef = useRef<(forceHistory?: boolean) => void>(() => {});
  const hoveredRef = useRef(false);
  const dismissTimer = useRef<ReturnType<typeof setTimeout> | null>(null);

  const update = useCallback((change: (current: CompanionSessionBubbleState) => CompanionSessionBubbleState) => {
    const next = change(stateRef.current);
    if (next === stateRef.current) return;
    stateRef.current = next;
    setState(next);
  }, []);
  const clearTimer = useCallback(() => {
    if (dismissTimer.current) clearTimeout(dismissTimer.current);
    dismissTimer.current = null;
  }, []);
  const armCompletedDismiss = useCallback(() => {
    clearTimer();
    const { conversationId, turnId, phase, bubble } = stateRef.current;
    if (phase !== 'settled' || !bubble || hoveredRef.current) return;
    dismissTimer.current = setTimeout(() => {
      dismissTimer.current = null;
      if (!hoveredRef.current && stateRef.current.conversationId === conversationId) {
        update(current => clearCompanionSessionBubble(current, turnId));
      }
    }, COMPLETED_BUBBLE_MS);
  }, [clearTimer, update]);

  useEffect(() => {
    clearTimer();
    update(() => emptyCompanionSessionBubble(null));
    const binding = ++bindingGeneration.current;
    let disposed = false;
    let resolveGeneration = 0;
    let historyGeneration = 0;
    let bindingRetry: ReturnType<typeof setTimeout> | null = null;
    let historyRetry: ReturnType<typeof setTimeout> | null = null;
    let hydratedRoot: MessageId | null = null;
    let hydratingRoot: MessageId | null = null;
    const pendingStreams: IResponseMessage[] = [];
    const pendingAccepted = new Map<ConversationId, IUserMessageCreatedEvent>();
    const headers = new Map<MessageId, { platform: string; inbound: string }>();
    const pendingCompleted = new Map<ConversationId, IConversationTurnCompletedEvent>();
    // Remember only presentation fences for observed roots in this binding.
    const settledRoots = new Set<MessageId>();
    const currentBinding = () => !disposed && bindingGeneration.current === binding;
    const clearHistoryRetry = () => {
      if (historyRetry) clearTimeout(historyRetry);
      historyRetry = null;
    };
    const doneText = () => tRef.current('nomi.companion.done');
    const applyStream = (message: IResponseMessage) => {
      const browser = browserNarrationFor(message.data);
      const toolHint = browser ? tRef.current(browser.key, { name: nameRef.current, ...browser.params })
        : tRef.current('nomi.companion.usingTools', { name: nameRef.current });
      update(current => streamCompanionSessionBubble(current, message, toolHint,
        tRef.current(companionErrorKey(streamErrorCode(message.data)))));
    };
    const hydrate = (root: MessageId, force = false, retryAttempt = 0) => {
      const conversationId = stateRef.current.conversationId;
      if (!conversationId || (!force && (hydratedRoot === root || hydratingRoot === root))) return;
      clearHistoryRetry();
      hydratingRoot = root;
      const generation = ++historyGeneration;
      const isCurrent = () => currentBinding() && historyGeneration === generation
        && stateRef.current.conversationId === conversationId && stateRef.current.turnId === root;
      const retryHistory = () => {
        if (!isCurrent()) return;
        hydratedRoot = null;
        historyRetry = setTimeout(() => {
          historyRetry = null;
          if (isCurrent()) hydrate(root, true, retryAttempt + 1);
        }, terminalReconcileDelayForAttempt(retryAttempt));
      };
      void (async () => {
        for (let attempt = 0; attempt < 2 && isCurrent(); attempt += 1) {
          const observed = stateRef.current;
          const messages: TMessage[] = [];
          let cursor = '';
          while (isCurrent()) {
            const page = await ipcBridge.database.getConversationMessages.invoke({ conversation_id: conversationId,
              cursor, page_size: 100, order: 'DESC', content_mode: 'full' });
            if (!isCurrent()) return;
            messages.push(...page.items);
            // Walk only the current canonical Turn, stopping at its accepted root
            // or the preceding Turn. Tool-heavy turns can span several pages.
            if (!page.has_more || page.items.some(message => message.message_id === root || message.msg_id === root
              || (message.turn_id != null && message.turn_id !== root))) break;
            const oldest = page.items.at(-1);
            if (!oldest?.message_id || oldest.created_at == null) break;
            const nextCursor = `${oldest.created_at}:${oldest.message_id}`;
            if (nextCursor === cursor) break;
            cursor = nextCursor;
          }
          if (!isCurrent()) return;
          const raced = messages.some(message => {
            const id = message.msg_id ?? message.message_id;
            return id && message.turn_id === root && message.type === 'text'
              && stateRef.current.segmentRevisions.get(id) !== observed.segmentRevisions.get(id);
          });
          update(current => hydrateCompanionSessionBubble(current, messages, observed));
          // A concurrent live update wins the first read. Re-read once against
          // that update so a lost prefix can be recovered without guessing overlap.
          if (!raced) {
            hydratedRoot = root;
            hydratingRoot = null;
            return;
          }
        }
        // Continuous streaming can race both immediate reads. Keep recovery
        // pending and retry with capped backoff rather than accepting lost text.
        retryHistory();
      })().catch(error => {
        if (isCurrent()) {
          console.warn('companion Turn text could not be recovered:', error);
          retryHistory();
        }
      });
    };
    const settle = (root: MessageId, paused = false) => {
      if (!currentBinding() || stateRef.current.turnId !== root) return;
      if (!paused) settledRoots.add(root);
      update(current => settleCompanionSessionBubble(current, root, doneText(), paused));
      if (!paused) hydrate(root, true);
    };
    const begin = (root: MessageId, phase: 'accepted' | 'running') => {
      if (settledRoots.has(root)) return;
      if (stateRef.current.turnId && stateRef.current.turnId !== root) {
        settledRoots.add(stateRef.current.turnId);
        ++historyGeneration;
        hydratedRoot = null;
        hydratingRoot = null;
        clearHistoryRetry();
      }
      clearTimer();
      update(current => {
        const next = beginCompanionSessionBubble(current, root, phase);
        return next.remoteHeader === (headers.get(root) ?? null) ? next : { ...next, remoteHeader: headers.get(root) ?? null };
      });
      for (const message of pendingStreams) if (message.turn_id === root) applyStream(message);
      pendingStreams.splice(0, pendingStreams.length);
      if (phase === 'running') hydrate(root);
    };
    const reconcileRuntime = (forceHistory = false) => {
      const conversationId = stateRef.current.conversationId;
      if (!currentBinding() || !conversationId) return;
      const generation = ++reconciliationGeneration.current;
      const isCurrent = () => currentBinding() && reconciliationGeneration.current === generation
        && stateRef.current.conversationId === conversationId;
      void reconcileConversationAuthoritativeRuntime(conversationId, {
        isCurrent,
        onProcessing: conversation => {
          const root = conversation.runtime?.active_turn_id;
          if (root) {
            begin(root, 'running');
            if (forceHistory) hydrate(root, true);
          }
        },
        onPaused: conversation => {
          const root = conversation.runtime?.active_turn_id;
          if (root && !settledRoots.has(root)) {
            begin(root, 'running');
            settle(root, true);
          }
        },
        onIdle: () => {
          const root = stateRef.current.turnId;
          if (root) settle(root);
        },
        announceSettled: false,
        logLabel: 'companion bubble',
      });
    };
    reconcileRef.current = reconcileRuntime;
    const resolveBinding = (receipt?: MessageId, retryAttempt = 0) => {
      if (!companionId || !currentBinding()) return;
      if (bindingRetry) clearTimeout(bindingRetry);
      bindingRetry = null;
      const resolving = ++resolveGeneration;
      void ipcBridge.companion.getCompanionSession.invoke({ companion_id: companionId }).then(active => {
        if (!currentBinding() || resolving !== resolveGeneration) return;
        if (active.conversation_id !== stateRef.current.conversationId) {
          ++reconciliationGeneration.current;
          settledRoots.clear();
          headers.clear();
          hydratedRoot = null;
          hydratingRoot = null;
          clearHistoryRetry();
          ++historyGeneration;
          clearTimer();
          update(() => emptyCompanionSessionBubble(active.conversation_id));
        }
        const accepted = active.conversation_id ? pendingAccepted.get(active.conversation_id) : undefined;
        if (accepted && !settledRoots.has(accepted.msg_id)) {
          if (accepted.channel_platform) headers.set(accepted.msg_id, { platform: accepted.channel_platform, inbound: accepted.content });
          begin(accepted.msg_id, 'accepted');
        }
        const completed = active.conversation_id ? pendingCompleted.get(active.conversation_id) : undefined;
        if (completed?.turn_id && !stateRef.current.turnId) begin(completed.turn_id, 'accepted');
        if (completed?.turn_id && completed.turn_id === stateRef.current.turnId) {
          settle(completed.turn_id);
        }
        // A POST receipt is an identity hint, never execution authority.
        // If its stream already settled, a delayed response cannot reopen it.
        if (receipt && settledRoots.has(receipt)) return;
        reconcileRuntime();
      }).catch(error => {
        if (!currentBinding() || resolving !== resolveGeneration) return;
        console.warn('companion Session binding could not be read:', error);
        if (isBackendHttpError(error) && error.status === 404) return;
        bindingRetry = setTimeout(() => {
          bindingRetry = null;
          if (currentBinding() && resolving === resolveGeneration) resolveBinding(receipt, retryAttempt + 1);
        }, terminalReconcileDelayForAttempt(retryAttempt));
      });
    };
    bindRef.current = resolveBinding;
    if (!companionId) return () => { disposed = true; };
    const unsubs = [
      ipcBridge.conversation.userCreated.on(event => {
        if (!currentBinding() || event.hidden) return;
        if (event.conversation_id !== stateRef.current.conversationId) {
          // A freshly ensured companion Session may not have existed at mount.
          if (!stateRef.current.conversationId) {
            pendingAccepted.set(event.conversation_id, event);
            if (pendingAccepted.size > 16) pendingAccepted.delete(pendingAccepted.keys().next().value!);
            resolveBinding();
          }
          return;
        }
        if (settledRoots.has(event.msg_id)) return;
        if (event.channel_platform) headers.set(event.msg_id, { platform: event.channel_platform, inbound: event.content });
        if (!stateRef.current.turnId || stateRef.current.turnId === event.msg_id || stateRef.current.phase === 'settled') {
          begin(event.msg_id, 'accepted');
        }
        reconcileRuntime();
      }),
      ipcBridge.conversation.turnStarted.on(event => {
        if (!currentBinding() || event.conversation_id !== stateRef.current.conversationId
          || !event.runtime.is_processing || event.runtime.active_turn_id !== event.turn_id
          || settledRoots.has(event.turn_id)) return;
        // A start for another root must be checked against the live aggregate.
        if (stateRef.current.turnId === event.turn_id) begin(event.turn_id, 'running');
        reconcileRuntime();
      }),
      ipcBridge.conversation.turnCompleted.on(event => {
        if (!currentBinding() || !event.turn_id || event.runtime.is_processing || event.runtime.active_turn_id != null
          || event.runtime.can_send_message !== true) return;
        if (!stateRef.current.conversationId) {
          pendingCompleted.set(event.conversation_id, event);
          if (pendingCompleted.size > 16) pendingCompleted.delete(pendingCompleted.keys().next().value!);
          return;
        }
        if (event.conversation_id !== stateRef.current.conversationId) return;
        if (!stateRef.current.turnId) begin(event.turn_id, 'accepted');
        if (event.turn_id !== stateRef.current.turnId) return;
        ++reconciliationGeneration.current;
        settle(event.turn_id);
      }),
      ipcBridge.conversation.turnPaused.on(event => {
        if (event.conversation_id === stateRef.current.conversationId && event.turn_id === stateRef.current.turnId) reconcileRuntime();
      }),
      ipcBridge.conversation.responseStream.on(message => {
        if (!currentBinding() || !message.turn_id) return;
        if (!stateRef.current.conversationId || (message.conversation_id === stateRef.current.conversationId
          && message.turn_id !== stateRef.current.turnId && !settledRoots.has(message.turn_id))) {
          if (!message.stream_complete) {
            pendingStreams.push(message);
            if (pendingStreams.length > 128) pendingStreams.shift();
          }
          return;
        }
        if (message.conversation_id !== stateRef.current.conversationId || message.turn_id !== stateRef.current.turnId) return;
        applyStream(message);
        if (!message.hidden && !message.stream_complete && (message.type === 'finish' || message.type === 'error')) {
          // Terminal stream is a trigger to read authority; it does not lower busy.
          reconcileRuntime();
        }
      }),
      ipcBridge.conversation.reconnected.on(() => {
        hydratedRoot = null;
        hydratingRoot = null;
        ++historyGeneration;
        clearHistoryRetry();
        resolveBinding();
      }),
      ipcBridge.companion.onConfigUpdated.on(event => {
        if (event.scope === companionId) resolveBinding();
      }),
    ];
    resolveBinding();
    return () => {
      disposed = true;
      if (bindingGeneration.current === binding) ++bindingGeneration.current;
      ++reconciliationGeneration.current;
      bindRef.current = () => {};
      reconcileRef.current = () => {};
      if (bindingRetry) clearTimeout(bindingRetry);
      clearHistoryRetry();
      clearTimer();
      unsubs.forEach(unsubscribe => unsubscribe());
    };
  }, [clearTimer, companionId, update]);

  useEffect(() => { armCompletedDismiss(); }, [state.bubble, state.phase, state.turnId, armCompletedDismiss]);
  const setHovered = useCallback((hovered: boolean) => {
    hoveredRef.current = hovered;
    if (hovered) clearTimer();
    else armCompletedDismiss();
  }, [armCompletedDismiss, clearTimer]);
  const clearCompleted = useCallback(() => {
    const root = stateRef.current.turnId;
    update(current => clearCompanionSessionBubble(current, root));
  }, [update]);
  const stopExactTurn = useCallback(() => {
    const { conversationId, turnId } = stateRef.current;
    if (!conversationId || !turnId) return;
    const binding = bindingGeneration.current;
    void ipcBridge.conversation.stop.invoke({ conversation_id: conversationId, expected_turn_id: turnId })
      .catch(error => {
        console.warn('companion Turn could not be interrupted:', error);
        if (binding === bindingGeneration.current && stateRef.current.conversationId === conversationId
          && stateRef.current.turnId === turnId && !stateRef.current.dismissed) {
          update(current => ({ ...current, interrupted: false }));
        }
      })
      .finally(() => {
        if (binding === bindingGeneration.current && stateRef.current.turnId === turnId) reconcileRef.current(true);
      });
  }, [update]);
  const dismiss = useCallback(() => {
    clearTimer();
    update(current => ({ ...current, bubble: '', loading: false, dismissed: true }));
    if (stateRef.current.phase === 'running' || stateRef.current.phase === 'accepted') stopExactTurn();
  }, [clearTimer, stopExactTurn, update]);
  const interrupt = useCallback(() => {
    update(current => ({ ...current, interrupted: true, loading: false }));
    stopExactTurn();
  }, [stopExactTurn, update]);
  const reconcile = useCallback((receipt?: MessageId) => bindRef.current(receipt), []);
  return { conversationId: state.conversationId, turnId: state.turnId, bubble: state.bubble,
    remoteHeader: state.remoteHeader,
    phase: state.phase, running: state.phase === 'running', loading: state.loading,
    dismiss, interrupt, reconcile, clearCompleted, setHovered };
}
