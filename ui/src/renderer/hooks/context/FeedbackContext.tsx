/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import React, { createContext, useCallback, useContext, useMemo, useState } from 'react';
import FeedbackReportModal from '@/renderer/components/settings/SettingsModal/contents/FeedbackReportModal';

type FeedbackContextValue = {
  openFeedback: () => Promise<void>;
};

const FeedbackContext = createContext<FeedbackContextValue | null>(null);

export const FeedbackProvider: React.FC<{ children: React.ReactNode }> = ({ children }) => {
  const [visible, setVisible] = useState(false);

  const openFeedback = useCallback(async () => {
    setVisible(true);
  }, []);

  const handleCancel = useCallback(() => {
    setVisible(false);
  }, []);

  const value = useMemo(() => ({ openFeedback }), [openFeedback]);

  return (
    <FeedbackContext.Provider value={value}>
      {children}
      <FeedbackReportModal visible={visible} onCancel={handleCancel} />
    </FeedbackContext.Provider>
  );
};

export const useFeedback = (): FeedbackContextValue => {
  const ctx = useContext(FeedbackContext);
  if (!ctx) {
    // Fallback so consumers don't crash when the provider isn't mounted (e.g. web build).
    return {
      openFeedback: async () => {
        /* no-op */
      },
    };
  }
  return ctx;
};
