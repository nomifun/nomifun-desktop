/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import type { CharacterDeskSpec, CustomFigureMeta } from './types';

export const FIGURE_HEIGHTS = { s: 360, m: 430, l: 500 } as const;
export const MAX_WINDOW_WIDTH = 480;
/** Never narrower than the classic chibi window — chat bar and bubble must fit. */
export const MIN_WINDOW_WIDTH = 240;
const SIDE_MARGIN = 14; // px each side
// IDLE window vertical chrome AROUND the figure: just the hover quick-input bar
// reserve below (~48px) + a little headroom above for the hop/breath animation.
// The bubble's headroom is NOT reserved here anymore — that left a big always-
// transparent strip above the figure ("透明背景占住桌面空间"). The bubble grows the
// window on demand (enterChatSize) and shrinks back (exitChatSize) instead.
const CHROME_HEIGHT = 64;

/** Pure metadata → desk computation for DIY custom figures. */
export function customDeskSpec(meta: CustomFigureMeta): CharacterDeskSpec {
  // Defend against degenerate metadata (corrupt config, division by zero).
  const aspect = Number.isFinite(meta.aspect) && meta.aspect > 0 ? meta.aspect : 1;
  let figureHeight: number = FIGURE_HEIGHTS[meta.sizeTier] ?? FIGURE_HEIGHTS.m;
  let windowWidth = Math.ceil(figureHeight * aspect) + SIDE_MARGIN * 2;
  if (windowWidth > MAX_WINDOW_WIDTH) {
    windowWidth = MAX_WINDOW_WIDTH;
    figureHeight = Math.floor((MAX_WINDOW_WIDTH - SIDE_MARGIN * 2) / aspect);
  }
  windowWidth = Math.max(windowWidth, MIN_WINDOW_WIDTH);
  return { windowWidth, windowHeight: figureHeight + CHROME_HEIGHT, figureHeight };
}
