/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { ipcBridge } from '@/common';
import { configService } from '@/common/config/configService';
import type { ICssTheme } from '@/common/config/storage';
import { uuid } from '@/common/utils';
import {
  BACKGROUND_BLOCK_START,
  backgroundCssBlockNeedsUpgrade,
  injectBackgroundCssBlock,
} from '@renderer/pages/settings/DisplaySettings/backgroundUtils';
import { DEFAULT_THEME_ID, PRESET_THEMES } from '@renderer/pages/settings/DisplaySettings/presets';
import { resolveExtensionAssetUrl } from '@renderer/utils/platform';
import { resolveCssByActiveTheme, setExtensionThemesCache } from '@renderer/utils/theme/themeCssSync';
import { Message } from '@arco-design/web-react';
import { useCallback, useEffect, useState } from 'react';
import { useTranslation } from 'react-i18next';

/**
 * useCssTheme — the single source of truth for the CSS skin/preset axis.
 *
 * The light/dark axis lives in ThemeContext; the CSS preset axis (rhythm-dark,
 * neon-rainbow, …) historically lived only inside `CssThemeSettings`. This hook
 * extracts that axis so EVERY surface (the Display settings page AND the sider
 * footer quick-switcher) shares one apply path: resolve preset CSS → write
 * `customCss` + `css.activeThemeId` → dispatch `custom-css-updated` (which
 * `Layout` listens for to inject + cross-window broadcast). Writes go through a
 * single module-level serial queue so concurrent applies from different surfaces
 * can never interleave against the same config keys.
 */

/** A theme that carries a `cover` image gets a background-image CSS block appended. */
export const ensureBackgroundCss = <T extends { cover?: string; css: string }>(theme: T): T => {
  if (
    theme.cover &&
    (!theme.css || !theme.css.includes(BACKGROUND_BLOCK_START) || backgroundCssBlockNeedsUpgrade(theme.css))
  ) {
    return { ...theme, css: injectBackgroundCssBlock(theme.css || '', theme.cover) };
  }
  return theme;
};

export const normalizeUserThemes = (themes: ICssTheme[]): { normalized: ICssTheme[]; updated: boolean } => {
  let updated = false;
  const normalized = themes.map((theme) => {
    const next = ensureBackgroundCss(theme);
    if (next !== theme) updated = true;
    return next;
  });
  return { normalized, updated };
};

const dispatchCustomCssUpdated = (css: string) => {
  window.dispatchEvent(new CustomEvent('custom-css-updated', { detail: { customCss: css } }));
};

/**
 * Cross-instance notification: the user theme LIST changed (add/edit/delete).
 * Every mounted `useCssTheme` reloads so the footer popover and the settings
 * grid stay consistent without a remount.
 */
const CSS_THEMES_CHANGED_EVENT = 'nomifun:css-themes-changed';
export const notifyCssThemesChanged = (): void => {
  window.dispatchEvent(new CustomEvent(CSS_THEMES_CHANGED_EVENT));
};

// Module-level serial queue — all preset applies (footer control or settings
// page) are processed in strict order against the same config keys, eliminating
// client-side async interleaving. True atomicity would need a single batched RPC.
let applyQueue: Promise<void> = Promise.resolve();

/**
 * Queued, best-effort apply of a resolved CSS string + active theme id. On
 * IPC/storage failure it recovers UI state unconditionally from what is actually
 * persisted (re-dispatching `custom-css-updated` so every listener re-syncs).
 */
export const applyCssThemeRaw = (css: string, themeId: string): Promise<void> => {
  const task = async () => {
    try {
      await Promise.all([configService.set('customCss', css), configService.set('css.activeThemeId', themeId)]);
      dispatchCustomCssUpdated(css);
    } catch (error) {
      console.error('Failed to apply theme (IPC/Storage). Recovering from storage:', error);
      try {
        const realCss = configService.get('customCss') || '';
        dispatchCustomCssUpdated(realCss);
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
  /** Merged, deduped theme list: presets + extension-contributed + user-defined. */
  themes: ICssTheme[];
  /** Currently active theme id (reactively tracks `css.activeThemeId`). */
  activeThemeId: string;
  /** Apply a theme by object; shows a toast. Used by every switcher surface. */
  selectTheme: (theme: ICssTheme) => Promise<void>;
  /** Reload the merged list + heal `customCss`/`css.activeThemeId` drift. */
  reload: () => Promise<void>;
  /** Optimistically replace the local list (settings page add/edit/delete). */
  setThemes: React.Dispatch<React.SetStateAction<ICssTheme[]>>;
  /** Add a new user theme, or update an existing one (editing a preset stores a copy). */
  saveUserTheme: (data: Omit<ICssTheme, 'id' | 'created_at' | 'updated_at' | 'is_preset'>, editing: ICssTheme | null) => Promise<void>;
  /** Delete a user theme; clears the active CSS if the deleted theme was active. */
  deleteUserTheme: (themeId: string) => Promise<void>;
}

export const useCssTheme = (): UseCssThemeResult => {
  const { t } = useTranslation();
  const [themes, setThemes] = useState<ICssTheme[]>([]);
  const [activeThemeId, setActiveThemeId] = useState<string>('');

  const reload = useCallback(async () => {
    try {
      const savedThemes = (configService.get('css.themes') || []) as ICssTheme[];
      const { normalized, updated } = normalizeUserThemes(savedThemes);
      const activeId = configService.get('css.activeThemeId');

      if (updated) {
        await configService.set(
          'css.themes',
          normalized.filter((th) => !th.is_preset)
        );
      }

      const normalizedPresets = PRESET_THEMES.map((th) => ensureBackgroundCss(th));

      // Extension-contributed themes (best-effort; absent in WebUI mode).
      let extensionThemes: ICssTheme[] = [];
      try {
        const loaded = await ipcBridge.extensions.getThemes.invoke();
        extensionThemes = loaded.map((th) => ({ ...th, cover: resolveExtensionAssetUrl(th.cover) }));
        setExtensionThemesCache(extensionThemes);
      } catch {
        // Extensions unavailable — keep presets + user themes only.
      }

      // Merge & dedupe by id (first occurrence wins): preset → extension → user.
      const seen = new Set<string>();
      const all: ICssTheme[] = [];
      for (const th of [...normalizedPresets, ...extensionThemes, ...normalized.filter((x) => !x.is_preset)]) {
        if (!th?.id || seen.has(th.id)) continue;
        seen.add(th.id);
        all.push(th);
      }

      const resolvedActiveId = activeId || DEFAULT_THEME_ID;
      let effectiveActiveId = resolvedActiveId;
      if (!all.find((th) => th.id === resolvedActiveId) && resolvedActiveId !== DEFAULT_THEME_ID) {
        // Active theme vanished (e.g. extension removed) → fall back + persist.
        effectiveActiveId = DEFAULT_THEME_ID;
        await configService.set('css.activeThemeId', effectiveActiveId);
      }

      setThemes(all);
      setActiveThemeId(effectiveActiveId);

      // Self-heal split-brain (activeThemeId ≠ customCss) from partial write failures.
      const expectedCss = resolveCssByActiveTheme(
        effectiveActiveId,
        normalized.filter((x) => !x.is_preset)
      );
      const savedCustomCss = configService.get('customCss') || '';
      if (savedCustomCss !== expectedCss) {
        await configService.set('customCss', expectedCss);
        dispatchCustomCssUpdated(expectedCss);
      }
    } catch (error) {
      console.error('Failed to load CSS themes:', error);
    }
  }, []);

  useEffect(() => {
    void reload();

    // Keep activeThemeId reactive: any surface that applies a preset dispatches
    // `custom-css-updated` after persisting, so re-read the id from config.
    const onCssUpdated = () => setActiveThemeId(configService.get('css.activeThemeId') || '');
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
        const css = resolveCssByActiveTheme(
          theme.id,
          themes.filter((x) => !x.is_preset)
        );
        await applyCssThemeRaw(css, theme.id);
        Message.success(t('settings.cssTheme.applied', { name: theme.name }));
      } catch {
        Message.error(t('settings.cssTheme.applyFailed'));
      }
    },
    [themes, t]
  );

  const saveUserTheme = useCallback(
    async (data: Omit<ICssTheme, 'id' | 'created_at' | 'updated_at' | 'is_preset'>, editing: ICssTheme | null) => {
      const now = Date.now();
      const normalized = ensureBackgroundCss(data);
      let updated: ICssTheme[];
      if (editing && !editing.is_preset) {
        updated = themes.map((th) => (th.id === editing.id ? { ...th, ...normalized, updated_at: now } : th));
      } else {
        // New theme (also covers "edit a preset" → saved as a fresh user copy).
        const created: ICssTheme = { id: uuid(), ...normalized, is_preset: false, created_at: now, updated_at: now };
        updated = [...themes, created];
      }
      await configService.set(
        'css.themes',
        updated.filter((th) => !th.is_preset)
      );
      setThemes(updated);
      notifyCssThemesChanged();
    },
    [themes]
  );

  const deleteUserTheme = useCallback(
    async (themeId: string) => {
      const updated = themes.filter((th) => th.id !== themeId);
      await configService.set(
        'css.themes',
        updated.filter((th) => !th.is_preset)
      );
      // Deleting the active theme clears the applied CSS (reload then falls back to default).
      if (activeThemeId === themeId) {
        await applyCssThemeRaw('', '');
      }
      setThemes(updated);
      notifyCssThemesChanged();
    },
    [themes, activeThemeId]
  );

  return { themes, activeThemeId, selectTheme, reload, setThemes, saveUserTheme, deleteUserTheme };
};
