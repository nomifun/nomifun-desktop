/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */
import type { ConversationId } from '@/common/types/ids';

import { ipcBridge } from '@/common';
import { transformMessage } from '@/common/chat/chatLib';
import { useEffect } from 'react';
import { useAddOrUpdateMessage } from './hooks';

/**
 * Owns message rendering for the basic conversation runtimes whose composer
 * only needs to track local busy state. Keeping this subscription at the chat
 * surface lets immutable transcripts stay live even when no SendBox is mounted.
 */
export function useConversationResponseMessages(conversation_id: ConversationId): void {
  const addOrUpdateMessage = useAddOrUpdateMessage();

  useEffect(() => {
    return ipcBridge.conversation.responseStream.on((message) => {
      if (message.conversation_id !== conversation_id || message.type === 'thought' || message.type === 'finish') {
        return;
      }

      const transformedMessage = transformMessage(message);
      if (transformedMessage) {
        addOrUpdateMessage(transformedMessage);
      }
    });
  }, [addOrUpdateMessage, conversation_id]);
}
