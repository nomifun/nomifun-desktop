/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { uuidv7 } from '@/common/utils/uuidv7';
import type { CreativeCanvasNodeDataByKind } from '../../domain';
import { boundsForGraphNodes, DEFAULT_GROUP_PADDING } from './geometry';
import type {
  CanvasClipboard,
  CanvasDocument,
  CanvasEdge,
  CanvasGraphNode,
  CanvasGroup,
  CanvasIdFactory,
  CanvasNode,
  CanvasNodeType,
  CanvasPoint,
  CanvasSize,
} from './types';

/** Durable canvas identities are canonical, lowercase, bare UUIDv7 values. */
export const createCanvasId: CanvasIdFactory = () => uuidv7();

export function cloneCanvasNode<T extends CanvasNode>(node: T): T {
  return structuredClone(node);
}

export function cloneCanvasEdge(edge: CanvasEdge): CanvasEdge {
  return structuredClone(edge);
}

export function cloneCanvasDocument(document: CanvasDocument): CanvasDocument {
  return {
    nodes: document.nodes.map(cloneCanvasNode),
    connections: document.connections.map(cloneCanvasEdge),
  };
}

export function cloneCanvasClipboard(clipboard: CanvasClipboard): CanvasClipboard {
  return cloneCanvasDocument(clipboard);
}

export function getCanvasGraphNodes(document: CanvasDocument): CanvasGraphNode[] {
  return [...document.nodes];
}

export function getCanvasGroups(document: CanvasDocument): CanvasGroup[] {
  return document.nodes.filter(isCanvasGroup);
}

export function findCanvasGraphNode(
  document: CanvasDocument,
  id: string
): CanvasGraphNode | undefined {
  return document.nodes.find((node) => node.id === id);
}

export function isCanvasGroup(node: CanvasGraphNode): node is CanvasGroup {
  return node.type === 'group';
}

export interface MakeCanvasNodeOptions<K extends CanvasNodeType> {
  id?: string;
  type: K;
  position: CanvasPoint;
  size: CanvasSize;
  data: CreativeCanvasNodeDataByKind[K];
  groupId?: string | null;
  zIndex?: number;
  locked?: boolean;
  idFactory?: CanvasIdFactory;
}

export function makeCanvasNode<K extends CanvasNodeType>(
  options: MakeCanvasNodeOptions<K>
): Extract<CanvasNode, { type: K }> {
  return {
    id: options.id ?? (options.idFactory ?? createCanvasId)(options.type === 'group' ? 'group' : 'node'),
    type: options.type,
    position: { ...options.position },
    size: { ...options.size },
    groupId: options.type === 'group' ? null : (options.groupId ?? null),
    zIndex: options.zIndex ?? 0,
    locked: options.locked ?? false,
    data: structuredClone(options.data),
  } as unknown as Extract<CanvasNode, { type: K }>;
}

export type GroupNodesResult =
  | { ok: true; document: CanvasDocument; group: CanvasGroup }
  | {
      ok: false;
      reason: 'requires_two_nodes' | 'nested_group_not_supported' | 'node_not_found';
    };

/**
 * Group at least two free ordinary nodes. Member positions stay absolute so
 * renderer, clipboard, and persistence all share one coordinate system.
 */
export function groupCanvasNodes(
  document: CanvasDocument,
  nodeIds: readonly string[],
  options: {
    groupId?: string;
    title?: string;
    padding?: number;
    idFactory?: CanvasIdFactory;
  } = {}
): GroupNodesResult {
  const requested = [...new Set(nodeIds)];
  const byId = new Map(document.nodes.map((node) => [node.id, node]));
  if (requested.some((id) => !byId.has(id))) return { ok: false, reason: 'node_not_found' };

  const nodes = requested.map((id) => byId.get(id) as CanvasNode);
  if (nodes.some((node) => node.type === 'group' || node.groupId !== null)) {
    return { ok: false, reason: 'nested_group_not_supported' };
  }
  if (nodes.length < 2) return { ok: false, reason: 'requires_two_nodes' };

  const bounds = boundsForGraphNodes(nodes);
  if (!bounds) return { ok: false, reason: 'requires_two_nodes' };
  const padding = Number.isFinite(options.padding)
    ? Math.max(0, options.padding as number)
    : DEFAULT_GROUP_PADDING;
  const group = makeCanvasNode({
    id: options.groupId,
    type: 'group',
    position: { x: bounds.x - padding, y: bounds.y - padding },
    size: {
      width: bounds.width + padding * 2,
      height: bounds.height + padding * 2,
    },
    zIndex: Math.min(...nodes.map((node) => node.zIndex)) - 1,
    data: {
      title: options.title?.trim() || 'Group',
      color: null,
      collapsed: false,
    },
    idFactory: options.idFactory,
  });
  const selected = new Set(requested);

  return {
    ok: true,
    group,
    document: {
      ...document,
      nodes: [
        ...document.nodes.map((node) =>
          selected.has(node.id) && node.type !== 'group'
            ? { ...node, groupId: group.id }
            : node
        ),
        group,
      ],
    },
  };
}

export function ungroupCanvasNodes(
  document: CanvasDocument,
  groupId: string
): CanvasDocument | null {
  if (!document.nodes.some((node) => node.id === groupId && node.type === 'group')) return null;
  return {
    ...document,
    nodes: document.nodes
      .filter((node) => node.id !== groupId)
      .map((node) => (node.groupId === groupId ? { ...node, groupId: null } : node)),
  };
}

/** Expand selected groups to their members for move/copy operations. */
export function expandCanvasNodeIds(
  document: CanvasDocument,
  selectedIds: readonly string[]
): Set<string> {
  const expanded = new Set(selectedIds);
  const selectedGroups = new Set(
    document.nodes
      .filter((node) => node.type === 'group' && expanded.has(node.id))
      .map((node) => node.id)
  );
  for (const node of document.nodes) {
    if (node.groupId && selectedGroups.has(node.groupId)) expanded.add(node.id);
  }
  return expanded;
}

export function copyCanvasFragment(
  document: CanvasDocument,
  selectedIds: readonly string[]
): CanvasClipboard | null {
  const expanded = expandCanvasNodeIds(document, selectedIds);
  const nodes = document.nodes.filter((node) => expanded.has(node.id)).map(cloneCanvasNode);
  if (nodes.length === 0) return null;

  const copiedIds = new Set(nodes.map((node) => node.id));
  const copiedGroupIds = new Set(
    nodes.filter((node) => node.type === 'group').map((node) => node.id)
  );
  const promotedNodes = nodes.map((node) =>
    node.type !== 'group' && node.groupId && !copiedGroupIds.has(node.groupId)
      ? { ...node, groupId: null }
      : node
  );

  return {
    nodes: promotedNodes,
    connections: document.connections
      .filter(
        (edge) => copiedIds.has(edge.sourceNodeId) && copiedIds.has(edge.targetNodeId)
      )
      .map(cloneCanvasEdge),
  };
}

export interface PastedCanvasFragment {
  nodes: CanvasNode[];
  connections: CanvasEdge[];
  selectedIds: string[];
}

export function materializeCanvasPaste(
  clipboard: CanvasClipboard,
  options: {
    offset?: CanvasPoint;
    idFactory?: CanvasIdFactory;
  } = {}
): PastedCanvasFragment {
  const offset = options.offset ?? { x: 32, y: 32 };
  const idFactory = options.idFactory ?? createCanvasId;
  const idMap = new Map<string, string>();
  for (const node of clipboard.nodes) {
    idMap.set(node.id, idFactory(node.type === 'group' ? 'group' : 'node'));
  }

  const nodes = clipboard.nodes.map((node): CanvasNode => {
    const clone = cloneCanvasNode(node);
    const nextGroupId = clone.type === 'group' || !clone.groupId ? null : (idMap.get(clone.groupId) ?? null);
    return {
      ...clone,
      id: idMap.get(node.id) as string,
      position: {
        x: clone.position.x + offset.x,
        y: clone.position.y + offset.y,
      },
      groupId: nextGroupId,
    } as CanvasNode;
  });
  const connections = clipboard.connections.flatMap((edge) => {
    const sourceNodeId = idMap.get(edge.sourceNodeId);
    const targetNodeId = idMap.get(edge.targetNodeId);
    if (!sourceNodeId || !targetNodeId) return [];
    return [
      {
        ...cloneCanvasEdge(edge),
        id: idFactory('edge'),
        sourceNodeId,
        targetNodeId,
      },
    ];
  });

  return {
    nodes,
    connections,
    selectedIds: nodes.map((node) => node.id),
  };
}
