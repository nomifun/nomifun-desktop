/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import type {
  CreativeCanvasConnection,
  CreativeCanvasNode,
  CreativePoint,
  CreativeSize,
  CreativeViewport,
} from '../../domain/schema';

export type CanvasHandleSide = 'left' | 'right' | 'top' | 'bottom';

/** Node-local handle geometry supplied by the node/interaction controller. */
export interface CanvasNodeHandleGeometry extends CreativePoint {
  side: CanvasHandleSide;
}

export type CanvasHandleGeometryByNode = Readonly<
  Record<string, Readonly<Record<string, CanvasNodeHandleGeometry>>>
>;

export interface CanvasResolvedEndpoint {
  point: CreativePoint;
  side: CanvasHandleSide;
}

export interface CanvasBezierGeometry {
  source: CanvasResolvedEndpoint;
  target: CanvasResolvedEndpoint;
  sourceControl: CreativePoint;
  targetControl: CreativePoint;
  path: string;
}

export interface CanvasWorldBounds extends CreativePoint, CreativeSize {}

export interface CanvasMiniMapProjection {
  width: number;
  height: number;
  padding: number;
  worldBounds: CanvasWorldBounds;
  scale: number;
  offset: CreativePoint;
}

const finiteOr = (value: number, fallback: number) => (Number.isFinite(value) ? value : fallback);
const positiveOr = (value: number, fallback: number) => Math.max(0.001, finiteOr(value, fallback));
const clamp = (value: number, min: number, max: number) => Math.min(max, Math.max(min, value));

const fallbackEndpoint = (node: CreativeCanvasNode, role: 'source' | 'target'): CanvasResolvedEndpoint => ({
  point: {
    x: node.position.x + (role === 'source' ? node.size.width : 0),
    y: node.position.y + node.size.height / 2,
  },
  side: role === 'source' ? 'right' : 'left',
});

export function resolveCanvasConnectionEndpoint(
  node: CreativeCanvasNode,
  handleId: string | null,
  role: 'source' | 'target',
  handles?: CanvasHandleGeometryByNode
): CanvasResolvedEndpoint {
  if (!handleId) return fallbackEndpoint(node, role);
  const local = handles?.[node.id]?.[handleId];
  if (!local) return fallbackEndpoint(node, role);
  return {
    point: {
      x: node.position.x + finiteOr(local.x, 0),
      y: node.position.y + finiteOr(local.y, 0),
    },
    side: local.side,
  };
}

const moveAlongSide = (point: CreativePoint, side: CanvasHandleSide, distance: number): CreativePoint => {
  switch (side) {
    case 'left':
      return { x: point.x - distance, y: point.y };
    case 'right':
      return { x: point.x + distance, y: point.y };
    case 'top':
      return { x: point.x, y: point.y - distance };
    case 'bottom':
      return { x: point.x, y: point.y + distance };
  }
};

const formatCoordinate = (value: number) => Number(value.toFixed(3));

export function buildCanvasConnectionBezier(
  connection: CreativeCanvasConnection,
  sourceNode: CreativeCanvasNode,
  targetNode: CreativeCanvasNode,
  handles?: CanvasHandleGeometryByNode
): CanvasBezierGeometry {
  const source = resolveCanvasConnectionEndpoint(sourceNode, connection.sourceHandle, 'source', handles);
  const target = resolveCanvasConnectionEndpoint(targetNode, connection.targetHandle, 'target', handles);
  const deltaX = target.point.x - source.point.x;
  const deltaY = target.point.y - source.point.y;
  const distance = Math.hypot(deltaX, deltaY);
  const controlDistance = clamp(distance * 0.42, 42, 240);
  const sourceControl = moveAlongSide(source.point, source.side, controlDistance);
  const targetControl = moveAlongSide(target.point, target.side, controlDistance);
  const path = `M ${formatCoordinate(source.point.x)} ${formatCoordinate(source.point.y)} C ${formatCoordinate(sourceControl.x)} ${formatCoordinate(sourceControl.y)}, ${formatCoordinate(targetControl.x)} ${formatCoordinate(targetControl.y)}, ${formatCoordinate(target.point.x)} ${formatCoordinate(target.point.y)}`;

  return { source, target, sourceControl, targetControl, path };
}

export function visibleWorldRect(viewport: CreativeViewport, viewportSize: CreativeSize): CanvasWorldBounds {
  const zoom = positiveOr(viewport.zoom, 1);
  return {
    x: -finiteOr(viewport.x, 0) / zoom,
    y: -finiteOr(viewport.y, 0) / zoom,
    width: Math.max(1, finiteOr(viewportSize.width, 1) / zoom),
    height: Math.max(1, finiteOr(viewportSize.height, 1) / zoom),
  };
}

export function createCanvasMiniMapProjection(
  nodes: readonly CreativeCanvasNode[],
  viewport: CreativeViewport,
  viewportSize: CreativeSize,
  options: { width?: number; height?: number; padding?: number; worldPadding?: number } = {}
): CanvasMiniMapProjection {
  const width = positiveOr(options.width ?? 240, 240);
  const height = positiveOr(options.height ?? 160, 160);
  const padding = clamp(finiteOr(options.padding ?? 10, 10), 0, Math.min(width, height) / 3);
  const worldPadding = Math.max(0, finiteOr(options.worldPadding ?? 120, 120));
  const viewportRect = visibleWorldRect(viewport, viewportSize);
  let minX = viewportRect.x;
  let minY = viewportRect.y;
  let maxX = viewportRect.x + viewportRect.width;
  let maxY = viewportRect.y + viewportRect.height;

  for (const node of nodes) {
    minX = Math.min(minX, finiteOr(node.position.x, 0));
    minY = Math.min(minY, finiteOr(node.position.y, 0));
    maxX = Math.max(maxX, finiteOr(node.position.x, 0) + positiveOr(node.size.width, 1));
    maxY = Math.max(maxY, finiteOr(node.position.y, 0) + positiveOr(node.size.height, 1));
  }

  minX -= worldPadding;
  minY -= worldPadding;
  maxX += worldPadding;
  maxY += worldPadding;
  const worldBounds: CanvasWorldBounds = {
    x: minX,
    y: minY,
    width: Math.max(1, maxX - minX),
    height: Math.max(1, maxY - minY),
  };
  const availableWidth = Math.max(1, width - padding * 2);
  const availableHeight = Math.max(1, height - padding * 2);
  const scale = Math.min(availableWidth / worldBounds.width, availableHeight / worldBounds.height);
  const projectedWidth = worldBounds.width * scale;
  const projectedHeight = worldBounds.height * scale;
  const offset = {
    x: (width - projectedWidth) / 2,
    y: (height - projectedHeight) / 2,
  };

  return { width, height, padding, worldBounds, scale, offset };
}

export function worldPointToMiniMap(point: CreativePoint, projection: CanvasMiniMapProjection): CreativePoint {
  return {
    x: (point.x - projection.worldBounds.x) * projection.scale + projection.offset.x,
    y: (point.y - projection.worldBounds.y) * projection.scale + projection.offset.y,
  };
}

export function miniMapPointToWorld(point: CreativePoint, projection: CanvasMiniMapProjection): CreativePoint {
  return {
    x: (point.x - projection.offset.x) / projection.scale + projection.worldBounds.x,
    y: (point.y - projection.offset.y) / projection.scale + projection.worldBounds.y,
  };
}

export function worldRectToMiniMap(rect: CanvasWorldBounds, projection: CanvasMiniMapProjection): CanvasWorldBounds {
  const origin = worldPointToMiniMap(rect, projection);
  return {
    ...origin,
    width: Math.max(2, rect.width * projection.scale),
    height: Math.max(2, rect.height * projection.scale),
  };
}

export function centerCanvasViewportAt(
  worldCenter: CreativePoint,
  viewport: CreativeViewport,
  viewportSize: CreativeSize
): CreativeViewport {
  const zoom = positiveOr(viewport.zoom, 1);
  return {
    x: finiteOr(viewportSize.width, 0) / 2 - worldCenter.x * zoom,
    y: finiteOr(viewportSize.height, 0) / 2 - worldCenter.y * zoom,
    zoom,
  };
}
