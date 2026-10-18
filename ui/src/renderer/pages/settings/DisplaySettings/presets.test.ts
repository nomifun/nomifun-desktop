/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { describe, expect, test } from 'bun:test';
import enSettings from '@/renderer/services/i18n/locales/en-US/settings.json';
import zhSettings from '@/renderer/services/i18n/locales/zh-CN/settings.json';
import { DEFAULT_THEME_ID, getCssThemeDisplayName, PRESET_THEMES, PRESET_THEME_NAME_KEYS } from './presets';

const getLocaleString = (settings: unknown, key: string): string => {
  const segments = key.replace(/^settings\./, '').split('.');
  let cursor: unknown = settings;
  for (const segment of segments) {
    if (!cursor || typeof cursor !== 'object' || !(segment in cursor)) {
      throw new Error(`Missing locale key: ${key}`);
    }
    cursor = (cursor as Record<string, unknown>)[segment];
  }
  if (typeof cursor !== 'string') {
    throw new Error(`Locale key is not a string: ${key}`);
  }
  return cursor;
};

describe('display theme presets', () => {
  test('ships Classic as the default followed by Frosted Glass', () => {
    expect(DEFAULT_THEME_ID).toBe('codex-neutral');
    expect(PRESET_THEMES.map((theme) => theme.id)).toEqual([DEFAULT_THEME_ID, 'frosted-glass']);
    expect(PRESET_THEMES[0]?.name).toBe('经典');
  });

  test('defines a localized display name key for every built-in preset', () => {
    expect(Object.keys(PRESET_THEME_NAME_KEYS).sort()).toEqual(PRESET_THEMES.map((theme) => theme.id).sort());

    for (const key of Object.values(PRESET_THEME_NAME_KEYS)) {
      expect(getLocaleString(zhSettings, key).trim()).toBeTruthy();
      expect(getLocaleString(enSettings, key).trim()).toBeTruthy();
    }
  });

  test('resolves built-in preset display names from the active locale only', () => {
    const zh = (key: string) => getLocaleString(zhSettings, key);
    const en = (key: string) => getLocaleString(enSettings, key);

    expect(PRESET_THEMES.map((theme) => getCssThemeDisplayName(theme, zh))).toEqual(['经典', '冰晶幻境']);
    expect(PRESET_THEMES.map((theme) => getCssThemeDisplayName(theme, en))).toEqual(['Classic', 'Frosted Glass']);
  });

  test('keeps user theme names literal instead of translating them', () => {
    const customTheme = { ...PRESET_THEMES[0]!, id: 'custom-user-theme', name: '我的 Theme', is_preset: false };
    const customThemeWithPresetId = { ...PRESET_THEMES[0]!, name: 'My Classic Copy', is_preset: false };

    expect(getCssThemeDisplayName(customTheme, (key) => getLocaleString(enSettings, key))).toBe('我的 Theme');
    expect(getCssThemeDisplayName(customThemeWithPresetId, (key) => getLocaleString(enSettings, key))).toBe(
      'My Classic Copy'
    );
  });
});
