/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 * Based on AionUi (https://github.com/iOfficeAI/AionUi)
 */

import React, { createContext, useContext } from 'react';
import { useConversationListSync } from '@/renderer/pages/conversation/SessionList/hooks/useConversationListSync';

export type ConversationHistoryContextValue = ReturnType<typeof useConversationListSync>;

const ConversationHistoryContext = createContext<ConversationHistoryContextValue | null>(null);

export const ConversationHistoryProvider: React.FC<React.PropsWithChildren> = ({ children }) => {
  const conversationListSync = useConversationListSync();

  return (
    <ConversationHistoryContext.Provider value={conversationListSync}>{children}</ConversationHistoryContext.Provider>
  );
};

export const useConversationHistoryContext = (): ConversationHistoryContextValue => {
  const context = useContext(ConversationHistoryContext);

  if (!context) {
    throw new Error('useConversationHistoryContext must be used within ConversationHistoryProvider');
  }

  return context;
};
