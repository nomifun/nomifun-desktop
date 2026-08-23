/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { useEffect, useState } from 'react';

import { configService } from '@/common/config/configService';
import {
  getLanguagePreference,
  type LanguagePreference,
} from '@/renderer/services/i18n';

/**
 * Keep language-mode controls in sync with the shared client preference cache.
 *
 * `currentLanguage` is included so the hook refreshes after i18next applies the
 * authoritative value during startup or a cross-window language update.
 */
export function useLanguagePreference(currentLanguage?: string): LanguagePreference {
  const [preference, setPreference] = useState<LanguagePreference>(() => getLanguagePreference());

  useEffect(() => {
    const sync = () => setPreference(getLanguagePreference());
    const unsubscribeLanguage = configService.subscribe('language', sync);
    const unsubscribeMode = configService.subscribe('languageMode', sync);

    sync();
    return () => {
      unsubscribeLanguage();
      unsubscribeMode();
    };
  }, [currentLanguage]);

  return preference;
}
