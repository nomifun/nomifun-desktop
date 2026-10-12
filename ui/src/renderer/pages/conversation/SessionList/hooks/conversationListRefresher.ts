/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import type { TChatConversation } from '@/common/config/storage';
import type { ConversationId } from '@/common/types/ids';

type ConversationListRefresherOptions = {
  load: () => Promise<TChatConversation[]>;
  apply: (conversations: TChatConversation[]) => void;
  remove: (conversationId: ConversationId) => void;
  onError: (error: unknown) => void;
};

/**
 * Keep one history read in flight and combine refresh events received during it.
 * Every completed read can publish, even while turns keep producing events.
 * Deletions observed during that read are replayed over its older snapshot.
 */
export const createConversationListRefresher = ({
  load,
  apply,
  remove,
  onError,
}: ConversationListRefresherOptions) => {
  let refreshRequested = false;
  let inFlight: Promise<void> | null = null;
  let removedDuringRead: Set<ConversationId> | null = null;

  const drain = async () => {
    try {
      while (refreshRequested) {
        refreshRequested = false;
        const removed = new Set<ConversationId>();
        removedDuringRead = removed;
        try {
          const conversations = await load();
          apply(conversations.filter((conversation) => !removed.has(conversation.id)));
        } catch (error) {
          // A failed read cannot establish that any existing Session is gone.
          onError(error);
        } finally {
          removedDuringRead = null;
        }
      }
    } finally {
      inFlight = null;
    }
  };

  const refresh = () => {
    refreshRequested = true;
    if (!inFlight) inFlight = Promise.resolve().then(drain);
    return inFlight;
  };

  const deleted = (conversationId: ConversationId) => {
    removedDuringRead?.add(conversationId);
    remove(conversationId);
    return refresh();
  };

  return { refresh, deleted };
};
