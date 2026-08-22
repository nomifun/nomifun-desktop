/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import {
  canvasCommands,
  normalizeCanvasViewport,
  type CanvasCommand,
  type CanvasGraphNode,
  type CanvasPoint,
  type CanvasSize,
  type CanvasViewport,
} from '../core';

export type CanvasResizeCorner =
  | 'top-left'
  | 'top-right'
  | 'bottom-left'
  | 'bottom-right';

export interface CanvasResizeConstraints {
  minSize?: CanvasSize;
  keepAspectRatio?: boolean;
  /** Defaults to the starting node ratio when ratio locking is enabled. */
  aspectRatio?: number;
}

export interface CanvasResizeGesture {
  kind: 'resize';
  pointerId: number;
  corner: CanvasResizeCorner;
  startClient: CanvasPoint;
  startNode: CanvasGraphNode;
  zoom: number;
  minSize: CanvasSize;
  keepAspectRatio: boolean;
  aspectRatio: number;
  mergeKey: string;
}

export type StartCanvasResizeResult =
  | { ok: true; gesture: CanvasResizeGesture }
  | { ok: false; reason: 'locked' | 'invalid-pointer' };

export interface UpdateCanvasResizeResult {
  matched: boolean;
  command: CanvasCommand | null;
  node: CanvasGraphNode | null;
}

export const SOURCE_CANVAS_MIN_NODE_SIZE: CanvasSize = {
  width: 220,
  height: 160,
};

const finitePoint = (point: CanvasPoint): boolean =>
  Number.isFinite(point.x) && Number.isFinite(point.y);

const positiveFinite = (value: number, fallback: number): number =>
  Number.isFinite(value) && value > 0 ? value : fallback;

export function startCanvasResize(
  node: CanvasGraphNode,
  pointerId: number,
  client: CanvasPoint,
  corner: CanvasResizeCorner,
  viewport: CanvasViewport,
  options: CanvasResizeConstraints & { mergeKey?: string } = {}
): StartCanvasResizeResult {
  if (node.locked) return { ok: false, reason: 'locked' };
  if (!Number.isInteger(pointerId) || pointerId < 0 || !finitePoint(client)) {
    return { ok: false, reason: 'invalid-pointer' };
  }

  const minSize = {
    width: positiveFinite(options.minSize?.width ?? SOURCE_CANVAS_MIN_NODE_SIZE.width, 1),
    height: positiveFinite(options.minSize?.height ?? SOURCE_CANVAS_MIN_NODE_SIZE.height, 1),
  };
  const startWidth = positiveFinite(node.size.width, minSize.width);
  const startHeight = positiveFinite(node.size.height, minSize.height);
  const aspectRatio = positiveFinite(
    options.aspectRatio ?? startWidth / startHeight,
    1
  );

  return {
    ok: true,
    gesture: {
      kind: 'resize',
      pointerId,
      corner,
      startClient: { ...client },
      startNode: structuredClone(node),
      zoom: normalizeCanvasViewport(viewport).zoom,
      minSize,
      keepAspectRatio: options.keepAspectRatio ?? false,
      aspectRatio,
      mergeKey: options.mergeKey ?? `resize:${node.id}`,
    },
  };
}

/**
 * Project a pointer move into one absolute canonical node replacement. Left and
 * top handles preserve the opposite edge; deltas are divided by canvas zoom.
 */
export function updateCanvasResize(
  gesture: CanvasResizeGesture,
  pointerId: number,
  client: CanvasPoint,
  at?: number
): UpdateCanvasResizeResult {
  if (gesture.pointerId !== pointerId || !finitePoint(client)) {
    return { matched: false, command: null, node: null };
  }

  const dx = (client.x - gesture.startClient.x) / gesture.zoom;
  const dy = (client.y - gesture.startClient.y) / gesture.zoom;
  const start = gesture.startNode;
  const fromLeft = gesture.corner.includes('left');
  const fromTop = gesture.corner.includes('top');
  const right = start.position.x + start.size.width;
  const bottom = start.position.y + start.size.height;
  let width = Math.max(
    gesture.minSize.width,
    start.size.width + (fromLeft ? -dx : dx)
  );
  let height = Math.max(
    gesture.minSize.height,
    start.size.height + (fromTop ? -dy : dy)
  );

  if (gesture.keepAspectRatio) {
    if (Math.abs(dx) >= Math.abs(dy)) height = width / gesture.aspectRatio;
    else width = height * gesture.aspectRatio;

    if (height < gesture.minSize.height) {
      height = gesture.minSize.height;
      width = height * gesture.aspectRatio;
    }
    if (width < gesture.minSize.width) {
      width = gesture.minSize.width;
      height = width / gesture.aspectRatio;
    }
  }

  const node = {
    ...structuredClone(start),
    position: {
      x: fromLeft ? right - width : start.position.x,
      y: fromTop ? bottom - height : start.position.y,
    },
    size: { width, height },
  } as CanvasGraphNode;

  return {
    matched: true,
    node,
    command: canvasCommands.updateNode(node, {
      at,
      mergeKey: gesture.mergeKey,
    }),
  };
}

export function finishCanvasResize(
  gesture: CanvasResizeGesture,
  pointerId: number
): boolean {
  return gesture.pointerId === pointerId;
}
