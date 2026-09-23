/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import {
  DEFAULT_THINKING_DISPLAY_PREFERENCES,
  normalizeThinkingContentDisplayLength,
  normalizeThinkingSummaryDisplayLength,
  type ThinkingDisplayPreferences,
} from '@/common/config/thinkingDisplay';
import { useMemo } from 'react';

import { useConfig } from './useConfig';

/** Reactive, normalized view of the install-wide reasoning presentation preferences. */
export const useThinkingDisplayPreferences = (): ThinkingDisplayPreferences => {
  const [visible] = useConfig('chat.thinking.visible');
  const [contentLength] = useConfig('chat.thinking.contentLength');
  const [summaryLength] = useConfig('chat.thinking.summaryLength');

  return useMemo(
    () => ({
      visible: typeof visible === 'boolean' ? visible : DEFAULT_THINKING_DISPLAY_PREFERENCES.visible,
      contentLength: normalizeThinkingContentDisplayLength(contentLength),
      summaryLength: normalizeThinkingSummaryDisplayLength(summaryLength),
    }),
    [contentLength, summaryLength, visible]
  );
};
