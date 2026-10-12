/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { ipcBridge } from '@/common';
import type { TChatConversation } from '@/common/config/storage';
import type { ConversationId } from '@/common/types/ids';
import { emitter } from '@/renderer/utils/emitter';
import { useCallback, useEffect, useMemo, useState } from 'react';

export const useBatchSelection = (batchMode: boolean, conversations: TChatConversation[]) => {
  const [selectedConversationIds, setSelectedConversationIds] = useState<Set<ConversationId>>(new Set());

  // Reset selection when batch mode is turned off
  useEffect(() => {
    if (!batchMode) {
      setSelectedConversationIds(new Set());
    }
  }, [batchMode]);

  // Absence from the live list is not a deletion receipt: a fenced deletion
  // whose cleanup failed is hidden there but must remain selectable for retry.
  useEffect(() => {
    const removeDeleted = (id: ConversationId) => {
      setSelectedConversationIds((previous) => {
        if (!previous.has(id)) return previous;
        const next = new Set(previous);
        next.delete(id);
        return next;
      });
    };
    emitter.on('conversation.deleted', removeDeleted);
    const off = ipcBridge.conversation.listChanged.on((event) => {
      if (event.action === 'deleted') removeDeleted(event.conversation_id);
    });
    return () => {
      emitter.off('conversation.deleted', removeDeleted);
      off();
    };
  }, []);

  const allConversationIds = useMemo(() => conversations.map((conversation) => conversation.id), [conversations]);
  const selectedCount = selectedConversationIds.size;
  const allSelected = allConversationIds.length > 0 && allConversationIds.every((id) => selectedConversationIds.has(id));

  const toggleSelectedConversation = useCallback((conversation: TChatConversation) => {
    setSelectedConversationIds((prev) => {
      const next = new Set(prev);
      if (next.has(conversation.id)) {
        next.delete(conversation.id);
      } else {
        next.add(conversation.id);
      }
      return next;
    });
  }, []);

  const handleToggleSelectAll = useCallback(() => {
    setSelectedConversationIds((prev) => {
      if (allConversationIds.length > 0 && allConversationIds.every((id) => prev.has(id))) {
        return new Set();
      }
      return new Set([...prev, ...allConversationIds]);
    });
  }, [allConversationIds]);

  return {
    selectedConversationIds,
    setSelectedConversationIds,
    selectedCount,
    allSelected,
    toggleSelectedConversation,
    handleToggleSelectAll,
  };
};
