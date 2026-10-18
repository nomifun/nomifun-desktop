import type { ConfigKeyMap } from '@/common/config/configKeys';
import type { ICssTheme } from '@/common/config/storage';
import {
  BACKGROUND_BLOCK_START,
  backgroundCssBlockNeedsUpgrade,
  injectBackgroundCssBlock,
} from '@renderer/pages/settings/DisplaySettings/backgroundUtils';
import { DEFAULT_THEME_ID, PRESET_THEMES } from '@renderer/pages/settings/DisplaySettings/presets';

const CSS_SYNC_RECENT_UPDATE_WINDOW_MS = 2000;

type CssThemeConfigUpdates = Partial<Pick<ConfigKeyMap, 'customCss' | 'css.activeThemeId' | 'css.themes'>>;

type ComputeCssSyncDecisionParams = {
  savedCss: string;
  activeThemeId: string;
  savedThemes: ICssTheme[];
  currentUiCss?: string;
  lastUiCssUpdateAt?: number;
  now?: number;
};

/** Apply the same background treatment when saving, selecting and restoring a theme. */
export const normalizeCssTheme = <T extends { cover?: string; css: string }>(theme: T): T => {
  if (
    theme.cover &&
    (!theme.css.includes(BACKGROUND_BLOCK_START) || backgroundCssBlockNeedsUpgrade(theme.css))
  ) {
    return { ...theme, css: injectBackgroundCssBlock(theme.css, theme.cover) };
  }
  return theme;
};

/** Built-ins come from the catalog; persisted entries may only supply user themes. */
export const resolveCssThemeState = (activeThemeId: string, savedThemes: ICssTheme[]) => {
  const themes = [...PRESET_THEMES];
  const seen = new Set(themes.map((theme) => theme.id));
  const userThemes: ICssTheme[] = [];
  for (const theme of savedThemes) {
    if (!theme?.id || theme.is_preset || seen.has(theme.id)) continue;
    seen.add(theme.id);
    userThemes.push(normalizeCssTheme(theme));
  }
  themes.push(...userThemes);

  const activeTheme = themes.find((theme) => theme.id === activeThemeId)
    ?? PRESET_THEMES.find((theme) => theme.id === DEFAULT_THEME_ID)!;
  return { themes, userThemes, activeTheme };
};

export const computeCssSyncDecision = ({
  savedCss,
  activeThemeId,
  savedThemes,
  currentUiCss = '',
  lastUiCssUpdateAt = 0,
  now = Date.now(),
}: ComputeCssSyncDecisionParams) => {
  const { themes, userThemes, activeTheme } = resolveCssThemeState(activeThemeId, savedThemes);
  const configUpdates: CssThemeConfigUpdates = {};
  if (activeThemeId !== activeTheme.id) configUpdates['css.activeThemeId'] = activeTheme.id;
  if (savedCss !== activeTheme.css) configUpdates.customCss = activeTheme.css;
  if (savedThemes.length !== userThemes.length || userThemes.some((theme, index) => theme !== savedThemes[index])) {
    configUpdates['css.themes'] = userThemes;
  }

  const recentUiUpdate = now - lastUiCssUpdateAt < CSS_SYNC_RECENT_UPDATE_WINDOW_MS;
  const shouldSkipApply = Object.keys(configUpdates).length === 0
    && recentUiUpdate && Boolean(currentUiCss) && savedCss !== currentUiCss;

  return {
    themes,
    activeThemeId: activeTheme.id,
    configUpdates,
    shouldSkipApply,
    effectiveCss: shouldSkipApply ? currentUiCss : activeTheme.css,
  };
};
