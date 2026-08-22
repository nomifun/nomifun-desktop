/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import type { CanvasPoint, CanvasViewport } from './types';

export const MIN_CANVAS_ZOOM = 0.05;
export const MAX_CANVAS_ZOOM = 5;

export function clampCanvasZoom(zoom: number): number {
  if (!Number.isFinite(zoom)) return 1;
  return Math.min(MAX_CANVAS_ZOOM, Math.max(MIN_CANVAS_ZOOM, zoom));
}

export function normalizeCanvasViewport(viewport: CanvasViewport): CanvasViewport {
  return {
    x: Number.isFinite(viewport.x) ? viewport.x : 0,
    y: Number.isFinite(viewport.y) ? viewport.y : 0,
    zoom: clampCanvasZoom(viewport.zoom),
  };
}

export function canvasToClient(point: CanvasPoint, viewport: CanvasViewport): CanvasPoint {
  return {
    x: point.x * viewport.zoom + viewport.x,
    y: point.y * viewport.zoom + viewport.y,
  };
}

export function clientToCanvas(point: CanvasPoint, viewport: CanvasViewport): CanvasPoint {
  const normalized = normalizeCanvasViewport(viewport);
  return {
    x: (point.x - normalized.x) / normalized.zoom,
    y: (point.y - normalized.y) / normalized.zoom,
  };
}

/**
 * Change zoom while keeping the canvas coordinate below `clientAnchor`
 * stationary. The returned zoom is always clamped to 5%-500%.
 */
export function zoomViewportAtPoint(
  viewport: CanvasViewport,
  requestedZoom: number,
  clientAnchor: CanvasPoint
): CanvasViewport {
  const current = normalizeCanvasViewport(viewport);
  const nextZoom = clampCanvasZoom(requestedZoom);
  const worldAnchor = clientToCanvas(clientAnchor, current);

  return {
    x: clientAnchor.x - worldAnchor.x * nextZoom,
    y: clientAnchor.y - worldAnchor.y * nextZoom,
    zoom: nextZoom,
  };
}

export function scaleViewportAtPoint(
  viewport: CanvasViewport,
  factor: number,
  clientAnchor: CanvasPoint
): CanvasViewport {
  const safeFactor = Number.isFinite(factor) && factor > 0 ? factor : 1;
  return zoomViewportAtPoint(viewport, viewport.zoom * safeFactor, clientAnchor);
}

/** Panning deltas are expressed in client pixels, independent of zoom. */
export function panViewport(viewport: CanvasViewport, clientDelta: CanvasPoint): CanvasViewport {
  const current = normalizeCanvasViewport(viewport);
  return {
    ...current,
    x: current.x + (Number.isFinite(clientDelta.x) ? clientDelta.x : 0),
    y: current.y + (Number.isFinite(clientDelta.y) ? clientDelta.y : 0),
  };
}
