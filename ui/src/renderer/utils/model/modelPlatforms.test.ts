/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { describe, expect, test } from 'bun:test';
import { getPlatformByValue, getProviderLogo, isCustomOption, MODEL_PLATFORMS } from './modelPlatforms';

const preset = (value: string) => {
  const found = getPlatformByValue(value);
  if (!found) throw new Error(`Missing model platform preset: ${value}`);
  return found;
};

describe('model platform display presets', () => {
  test('uses unique preset ids even when multiple products share a runtime family', () => {
    expect(new Set(MODEL_PLATFORMS.map((item) => item.value)).size).toBe(MODEL_PLATFORMS.length);

    expect(preset('SiliconFlow-CN').platform).toBe('siliconflow');
    expect(preset('SiliconFlow').platform).toBe('siliconflow');
    expect(preset('Ark').platform).toBe('ark');
    expect(preset('Ark-Coding-Plan').platform).toBe('ark-coding-plan');
    expect(preset('Ark-Agent-Plan').platform).toBe('ark-agent-plan');
    expect(preset('StepFun').platform).toBe('stepfun');
    expect(preset('StepFun-Plan').platform).toBe('stepfun-plan');
  });

  test('contains display metadata only', () => {
    const allowedKeys = ['i18nKey', 'logo', 'name', 'platform', 'value'];
    for (const item of MODEL_PLATFORMS) {
      expect(Object.keys(item).sort().every((key) => allowedKeys.includes(key))).toBe(true);
      expect(item.name.trim().length).toBeGreaterThan(0);
      expect(item.value.trim().length).toBeGreaterThan(0);
      expect(String(item.platform).trim().length).toBeGreaterThan(0);
    }
  });

  test('resolves logos from stable family or exact display name without endpoint inference', () => {
    expect(getProviderLogo({ platform: 'stepfun' })).toBe(preset('StepFun').logo);
    expect(getProviderLogo({ name: 'OpenAI' })).toBe(preset('OpenAI').logo);
    expect(getProviderLogo({ name: 'openai' })).toBe(preset('OpenAI').logo);
    expect(getProviderLogo({ name: 'Unknown provider' })).toBeNull();
  });

  test('recognizes only the explicit custom preset', () => {
    expect(isCustomOption('custom')).toBe(true);
    expect(isCustomOption('new-api')).toBe(false);
    expect(isCustomOption('OpenAI')).toBe(false);
  });
});
