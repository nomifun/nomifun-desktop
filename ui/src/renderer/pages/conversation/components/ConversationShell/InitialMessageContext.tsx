/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import React, { createContext, useCallback, useContext, useMemo, useState } from 'react';
import type { ConversationId } from '@/common/types/ids';
import type { PersistedInitialMessage } from '../../platforms/initialMessageDelivery';

/** A live submit's presentation only; never a persisted message or Turn fact. */
export type PendingInitialMessage = PersistedInitialMessage & { submittedAt: number };

type InitialMessageContextValue = {
  pending: PendingInitialMessage | null;
  begin: (message: PendingInitialMessage) => void;
  /** Only this exact delivery may retire its presentation. */
  end: (conversationId: ConversationId, idempotencyKey: string) => void;
};

const InitialMessageContext = createContext<InitialMessageContextValue>({
  pending: null,
  begin: () => undefined,
  end: () => undefined,
});

/** Persists across the welcome-to-conversation navigation, without an overlay. */
export const InitialMessageProvider: React.FC<{ children: React.ReactNode }> = ({ children }) => {
  const [pending, setPending] = useState<PendingInitialMessage | null>(null);
  const begin = useCallback((message: PendingInitialMessage) => setPending(message), []);
  const end = useCallback((conversationId: ConversationId, idempotencyKey: string) => {
    setPending(current => current?.conversation_id === conversationId && current.idempotency_key === idempotencyKey
      ? null : current);
  }, []);
  const value = useMemo(() => ({ pending, begin, end }), [pending, begin, end]);
  return <InitialMessageContext.Provider value={value}>{children}</InitialMessageContext.Provider>;
};

export const useInitialMessage = () => useContext(InitialMessageContext);
