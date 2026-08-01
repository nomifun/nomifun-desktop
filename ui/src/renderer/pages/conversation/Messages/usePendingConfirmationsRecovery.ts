/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */
import type { ConversationId } from '@/common/types/ids';

import { ipcBridge } from '@/common';
import type { IConfirmation, IMessagePermission, TMessage } from '@/common/chat/chatLib';
import { useEffect } from 'react';
import { useUpdateMessageList } from './hooks';

export const pendingConfirmationMsgId = (confirmationId: string) => `confirmation:${confirmationId}`;

export function buildPendingConfirmationMessage(
  conversation_id: ConversationId,
  confirmation: IConfirmation<unknown>
): IMessagePermission {
  return {
    id: pendingConfirmationMsgId(confirmation.id),
    type: 'permission',
    position: 'left',
    conversation_id,
    created_at: Date.now(),
    content: confirmation,
  };
}

export function hasPermissionMessageForCallId(list: TMessage[], callId: string): boolean {
  return list.some((message) => message.type === 'permission' && message.content?.call_id === callId);
}

export function removePermissionMessage(list: TMessage[], target: { id?: string; call_id?: string }): TMessage[] {
  return list.filter((message) => {
    if (message.type !== 'permission') return true;
    if (target.id && message.content.id === target.id) return false;
    if (target.call_id && message.content.call_id === target.call_id) return false;
    return true;
  });
}

function errorMessage(error: unknown): string {
  return error instanceof Error ? error.message : String(error);
}

export function usePendingConfirmationsRecovery(conversation_id: ConversationId, options?: { enabled?: boolean }) {
  const updateMessageList = useUpdateMessageList();
  const enabled = options?.enabled ?? true;

  useEffect(() => {
    if (!enabled || !conversation_id) return;
    let cancelled = false;

    const recoverPendingConfirmations = () => {
      void ipcBridge.conversation.confirmation.list
        .invoke({ conversation_id })
        .then((confirmations) => {
          if (cancelled) return;
          const pending = confirmations ?? [];
          const pendingCallIds = new Set(pending.map((confirmation) => confirmation.call_id));
          updateMessageList((list) => {
            // The fetched set is the authoritative pending list. A recovery-
            // created card (id prefixed `confirmation:`) whose confirmation is
            // no longer pending was resolved while delivery was gapped — its
            // `confirmation.remove` event is lost forever, so drop it here.
            // Stream-created permission cards keep their own lifecycle: they
            // may be newer than this snapshot (raised while the fetch was in
            // flight) and must not be judged by it.
            let next = list.filter((message) => {
              if (message.type !== 'permission') return true;
              if (!message.id.startsWith('confirmation:')) return true;
              const callId = message.content?.call_id;
              return callId ? pendingCallIds.has(callId) : true;
            });
            for (const confirmation of pending) {
              if (hasPermissionMessageForCallId(next, confirmation.call_id)) continue;
              next = next.concat(buildPendingConfirmationMessage(conversation_id, confirmation));
            }
            return next;
          });
        })
        .catch((error) => {
          console.warn('[pending-confirmations] failed to recover pending confirmations', {
            conversation_id,
            error: errorMessage(error),
          });
        });
    };

    recoverPendingConfirmations();

    // WebSocket delivery has no replay: a confirmation raised while delivery
    // was gapped never arrives as an event, so re-run the durable recovery
    // fetch after every reconnect.
    const offReconnected = ipcBridge.conversation.reconnected.on(() => {
      recoverPendingConfirmations();
    });

    const off = ipcBridge.conversation.confirmation.remove.on((event) => {
      if (event.conversation_id !== conversation_id) return;
      updateMessageList((list) => removePermissionMessage(list, { id: event.id, call_id: event.id }));
    });

    return () => {
      cancelled = true;
      offReconnected();
      off();
    };
  }, [conversation_id, enabled, updateMessageList]);
}
