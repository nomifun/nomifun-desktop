/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { useEffect } from 'react';
import { ipcBridge } from '@/common';
import type { ConversationId } from '@/common/types/ids';

/** Refresh the shared Session projection for sidebar controls as well as chat. */
export function useCompanionConversationUpdates(conversationId: ConversationId | null, refresh: () => unknown) {
  useEffect(() => {
    if (!conversationId) return;
    // SWR retains its read error. The owner disables editing while that read is
    // unavailable; failed event refreshes must not become unhandled promises.
    const update = () => { void Promise.resolve(refresh()).catch(() => {}); };
    const updateTurn = (event: { conversation_id: ConversationId }) => {
      if (event.conversation_id === conversationId) update();
    };
    const unsubscribes = [
      ipcBridge.conversation.listChanged.on((event) => {
        if (event.conversation_id === conversationId) update();
      }),
      ipcBridge.agentPlatform.sessions.onAgentChanged.on((event) => {
        if (String(event.agent_session_id) === String(conversationId)) update();
      }),
      ipcBridge.conversation.turnStarted.on(updateTurn),
      ipcBridge.conversation.turnPaused.on(updateTurn),
      ipcBridge.conversation.turnCompleted.on(updateTurn),
      ipcBridge.conversation.reconnected.on(update),
    ];
    return () => unsubscribes.forEach((unsubscribe) => unsubscribe());
  }, [conversationId, refresh]);
}
