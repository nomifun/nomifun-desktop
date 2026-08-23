/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import type {
  CreativeCanvasConnection,
  CreativeCanvasNode,
  CreativeSize,
  CreativeViewport,
} from '../../domain';

export const DEFAULT_CANVAS_VIEWPORT_OVERSCAN_PX = 720;

export interface CanvasViewportCullingRect {
  x: number;
  y: number;
  width: number;
  height: number;
}

export interface CanvasViewportCullingInput {
  nodes: readonly CreativeCanvasNode[];
  connections: readonly CreativeCanvasConnection[];
  viewport: CreativeViewport;
  containerSize: CreativeSize | null;
  selectedNodeIds?: readonly string[];
  selectedEdgeIds?: readonly string[];
  requiredNodeIds?: readonly string[];
  overscanPx?: number;
}

export interface CanvasViewportCullingResult {
  /** True when measurement was unavailable and every layer item is retained. */
  renderAll: boolean;
  /** The measured world-space clip rect, or null when the result is full render. */
  worldRect: CanvasViewportCullingRect | null;
  nodeIds: ReadonlySet<string>;
  connectionIds: ReadonlySet<string>;
}

const finitePositive = (value: number): boolean => Number.isFinite(value) && value > 0;

const finiteNonNegative = (value: number): boolean =>
  Number.isFinite(value) && value >= 0;

const normaliseZero = (value: number): number => (Object.is(value, -0) ? 0 : value);

const nodeRect = (node: CreativeCanvasNode): CanvasViewportCullingRect | null => {
  const { x, y } = node.position;
  const { width, height } = node.size;
  if (
    !Number.isFinite(x) ||
    !Number.isFinite(y) ||
    !finiteNonNegative(width) ||
    !finiteNonNegative(height)
  ) {
    return null;
  }
  return { x, y, width, height };
};

const rectsIntersectOrTouch = (
  left: CanvasViewportCullingRect,
  right: CanvasViewportCullingRect
): boolean =>
  left.x <= right.x + right.width &&
  left.x + left.width >= right.x &&
  left.y <= right.y + right.height &&
  left.y + left.height >= right.y;

const unionRects = (
  left: CanvasViewportCullingRect,
  right: CanvasViewportCullingRect
): CanvasViewportCullingRect => {
  const x = Math.min(left.x, right.x);
  const y = Math.min(left.y, right.y);
  const rightEdge = Math.max(left.x + left.width, right.x + right.width);
  const bottomEdge = Math.max(left.y + left.height, right.y + right.height);
  return {
    x,
    y,
    width: rightEdge - x,
    height: bottomEdge - y,
  };
};

const allItems = (
  nodes: readonly CreativeCanvasNode[],
  connections: readonly CreativeCanvasConnection[]
): CanvasViewportCullingResult => ({
  renderAll: true,
  worldRect: null,
  nodeIds: new Set(nodes.map((node) => node.id)),
  connectionIds: new Set(connections.map((connection) => connection.id)),
});

const groupClosure = (
  nodes: readonly CreativeCanvasNode[],
  nodeById: ReadonlyMap<string, CreativeCanvasNode>,
  seedIds: ReadonlySet<string>
): Set<string> => {
  const retained = new Set(seedIds);
  const memberIdsByGroup = new Map<string, string[]>();
  for (const node of nodes) {
    if (!node.groupId) continue;
    const members = memberIdsByGroup.get(node.groupId);
    if (members) members.push(node.id);
    else memberIdsByGroup.set(node.groupId, [node.id]);
  }
  const pending = [...retained];

  for (let index = 0; index < pending.length; index += 1) {
    const node = nodeById.get(pending[index]);
    if (!node) continue;

    if (node.groupId && !retained.has(node.groupId)) {
      retained.add(node.groupId);
      pending.push(node.groupId);
    }
    if (node.type !== 'group') continue;
    for (const memberId of memberIdsByGroup.get(node.id) ?? []) {
      if (retained.has(memberId)) continue;
      retained.add(memberId);
      pending.push(memberId);
    }
  }

  return retained;
};

const addSelectedEdgeEndpoints = (
  connections: readonly CreativeCanvasConnection[],
  selectedEdgeIds: ReadonlySet<string>,
  retainedNodeIds: Set<string>
): void => {
  for (const connection of connections) {
    if (!selectedEdgeIds.has(connection.id)) continue;
    retainedNodeIds.add(connection.sourceNodeId);
    retainedNodeIds.add(connection.targetNodeId);
  }
};

/**
 * Compute the conservative set of world items needed for the current viewport.
 *
 * The clip rect is expanded in screen pixels, so the visual safety margin stays
 * stable while zooming. Forced items and group closure intentionally over-render
 * rather than risk losing a live interaction or a required group container.
 */
export function computeCanvasViewportCulling(
  input: CanvasViewportCullingInput
): CanvasViewportCullingResult {
  const { nodes, connections, viewport, containerSize } = input;
  if (
    !containerSize ||
    !finitePositive(containerSize.width) ||
    !finitePositive(containerSize.height) ||
    !Number.isFinite(viewport.x) ||
    !Number.isFinite(viewport.y) ||
    !finitePositive(viewport.zoom)
  ) {
    return allItems(nodes, connections);
  }

  const overscanPx =
    input.overscanPx === undefined || !finiteNonNegative(input.overscanPx)
      ? DEFAULT_CANVAS_VIEWPORT_OVERSCAN_PX
      : input.overscanPx;
  const overscanWorld = overscanPx / viewport.zoom;
  const worldRect: CanvasViewportCullingRect = {
    x: normaliseZero(-viewport.x / viewport.zoom - overscanWorld),
    y: normaliseZero(-viewport.y / viewport.zoom - overscanWorld),
    width: containerSize.width / viewport.zoom + overscanWorld * 2,
    height: containerSize.height / viewport.zoom + overscanWorld * 2,
  };

  const retainedNodeIds = new Set<string>();
  for (const node of nodes) {
    const rect = nodeRect(node);
    if (!rect || rectsIntersectOrTouch(rect, worldRect)) {
      retainedNodeIds.add(node.id);
    }
  }

  for (const nodeId of input.selectedNodeIds ?? []) retainedNodeIds.add(nodeId);
  for (const nodeId of input.requiredNodeIds ?? []) retainedNodeIds.add(nodeId);

  const selectedEdgeIds = new Set(input.selectedEdgeIds ?? []);
  addSelectedEdgeEndpoints(connections, selectedEdgeIds, retainedNodeIds);
  const nodeById = new Map(nodes.map((node) => [node.id, node]));
  const nodesWithGroups = groupClosure(nodes, nodeById, retainedNodeIds);
  const connectionIds = new Set<string>();
  for (const connection of connections) {
    if (selectedEdgeIds.has(connection.id)) {
      connectionIds.add(connection.id);
      continue;
    }
    if (
      nodesWithGroups.has(connection.sourceNodeId) ||
      nodesWithGroups.has(connection.targetNodeId)
    ) {
      connectionIds.add(connection.id);
      continue;
    }

    const source = nodeById.get(connection.sourceNodeId);
    const target = nodeById.get(connection.targetNodeId);
    const sourceRect = source ? nodeRect(source) : null;
    const targetRect = target ? nodeRect(target) : null;
    if (!sourceRect || !targetRect) {
      // Malformed geometry is rare, but retaining its edge is the conservative
      // choice because a reliable visibility decision is impossible.
      connectionIds.add(connection.id);
      continue;
    }
    if (rectsIntersectOrTouch(unionRects(sourceRect, targetRect), worldRect)) {
      connectionIds.add(connection.id);
    }
  }

  return {
    renderAll: false,
    worldRect,
    nodeIds: nodesWithGroups,
    connectionIds,
  };
}
