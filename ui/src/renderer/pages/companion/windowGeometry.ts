/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

export interface GeomRect {
  x: number;
  y: number;
  width: number;
  height: number;
}

export interface GeomSize {
  width: number;
  height: number;
}

const clamp = (v: number, lo: number, hi: number) => Math.min(Math.max(v, lo), Math.max(lo, hi));

const pickMostOverlapping = (rect: GeomRect, monitors: GeomRect[]): GeomRect | null => {
  let best: GeomRect | null = null;
  let bestArea = 0;
  for (const m of monitors) {
    const w = Math.min(rect.x + rect.width, m.x + m.width) - Math.max(rect.x, m.x);
    const h = Math.min(rect.y + rect.height, m.y + m.height) - Math.max(rect.y, m.y);
    const area = Math.max(0, w) * Math.max(0, h);
    if (area > bestArea) {
      bestArea = area;
      best = m;
    }
  }
  return best ?? monitors[0] ?? null;
};

/**
 * Placement for an in-place companion-window resize: the bottom edge stays put and
 * the window grows/shrinks around its horizontal center, then the result is
 * clamped into the monitor the old rect overlaps most — a taller window must
 * never sink below the screen (if the window exceeds the monitor itself, the
 * top edge pins to the monitor's top). Used only at actual size-change moments, so it
 * never disturbs a user's deliberate half-off-screen placement during normal
 * position restores. All values in physical px.
 */
export function placeResizedWindow(oldRect: GeomRect, newSize: GeomSize, monitors: GeomRect[]): { x: number; y: number } {
  let x = oldRect.x + Math.round((oldRect.width - newSize.width) / 2);
  let y = oldRect.y + (oldRect.height - newSize.height);
  const monitor = pickMostOverlapping(oldRect, monitors);
  if (monitor) {
    x = clamp(x, monitor.x, monitor.x + monitor.width - newSize.width);
    y = clamp(y, monitor.y, monitor.y + monitor.height - newSize.height);
  }
  return { x, y };
}
