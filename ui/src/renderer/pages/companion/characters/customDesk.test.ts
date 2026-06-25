/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { describe, expect, it } from 'vitest';
import { customDeskSpec, FIGURE_HEIGHTS, MAX_WINDOW_WIDTH, MIN_WINDOW_WIDTH } from './customDesk';

describe('customDeskSpec', () => {
  it('computes window from aspect and tier', () => {
    // aspect 0.9444, m → figure 430, width ceil(430*0.9444)=407, window 407+14*2=435? 见实现：margin 常量
    const d = customDeskSpec({ aspect: 0.9444, headBox: { x: 0.3, y: 0, w: 0.3, h: 0.3 }, sizeTier: 'm' });
    expect(d.figureHeight).toBe(430);
    expect(d.windowHeight).toBe(494); // figure 430 + CHROME_HEIGHT 64 (bubble grows on demand)
    expect(d.windowWidth).toBe(Math.min(MAX_WINDOW_WIDTH, Math.ceil(430 * 0.9444) + 28));
  });
  it('clamps extreme wide images and shrinks the figure to fit', () => {
    const d = customDeskSpec({ aspect: 2.0, headBox: { x: 0.3, y: 0, w: 0.3, h: 0.3 }, sizeTier: 'l' });
    expect(d.windowWidth).toBe(480);
    expect(d.figureHeight).toBe(Math.floor((480 - 28) / 2.0));
  });
  it('never narrower than the classic window (skinny images keep chat usable)', () => {
    const d = customDeskSpec({ aspect: 0.3, headBox: { x: 0.3, y: 0, w: 0.3, h: 0.3 }, sizeTier: 's' });
    // ceil(360*0.3)+28 = 136 → clamped up; figure keeps its tier height
    expect(d.windowWidth).toBe(MIN_WINDOW_WIDTH);
    expect(d.figureHeight).toBe(360);
  });
  it('survives degenerate aspect values', () => {
    const d = customDeskSpec({ aspect: Number.NaN, headBox: { x: 0.3, y: 0, w: 0.3, h: 0.3 }, sizeTier: 'm' });
    expect(Number.isFinite(d.windowWidth)).toBe(true);
    expect(d.figureHeight).toBe(430);
  });
  it('size tiers map to fixed heights', () => {
    expect(FIGURE_HEIGHTS).toEqual({ s: 360, m: 430, l: 500 });
  });
});
