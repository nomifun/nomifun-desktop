/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { describe, expect, test } from 'bun:test';
import {
  OUTPUT_LIMIT_PRESETS,
  displayValueFromOutputLimit,
  normalizeOutputLimit,
  outputLimitFromDisplayValue,
} from './OutputLimitInput';

describe('output limit presets and unit conversion', () => {
  test('offers common model output ceilings from 1k through 128k', () => {
    expect(OUTPUT_LIMIT_PRESETS).toEqual([
      1_024,
      2_048,
      4_096,
      8_192,
      16_384,
      32_768,
      65_536,
      131_072,
    ]);
  });

  test('converts token, decimal-k, and decimal-M input to the persisted integer token count', () => {
    expect(outputLimitFromDisplayValue(8_192, 'tokens')).toBe(8_192);
    expect(outputLimitFromDisplayValue(8.192, 'k')).toBe(8_192);
    expect(outputLimitFromDisplayValue(0.008192, 'm')).toBe(8_192);
  });

  test('keeps an existing exact token count stable while changing display units', () => {
    expect(displayValueFromOutputLimit(8_192, 'tokens')).toBe(8_192);
    expect(displayValueFromOutputLimit(8_192, 'k')).toBe(8.192);
    expect(displayValueFromOutputLimit(8_192, 'm')).toBe(0.008192);
  });

  test('treats blank or invalid values as the provider default', () => {
    expect(normalizeOutputLimit(undefined)).toBeUndefined();
    expect(normalizeOutputLimit(0)).toBeUndefined();
    expect(normalizeOutputLimit(0.5)).toBeUndefined();
    expect(outputLimitFromDisplayValue(Number.NaN, 'tokens')).toBeUndefined();
    expect(outputLimitFromDisplayValue(-1, 'k')).toBeUndefined();
    expect(outputLimitFromDisplayValue(4_295, 'm')).toBeUndefined();
  });
});
