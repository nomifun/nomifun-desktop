/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { configService } from '@/common/config/configService';
import type { ICssTheme } from '@/common/config/storage';
import { DEFAULT_THEME_ID } from '@renderer/pages/settings/DisplaySettings/presets';
import { processCustomCss } from '@renderer/utils/theme/customCssProcessor';
import { broadcastCustomCssSync } from '@renderer/utils/theme/themeBroadcast';
import {
  ensureThemeControlContract,
  removeThemeControlContract,
  THEME_CONTROL_CONTRACT_STYLE_ID,
} from '@renderer/utils/theme/themeControlContract';
import { computeCssSyncDecision, resolveCssByActiveTheme } from '@renderer/utils/theme/themeCssSync';
import React, { useCallback, useEffect, useRef, useState } from 'react';
import { useLocation } from 'react-router-dom';

const CUSTOM_CSS_STYLE_ID = 'user-defined-custom-css';

/**
 * Owns the authenticated application's CSS theme lifecycle without rendering UI.
 *
 * It lives above routed product layouts so switching between the normal
 * workbench and a focused surface cannot remove the user's active theme.
 */
const AppThemeRuntime: React.FC = () => {
  const location = useLocation();
  const [customCss, setCustomCss] = useState<string>('');
  const lastCssRef = useRef('');
  const lastUiCssUpdateAtRef = useRef(0);

  const loadAndHealCustomCss = useCallback(async () => {
    try {
      const [savedCssRaw, savedActiveThemeId, savedThemes] = await Promise.all([
        configService.get('customCss'),
        configService.get('css.activeThemeId'),
        configService.get('css.themes'),
      ]);

      // Fall back to the system default theme when none is selected, so fresh users apply it from first paint.
      const activeThemeId = savedActiveThemeId || DEFAULT_THEME_ID;

      const decision = computeCssSyncDecision({
        savedCss: savedCssRaw || '',
        activeThemeId,
        savedThemes: (savedThemes || []) as ICssTheme[],
        currentUiCss: customCss,
        lastUiCssUpdateAt: lastUiCssUpdateAtRef.current,
      });

      if (decision.shouldSkipApply) {
        return;
      }

      let effectiveCss = decision.effectiveCss;

      if (!effectiveCss && activeThemeId && activeThemeId !== DEFAULT_THEME_ID) {
        const defaultCss = resolveCssByActiveTheme(DEFAULT_THEME_ID, (savedThemes || []) as ICssTheme[]);
        effectiveCss = defaultCss;
        await Promise.all([
          configService.set('css.activeThemeId', DEFAULT_THEME_ID),
          configService.set('customCss', effectiveCss),
        ]).catch((error) => {
          console.warn('Failed to persist theme fallback:', error);
        });
      } else if (decision.shouldHealStorage) {
        await configService.set('customCss', effectiveCss).catch((error) => {
          console.warn('Failed to heal custom CSS from active theme:', error);
        });
      }

      setCustomCss(effectiveCss);
      if (lastCssRef.current !== effectiveCss) {
        lastCssRef.current = effectiveCss;
        window.dispatchEvent(new CustomEvent('custom-css-updated', { detail: { customCss: effectiveCss } }));
      }
    } catch (error) {
      console.error('Failed to load or heal custom CSS:', error);
    }
  }, [customCss]);

  useEffect(() => {
    void loadAndHealCustomCss();

    const handleCssUpdate = (event: CustomEvent) => {
      if (event.detail?.customCss !== undefined) {
        const css = event.detail.customCss || '';
        lastCssRef.current = css;
        lastUiCssUpdateAtRef.current = Date.now();
        setCustomCss(css);
      }
    };
    const handleStorageChange = (event: StorageEvent) => {
      if (event.key && (event.key.includes('customCss') || event.key.includes('css.activeThemeId'))) {
        void loadAndHealCustomCss();
      }
    };

    window.addEventListener('custom-css-updated', handleCssUpdate as EventListener);
    window.addEventListener('storage', handleStorageChange);

    return () => {
      window.removeEventListener('custom-css-updated', handleCssUpdate as EventListener);
      window.removeEventListener('storage', handleStorageChange);
    };
  }, [loadAndHealCustomCss]);

  // Some settings surfaces do not mount the theme editor, so route changes are
  // also a reconciliation point for persisted CSS.
  useEffect(() => {
    void loadAndHealCustomCss();
  }, [location.pathname, location.search, location.hash, loadAndHealCustomCss]);

  useEffect(() => {
    broadcastCustomCssSync(customCss);

    if (!customCss) {
      document.getElementById(CUSTOM_CSS_STYLE_ID)?.remove();
      ensureThemeControlContract();
      return;
    }

    const wrappedCss = processCustomCss(customCss);

    const ensureStyleAtEnd = () => {
      let styleEl = document.getElementById(CUSTOM_CSS_STYLE_ID) as HTMLStyleElement | null;
      const controlStyle = document.getElementById(THEME_CONTROL_CONTRACT_STYLE_ID);

      if (
        styleEl &&
        styleEl.textContent === wrappedCss &&
        styleEl.nextElementSibling === controlStyle &&
        controlStyle === document.head.lastElementChild
      ) {
        return;
      }

      styleEl?.remove();
      controlStyle?.remove();
      styleEl = document.createElement('style');
      styleEl.id = CUSTOM_CSS_STYLE_ID;
      styleEl.type = 'text/css';
      styleEl.textContent = wrappedCss;
      document.head.appendChild(styleEl);
      ensureThemeControlContract();
    };

    ensureStyleAtEnd();

    const observer = new MutationObserver((mutations) => {
      const hasNewStyle = mutations.some((mutation) =>
        Array.from(mutation.addedNodes).some((node) => node.nodeName === 'STYLE' || node.nodeName === 'LINK')
      );

      if (hasNewStyle) {
        const element = document.getElementById(CUSTOM_CSS_STYLE_ID);
        const controlStyle = document.getElementById(THEME_CONTROL_CONTRACT_STYLE_ID);
        if (element && (element.nextElementSibling !== controlStyle || controlStyle !== document.head.lastElementChild)) {
          ensureStyleAtEnd();
        }
      }
    });

    observer.observe(document.head, { childList: true });

    return () => {
      observer.disconnect();
      document.getElementById(CUSTOM_CSS_STYLE_ID)?.remove();
      removeThemeControlContract();
    };
  }, [customCss]);

  return null;
};

export default AppThemeRuntime;
