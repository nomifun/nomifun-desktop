/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { describe, expect, it } from 'bun:test';
import {
  clampDesktopFigureHeight,
  customDeskSpec,
  FIGURE_HEIGHTS,
  MAX_WINDOW_WIDTH,
  MIN_WINDOW_WIDTH,
  SIZE_MIN,
  SIZE_MAX,
} from './customDesk';

describe('customDeskSpec', () => {
  it('computes window from aspect and tier (no sizePx override)', () => {
    // m → figure 168; a slightly-landscape aspect keeps width between MIN and MAX (no clamp).
    const d = customDeskSpec({ aspect: 1.2, headBox: { x: 0.3, y: 0, w: 0.3, h: 0.3 }, sizeTier: 'm' });
    expect(d.figureHeight).toBe(168);
    expect(d.windowHeight).toBe(228); // figure 168 + CHROME_HEIGHT 60
    expect(d.windowWidth).toBe(Math.ceil(168 * 1.2) + 24); // 226, within [MIN, MAX]
  });

  it('uses sizePx as the figure height when set, overriding the tier', () => {
    // sizePx 256 wins over tier 'm' (168). aspect 0.9 → width within [MIN, MAX].
    const d = customDeskSpec({ aspect: 0.9, headBox: { x: 0.3, y: 0, w: 0.3, h: 0.3 }, sizeTier: 'm', sizePx: 256 });
    expect(d.figureHeight).toBe(256);
    expect(d.windowHeight).toBe(316); // 256 + 60
    expect(d.windowWidth).toBe(Math.ceil(256 * 0.9) + 24); // 255
  });

  it('clamps sizePx to [SIZE_MIN, SIZE_MAX]', () => {
    const big = customDeskSpec({ aspect: 0.5, headBox: { x: 0.3, y: 0, w: 0.3, h: 0.3 }, sizeTier: 'm', sizePx: 1000 });
    expect(big.figureHeight).toBe(SIZE_MAX); // 320
    const small = customDeskSpec({ aspect: 1, headBox: { x: 0.3, y: 0, w: 0.3, h: 0.3 }, sizeTier: 'l', sizePx: 50 });
    expect(small.figureHeight).toBe(SIZE_MIN); // 112; desktop stage explicitly stays full-body
  });

  it('ignores a degenerate sizePx and falls back to the tier', () => {
    const nan = customDeskSpec({ aspect: 1, headBox: { x: 0.3, y: 0, w: 0.3, h: 0.3 }, sizeTier: 'l', sizePx: Number.NaN });
    expect(nan.figureHeight).toBe(FIGURE_HEIGHTS.l); // 224
    const zero = customDeskSpec({ aspect: 1, headBox: { x: 0.3, y: 0, w: 0.3, h: 0.3 }, sizeTier: 's', sizePx: 0 });
    expect(zero.figureHeight).toBe(FIGURE_HEIGHTS.s); // 120
  });

  it('clamps extreme wide images to MAX_WINDOW_WIDTH and shrinks the figure to fit', () => {
    // sizePx 320 at aspect 2.0 → raw width ceil(640)+24 = 664 > 320 → clamp.
    const d = customDeskSpec({ aspect: 2.0, headBox: { x: 0.3, y: 0, w: 0.3, h: 0.3 }, sizeTier: 'l', sizePx: 320 });
    expect(d.windowWidth).toBe(MAX_WINDOW_WIDTH); // 320
    expect(d.figureHeight).toBe(Math.floor((MAX_WINDOW_WIDTH - 24) / 2.0)); // 148
  });

  it('never narrower than the classic window (skinny images keep chat usable)', () => {
    const d = customDeskSpec({ aspect: 0.3, headBox: { x: 0.3, y: 0, w: 0.3, h: 0.3 }, sizeTier: 's' });
    // ceil(120*0.3)+24 = 60 → clamped up; figure keeps its tier height
    expect(d.windowWidth).toBe(MIN_WINDOW_WIDTH);
    expect(d.figureHeight).toBe(120);
  });

  it('survives degenerate aspect values', () => {
    const d = customDeskSpec({ aspect: Number.NaN, headBox: { x: 0.3, y: 0, w: 0.3, h: 0.3 }, sizeTier: 'm' });
    expect(Number.isFinite(d.windowWidth)).toBe(true);
    expect(d.figureHeight).toBe(168);
  });

  it('size tiers map to fixed heights and slider bounds are sane', () => {
    expect(FIGURE_HEIGHTS).toEqual({ s: 120, m: 168, l: 224 });
    expect(SIZE_MIN).toBe(112);
    expect(SIZE_MAX).toBe(320);
    expect(MAX_WINDOW_WIDTH).toBe(320);
    expect(MIN_WINDOW_WIDTH).toBe(200);
  });

  it('normalizes persisted overrides to the compact slider range', () => {
    expect(clampDesktopFigureHeight(400)).toBe(320);
    expect(clampDesktopFigureHeight(80)).toBe(112);
    expect(clampDesktopFigureHeight(168)).toBe(168);
  });
});
