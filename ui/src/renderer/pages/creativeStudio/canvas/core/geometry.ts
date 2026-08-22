/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import type {
  CanvasGraphNode,
  CanvasPoint,
  CanvasSelectionRect,
} from './types';

export const DEFAULT_GROUP_PADDING = 24;

export function normalizeSelectionRect(
  anchor: CanvasPoint,
  current: CanvasPoint
): CanvasSelectionRect {
  return {
    x: Math.min(anchor.x, current.x),
    y: Math.min(anchor.y, current.y),
    width: Math.abs(current.x - anchor.x),
    height: Math.abs(current.y - anchor.y),
  };
}

export function graphNodeRect(node: CanvasGraphNode): CanvasSelectionRect {
  return {
    x: node.position.x,
    y: node.position.y,
    width: node.size.width,
    height: node.size.height,
  };
}

export function rectsIntersect(a: CanvasSelectionRect, b: CanvasSelectionRect): boolean {
  return (
    a.x < b.x + b.width &&
    a.x + a.width > b.x &&
    a.y < b.y + b.height &&
    a.y + a.height > b.y
  );
}

export function graphNodeIntersectsRect(
  node: CanvasGraphNode,
  rect: CanvasSelectionRect
): boolean {
  return rectsIntersect(graphNodeRect(node), rect);
}

export function boundsForGraphNodes(nodes: readonly CanvasGraphNode[]): CanvasSelectionRect | null {
  if (nodes.length === 0) return null;

  let left = Number.POSITIVE_INFINITY;
  let top = Number.POSITIVE_INFINITY;
  let right = Number.NEGATIVE_INFINITY;
  let bottom = Number.NEGATIVE_INFINITY;

  for (const node of nodes) {
    left = Math.min(left, node.position.x);
    top = Math.min(top, node.position.y);
    right = Math.max(right, node.position.x + node.size.width);
    bottom = Math.max(bottom, node.position.y + node.size.height);
  }

  return {
    x: left,
    y: top,
    width: right - left,
    height: bottom - top,
  };
}
