/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import type { ICssTheme } from '@/common/config/storage.ts';

// Theme CSS loaded as raw strings via Vite ?raw imports
import codexNeutralCss from './presets/codex-neutral.css?raw';
import frostedGlassCss from './presets/frosted-glass.css?raw';

/**
 * 系统默认主题 ID / System default theme ID
 * 无显式选择时（空 activeThemeId）回退并应用此主题；也是主题缺失时的兜底。
 * Applied when no theme is explicitly selected (empty activeThemeId); also the fallback when a theme is missing.
 */
export const DEFAULT_THEME_ID = 'codex-neutral';

export const PRESET_THEME_NAME_KEYS = {
  'codex-neutral': 'settings.cssTheme.presets.codexNeutral',
  'frosted-glass': 'settings.cssTheme.presets.frostedGlass',
} as const;

export type PresetThemeNameKey = (typeof PRESET_THEME_NAME_KEYS)[keyof typeof PRESET_THEME_NAME_KEYS];

const getPresetThemeNameKey = (themeId: string): PresetThemeNameKey | undefined => {
  if (!Object.prototype.hasOwnProperty.call(PRESET_THEME_NAME_KEYS, themeId)) return undefined;
  return PRESET_THEME_NAME_KEYS[themeId as keyof typeof PRESET_THEME_NAME_KEYS];
};

export const getCssThemeDisplayName = (
  theme: Pick<ICssTheme, 'id' | 'name' | 'is_preset'>,
  translate: (key: PresetThemeNameKey) => string
): string => {
  const nameKey = theme.is_preset ? getPresetThemeNameKey(theme.id) : undefined;
  return nameKey ? translate(nameKey) : theme.name;
};

/**
 * 预设 CSS 主题列表 / Preset CSS themes list
 * 这些主题是内置的，用户可以直接选择使用 / These themes are built-in and can be directly used by users
 * 新增主题请遵循 docs/contributing/styling.md 的主题契约 / New themes must follow the contract in docs/contributing/styling.md
 * 数组顺序 = 卡片展示顺序：系统默认主题「经典」置首。
 * Array order = card display order: the system default "Classic" first.
 */
export const PRESET_THEMES: ICssTheme[] = [
  {
    id: DEFAULT_THEME_ID,
    name: '经典',
    is_preset: true,
    css: codexNeutralCss,
  },
  {
    id: 'frosted-glass',
    name: '冰晶幻境',
    is_preset: true,
    css: frostedGlassCss,
  },
];
