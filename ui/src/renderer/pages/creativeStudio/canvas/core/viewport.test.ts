/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { describe, expect, test } from 'bun:test';
import {
  canvasToClient,
  clampCanvasZoom,
  clientToCanvas,
  MAX_CANVAS_ZOOM,
  MIN_CANVAS_ZOOM,
  normalizeCanvasViewport,
  panViewport,
  scaleViewportAtPoint,
  zoomViewportAtPoint,
} from './viewport';

describe('Creative Studio viewport', () => {
  test('clamps every entry point to the 5%-500% product range', () => {
    expect(clampCanvasZoom(0)).toBe(MIN_CANVAS_ZOOM);
    expect(clampCanvasZoom(0.049)).toBe(MIN_CANVAS_ZOOM);
    expect(clampCanvasZoom(5.001)).toBe(MAX_CANVAS_ZOOM);
    expect(clampCanvasZoom(Number.NaN)).toBe(1);
    expect(normalizeCanvasViewport({ x: Number.NaN, y: Number.POSITIVE_INFINITY, zoom: 99 })).toEqual({
      x: 0,
      y: 0,
      zoom: 5,
    });
  });

  test('keeps the world point beneath the pointer stationary while zooming', () => {
    const viewport = { x: 140, y: -60, zoom: 1.25 };
    const pointer = { x: 920, y: 480 };
    const worldBefore = clientToCanvas(pointer, viewport);
    const next = zoomViewportAtPoint(viewport, 3.4, pointer);

    expect(clientToCanvas(pointer, next).x).toBeCloseTo(worldBefore.x, 10);
    expect(clientToCanvas(pointer, next).y).toBeCloseTo(worldBefore.y, 10);
    expect(canvasToClient(worldBefore, next).x).toBeCloseTo(pointer.x, 10);
    expect(canvasToClient(worldBefore, next).y).toBeCloseTo(pointer.y, 10);
  });

  test('scales at the pointer and ignores unsafe factors', () => {
    const viewport = { x: 10, y: 20, zoom: 2 };
    expect(scaleViewportAtPoint(viewport, 1.5, { x: 100, y: 80 }).zoom).toBe(3);
    expect(scaleViewportAtPoint(viewport, -2, { x: 100, y: 80 }).zoom).toBe(2);
  });

  test('pans in client pixels independently of zoom', () => {
    expect(panViewport({ x: 20, y: 30, zoom: 4 }, { x: -8, y: 12 })).toEqual({
      x: 12,
      y: 42,
      zoom: 4,
    });
  });
});
