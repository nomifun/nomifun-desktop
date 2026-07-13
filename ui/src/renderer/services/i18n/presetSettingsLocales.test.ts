/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { describe, expect, test } from 'bun:test';
import enSettings from './locales/en-US/settings.json';
import zhSettings from './locales/zh-CN/settings.json';

const REQUIRED_PRESETS_HUB_KEYS = ['title', 'subtitle', 'railTitle'] as const;

const assertPresetSettingsLocale = (settings: Record<string, unknown>) => {
  expect(typeof settings.presetSkills).toBe('string');
  expect((settings.presetSkills as string).trim()).toBeTruthy();

  expect(settings.presetsHub).toBeDefined();
  expect(typeof settings.presetsHub).toBe('object');
  for (const key of REQUIRED_PRESETS_HUB_KEYS) {
    const value = (settings.presetsHub as Record<string, unknown> | undefined)?.[key];
    expect(typeof value).toBe('string');
    expect((value as string).trim()).toBeTruthy();
  }
};

describe('preset settings locale coverage', () => {
  test('en-US keeps editor skill label separate from the preset/skill hub strings', () => {
    assertPresetSettingsLocale(enSettings);
  });

  test('zh-CN keeps editor skill label separate from the preset/skill hub strings', () => {
    assertPresetSettingsLocale(zhSettings);
  });

  test('zh-CN keeps Presets and Skills as independent destinations', () => {
    expect(zhSettings.presetsHub.title).toBe('设定');
    expect(zhSettings.presetsHub.railTitle).toBe('设定');
    expect(zhSettings.skillsHub.railTitle).toBe('技能');
  });

  test('localizes the inline preset-tag Enter hint', () => {
    expect(enSettings.presetTagAddHint).toBe('Press Enter to finish adding');
    expect(zhSettings.presetTagAddHint).toBe('按回车完成添加');
  });
});
