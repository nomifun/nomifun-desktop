/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { conversation } from '@/common/adapter/ipcBridge';
import { getConversationRuntimeAuthority } from '@/renderer/pages/conversation/utils/conversationRuntime';
import {
  stopConversationAndConfirmRelease,
  waitForConversationTurnReleaseUntilSettled,
} from '@/renderer/pages/conversation/platforms/requestConversationStop';

import type { NomiCreativeStudioAgentTransport } from './types';

/**
 * Real transport over NomiFun's existing conversation REST and WebSocket
 * services. It imports no conversation page component, provider catalog or
 * router state.
 */
export function createNomiCreativeStudioAgentTransport(): NomiCreativeStudioAgentTransport {
  return {
    async inspect(conversationId) {
      const snapshot = await conversation.get.invoke({ conversation_id: conversationId });
      return {
        conversationId: snapshot.id,
        model: {
          providerId: snapshot.model.id,
          model: snapshot.model.use_model,
        },
        authority: getConversationRuntimeAuthority(snapshot),
        ...(snapshot.runtime?.active_turn_id
          ? { activeTurnId: snapshot.runtime.active_turn_id }
          : {}),
      };
    },

    sendMessage({ conversationId, modelInput, skillIds, idempotencyKey }) {
      return conversation.sendMessage.invoke({
        conversation_id: conversationId,
        input: modelInput,
        idempotency_key: idempotencyKey,
        inject_skills: [...skillIds],
      });
    },

    async stopAndConfirm(conversationId) {
      const initial = await stopConversationAndConfirmRelease(conversationId);
      if (initial.status === 'released' || initial.status === 'deleted') return;

      // Once cancellation was requested, never turn a transport ambiguity into
      // a false "stopped" result. Existing authoritative reconciliation keeps
      // polling until the runtime is truly idle/deleted.
      for (;;) {
        const settled = await waitForConversationTurnReleaseUntilSettled(conversationId);
        if (settled === 'released' || settled === 'deleted') return;
      }
    },

    onResponse: (listener) => conversation.responseStream.on(listener),
    onTurnStarted: (listener) => conversation.turnStarted.on(listener),
    onTurnCompleted: (listener) => conversation.turnCompleted.on(listener),
    onReconnected: (listener) => conversation.reconnected.on(listener),
  };
}
