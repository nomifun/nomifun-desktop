/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { describe, expect, it } from 'bun:test';
import { CHARACTERS, CUSTOM_CHARACTER_ID, DEFAULT_DESK, getDeskSpecFor } from './index';
import type { CustomFigureMeta } from './types';

const customFigure: CustomFigureMeta = {
  aspect: 0.9, headBox: { x: 0.3, y: 0, w: 0.3, h: 0.3 }, sizeTier: 'm', sizePx: 256,
};

describe('getDeskSpecFor', () => {
  it('uses the compact built-in desktop footprint', () => {
    expect(DEFAULT_DESK).toEqual({ windowWidth: 200, windowHeight: 180, figureHeight: 120 });
  });

  it('uses DEFAULT_DESK for unavailable looks and custom looks without metadata', () => {
    for (const id of ['ink', 'no-such-character', CUSTOM_CHARACTER_ID, null, undefined]) {
      expect(getDeskSpecFor(id)).toBe(DEFAULT_DESK);
    }
  });

  it('keeps every roster character on the default desk', () => {
    for (const { id } of CHARACTERS) {
      expect(getDeskSpecFor(id)).toBe(DEFAULT_DESK);
      expect(getDeskSpecFor(id, customFigure)).toBe(DEFAULT_DESK);
    }
  });

  it('uses the configured custom-figure dimensions through the desktop entry point', () => {
    expect(getDeskSpecFor(CUSTOM_CHARACTER_ID, customFigure)).toEqual({
      windowWidth: 255, windowHeight: 316, figureHeight: 256,
    });
  });

  it('uses the teal palette for bolt', () => {
    const bolt = CHARACTERS.find((c) => c.id === 'bolt');
    expect(bolt?.palette).toEqual(['#bfeee0', '#37e0ff']);
  });
});
