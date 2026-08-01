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

export const clamp = (v: number, lo: number, hi: number): number => Math.min(Math.max(v, lo), Math.max(lo, hi));

/** Intersection area of two rects (0 when disjoint). */
export const overlapArea = (a: GeomRect, b: GeomRect): number =>
  Math.max(0, Math.min(a.x + a.width, b.x + b.width) - Math.max(a.x, b.x)) *
  Math.max(0, Math.min(a.y + a.height, b.y + b.height) - Math.max(a.y, b.y));

/**
 * Pick the item whose bounds overlap `anchor` the most. Ties go to the earlier
 * item; when nothing overlaps the first item is returned; empty input → null.
 */
export function pickHost<T>(anchor: GeomRect, items: T[], boundsOf: (item: T) => GeomRect): T | null {
  if (items.length === 0) return null;
  return items.reduce((best, item) => (overlapArea(anchor, boundsOf(item)) > overlapArea(anchor, boundsOf(best)) ? item : best));
}

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
  const monitor = pickHost(oldRect, monitors, (m) => m);
  if (monitor) {
    x = clamp(x, monitor.x, monitor.x + monitor.width - newSize.width);
    y = clamp(y, monitor.y, monitor.y + monitor.height - newSize.height);
  }
  return { x, y };
}
