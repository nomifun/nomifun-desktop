/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { configService } from '@/common/config/configService';
import type { ICssTheme } from '@/common/config/storage';
import { uuid } from '@/common/utils';
import { getCssThemeDisplayName } from '@renderer/pages/settings/DisplaySettings/presets';
import { computeCssSyncDecision, normalizeCssTheme, resolveCssThemeState } from '@renderer/utils/theme/themeCssSync';
import { Message } from '@arco-design/web-react';
import { useCallback, useEffect, useState } from 'react';
import { useTranslation } from 'react-i18next';

/**
 * useCssTheme — the single source of truth for the CSS skin/preset axis.
 *
 * ThemeContext owns light/dark mode. This hook manages the preset catalog and
 * user themes, sharing selection/default/background resolution with
 * AppThemeRuntime. Theme writes are batched and serialized before notifying
 * the runtime to inject and broadcast the CSS.
 */

const dispatchCustomCssUpdated = (css: string) => {
  window.dispatchEvent(new CustomEvent('custom-css-updated', { detail: { customCss: css } }));
};

/**
 * Cross-instance notification: the user theme LIST changed (add/edit/delete).
 * Every mounted `useCssTheme` reloads so the footer popover and the settings
 * grid stay consistent without a remount.
 */
const CSS_THEMES_CHANGED_EVENT = 'nomifun:css-themes-changed';
const notifyCssThemesChanged = (): void => {
  window.dispatchEvent(new CustomEvent(CSS_THEMES_CHANGED_EVENT));
};

// Module-level serial queue — all preset applies (footer control or settings
// page) are processed in strict order against the same config keys, eliminating
// client-side async interleaving. Each apply uses one batched settings RPC.
let applyQueue: Promise<void> = Promise.resolve();

/**
 * Queued, best-effort apply of a resolved CSS string + active theme id. On
 * IPC/storage failure it recovers UI state unconditionally from what is actually
 * persisted (re-dispatching `custom-css-updated` so every listener re-syncs).
 */
const applyCssThemeRaw = (css: string, themeId: string): Promise<void> => {
  const task = async () => {
    try {
      await configService.setBatch({ customCss: css, 'css.activeThemeId': themeId });
      dispatchCustomCssUpdated(css);
    } catch (error) {
      console.error('Failed to apply theme (IPC/Storage). Recovering from storage:', error);
      try {
        await configService.reload();
        const { activeTheme } = resolveCssThemeState(
          configService.get('css.activeThemeId') || '',
          configService.get('css.themes') || []
        );
        dispatchCustomCssUpdated(activeTheme.css);
      } catch (syncError) {
        console.error('Theme recovery sync failed:', syncError);
      }
      throw error;
    }
  };
  applyQueue = applyQueue.then(task, task);
  return applyQueue;
};

export interface UseCssThemeResult {
  /** Merged theme list: built-in presets plus user-defined themes. */
  themes: ICssTheme[];
  /** Currently active theme id (reactively tracks `css.activeThemeId`). */
  activeThemeId: string;
  /** Apply a theme by object; shows a toast. Used by every switcher surface. */
  selectTheme: (theme: ICssTheme) => Promise<void>;
  /** Add a new user theme, or update an existing one (editing a preset stores a copy). */
  saveUserTheme: (data: Omit<ICssTheme, 'id' | 'is_preset'>, editing: ICssTheme | null) => Promise<void>;
  /** Delete a user theme; immediately applies the default if it was active. */
  deleteUserTheme: (themeId: string) => Promise<void>;
}

export const useCssTheme = (): UseCssThemeResult => {
  const { t } = useTranslation();
  const [{ themes, activeThemeId }, setThemeState] = useState(() => {
    const { themes, activeTheme } = resolveCssThemeState(
      configService.get('css.activeThemeId') || '',
      configService.get('css.themes') || []
    );
    return { themes, activeThemeId: activeTheme.id };
  });

  const reload = useCallback(async () => {
    try {
      await configService.whenReady();
      const decision = computeCssSyncDecision({
        savedCss: configService.get('customCss') || '',
        activeThemeId: configService.get('css.activeThemeId') || '',
        savedThemes: configService.get('css.themes') || [],
      });
      if (configService.isInitialized() && Object.keys(decision.configUpdates).length) {
        await configService.setBatch(decision.configUpdates);
      }
      // A newer choice may have completed while reconciliation was persisting.
      const current = resolveCssThemeState(
        configService.get('css.activeThemeId') || '',
        configService.get('css.themes') || []
      );
      if (current.activeTheme.id !== decision.activeThemeId || current.activeTheme.css !== decision.effectiveCss) return;
      setThemeState({ themes: current.themes, activeThemeId: current.activeTheme.id });
      if (decision.configUpdates.customCss !== undefined) dispatchCustomCssUpdated(decision.effectiveCss);
    } catch (error) {
      console.error('Failed to load CSS themes:', error);
    }
  }, []);

  useEffect(() => {
    void reload();

    // Keep activeThemeId reactive: any surface that applies a preset dispatches
    // `custom-css-updated` after persisting, so re-read the id from config.
    const onCssUpdated = () => {
      const { themes, activeTheme } = resolveCssThemeState(
        configService.get('css.activeThemeId') || '',
        configService.get('css.themes') || []
      );
      setThemeState({ themes, activeThemeId: activeTheme.id });
    };
    // The user theme LIST changed elsewhere → reload the merged set.
    const onThemesChanged = () => void reload();

    window.addEventListener('custom-css-updated', onCssUpdated as EventListener);
    window.addEventListener(CSS_THEMES_CHANGED_EVENT, onThemesChanged as EventListener);
    return () => {
      window.removeEventListener('custom-css-updated', onCssUpdated as EventListener);
      window.removeEventListener(CSS_THEMES_CHANGED_EVENT, onThemesChanged as EventListener);
    };
  }, [reload]);

  const selectTheme = useCallback(
    async (theme: ICssTheme) => {
      try {
        const { activeTheme } = resolveCssThemeState(
          theme.id,
          configService.get('css.themes') || []
        );
        await applyCssThemeRaw(activeTheme.css, activeTheme.id);
        Message.success(t('settings.cssTheme.applied', { name: getCssThemeDisplayName(activeTheme, t) }));
      } catch {
        Message.error(t('settings.cssTheme.applyFailed'));
      }
    },
    [t]
  );

  const saveUserTheme = useCallback(
    async (data: Omit<ICssTheme, 'id' | 'is_preset'>, editing: ICssTheme | null) => {
      const normalized = normalizeCssTheme(data);
      const { userThemes } = resolveCssThemeState('', configService.get('css.themes') || []);
      let updated: ICssTheme[];
      if (editing && !editing.is_preset) {
        updated = userThemes.map((th) => (th.id === editing.id ? { ...th, ...normalized } : th));
      } else {
        // New theme (also covers "edit a preset" → saved as a fresh user copy).
        const created: ICssTheme = { id: uuid(), ...normalized, is_preset: false };
        updated = [...userThemes, created];
      }
      await configService.set('css.themes', updated);
      notifyCssThemesChanged();
    },
    []
  );

  const deleteUserTheme = useCallback(
    async (themeId: string) => {
      const savedThemes = configService.get('css.themes') || [];
      const decision = computeCssSyncDecision({
        savedCss: configService.get('customCss') || '',
        activeThemeId: configService.get('css.activeThemeId') || '',
        savedThemes: savedThemes.filter((th) => th.id !== themeId),
      });
      await configService.setBatch({ ...decision.configUpdates, 'css.themes': decision.themes.filter((th) => !th.is_preset) });
      dispatchCustomCssUpdated(decision.effectiveCss);
      notifyCssThemesChanged();
    },
    []
  );

  return { themes, activeThemeId, selectTheme, saveUserTheme, deleteUserTheme };
};
