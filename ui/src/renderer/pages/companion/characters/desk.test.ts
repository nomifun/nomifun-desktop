/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { describe, expect, it } from 'vitest';
import { CHARACTERS, DEFAULT_DESK, getDeskSpec } from './index';

describe('getDeskSpec', () => {
  it('falls back to DEFAULT_DESK for unknown / missing ids', () => {
    expect(getDeskSpec('no-such-character')).toBe(DEFAULT_DESK);
    expect(getDeskSpec(null)).toBe(DEFAULT_DESK);
    expect(getDeskSpec(undefined)).toBe(DEFAULT_DESK);
  });

  it('keeps every roster character on the default desk', () => {
    for (const id of ['mochi', 'ink', 'bolt']) {
      expect(getDeskSpec(id)).toBe(DEFAULT_DESK);
    }
  });

  it('uses the teal palette for bolt', () => {
    const bolt = CHARACTERS.find((c) => c.id === 'bolt');
    expect(bolt?.palette).toEqual(['#bfeee0', '#37e0ff']);
  });
});
