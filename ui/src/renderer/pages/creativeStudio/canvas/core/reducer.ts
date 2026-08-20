/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import type { CanvasCommand } from './commands';
import {
  cloneCanvasDocument,
  copyCanvasFragment,
  expandCanvasNodeIds,
  findCanvasGraphNode,
  groupCanvasNodes,
  ungroupCanvasNodes,
} from './document';
import { graphNodeIntersectsRect, normalizeSelectionRect } from './geometry';
import { validateCanvasConnection } from './graph';
import type {
  CanvasDocument,
  CanvasHistoryMeta,
  CanvasSelection,
  CanvasState,
} from './types';
import {
  DEFAULT_CANVAS_VIEWPORT,
  EMPTY_CANVAS_DOCUMENT,
  EMPTY_CANVAS_SELECTION,
} from './types';
import { normalizeCanvasViewport, panViewport, zoomViewportAtPoint } from './viewport';

export const CANVAS_HISTORY_LIMIT = 50;
export const CANVAS_HISTORY_MERGE_MS = 180;

const emptySelection = (): CanvasSelection => ({
  ...EMPTY_CANVAS_SELECTION,
  nodeIds: [],
  edgeIds: [],
});

export function createInitialCanvasState(
  input: {
    document?: CanvasDocument;
    viewport?: typeof DEFAULT_CANVAS_VIEWPORT;
  } = {}
): CanvasState {
  return {
    document: cloneCanvasDocument(input.document ?? EMPTY_CANVAS_DOCUMENT),
    viewport: normalizeCanvasViewport(input.viewport ?? DEFAULT_CANVAS_VIEWPORT),
    selection: emptySelection(),
    clipboard: null,
    history: { past: [], future: [], merge: null },
  };
}

function capHistory(documents: CanvasDocument[]): CanvasDocument[] {
  return documents.slice(-CANVAS_HISTORY_LIMIT);
}

function recordDocument(
  state: CanvasState,
  document: CanvasDocument,
  meta: CanvasHistoryMeta,
  selection: CanvasSelection = state.selection
): CanvasState {
  if (document === state.document) return state;
  const previousMerge = state.history.merge;
  const merge =
    meta.mergeKey !== undefined &&
    previousMerge?.key === meta.mergeKey &&
    meta.at >= previousMerge.at &&
    meta.at - previousMerge.at <= CANVAS_HISTORY_MERGE_MS;
  const past = merge
    ? state.history.past
    : capHistory([...state.history.past, cloneCanvasDocument(state.document)]);

  return {
    ...state,
    document,
    selection,
    history: {
      past,
      future: [],
      merge: meta.mergeKey ? { key: meta.mergeKey, at: meta.at } : null,
    },
  };
}

function undo(state: CanvasState): CanvasState {
  const previous = state.history.past.at(-1);
  if (!previous) return state;
  return {
    ...state,
    document: cloneCanvasDocument(previous),
    selection: emptySelection(),
    history: {
      past: state.history.past.slice(0, -1),
      future: capHistory([...state.history.future, cloneCanvasDocument(state.document)]),
      merge: null,
    },
  };
}

function redo(state: CanvasState): CanvasState {
  const next = state.history.future.at(-1);
  if (!next) return state;
  return {
    ...state,
    document: cloneCanvasDocument(next),
    selection: emptySelection(),
    history: {
      past: capHistory([...state.history.past, cloneCanvasDocument(state.document)]),
      future: state.history.future.slice(0, -1),
      merge: null,
    },
  };
}

function validSelection(
  state: CanvasState,
  nodeIds: readonly string[],
  edgeIds: readonly string[]
): Pick<CanvasSelection, 'nodeIds' | 'edgeIds'> {
  const existingNodeIds = new Set(state.document.nodes.map((node) => node.id));
  const existingEdgeIds = new Set(state.document.connections.map((edge) => edge.id));
  return {
    nodeIds: [...new Set(nodeIds)].filter((id) => existingNodeIds.has(id)),
    edgeIds: [...new Set(edgeIds)].filter((id) => existingEdgeIds.has(id)),
  };
}

function moveNodeIds(state: CanvasState, requested: readonly string[]): Set<string> {
  const direct = new Set(requested);
  const movable = new Set<string>();

  for (const id of direct) {
    const node = findCanvasGraphNode(state.document, id);
    if (!node || node.locked) continue;
    movable.add(id);
    if (node.type === 'group') {
      for (const child of state.document.nodes) {
        if (child.groupId === node.id) movable.add(child.id);
      }
    }
  }
  return movable;
}

function deleteSelection(state: CanvasState, command: Extract<CanvasCommand, { type: 'selection/delete' }>): CanvasState {
  const requestedNodeIds = command.nodeIds ?? state.selection.nodeIds;
  const deletedNodeIds = new Set(requestedNodeIds);
  const requestedEdgeIds = new Set(command.edgeIds ?? state.selection.edgeIds);

  if (command.deleteGroupMembers) {
    for (const node of state.document.nodes) {
      if (node.groupId && deletedNodeIds.has(node.groupId)) deletedNodeIds.add(node.id);
    }
  }

  const deletedGroupIds = new Set(
    state.document.nodes
      .filter((node) => node.type === 'group' && deletedNodeIds.has(node.id))
      .map((node) => node.id)
  );
  const nodes = state.document.nodes
    .filter((node) => !deletedNodeIds.has(node.id))
    .map((node) =>
      node.groupId && deletedGroupIds.has(node.groupId) ? { ...node, groupId: null } : node
    );
  const connections = state.document.connections.filter(
    (edge) =>
      !requestedEdgeIds.has(edge.id) &&
      !deletedNodeIds.has(edge.sourceNodeId) &&
      !deletedNodeIds.has(edge.targetNodeId)
  );
  if (
    nodes.length === state.document.nodes.length &&
    connections.length === state.document.connections.length
  ) {
    return state;
  }

  return recordDocument(
    state,
    { nodes, connections },
    command.history,
    emptySelection()
  );
}

export function canvasReducer(state: CanvasState, command: CanvasCommand): CanvasState {
  switch (command.type) {
    case 'document/load':
      return {
        ...createInitialCanvasState({
          document: command.document,
          viewport: command.viewport ?? state.viewport,
        }),
      };

    case 'node/add': {
      if (findCanvasGraphNode(state.document, command.node.id)) return state;
      if (command.node.type === 'group' && command.node.groupId !== null) return state;
      if (
        command.node.type !== 'group' &&
        command.node.groupId !== null &&
        !state.document.nodes.some(
          (node) => node.id === command.node.groupId && node.type === 'group'
        )
      ) {
        return state;
      }
      return recordDocument(
        state,
        { ...state.document, nodes: [...state.document.nodes, structuredClone(command.node)] },
        command.history,
        {
          ...emptySelection(),
          nodeIds: [command.node.id],
        }
      );
    }

    case 'node/move': {
      const requested = command.nodeIds ?? state.selection.nodeIds;
      const movable = moveNodeIds(state, requested);
      const dx = Number.isFinite(command.delta.x) ? command.delta.x : 0;
      const dy = Number.isFinite(command.delta.y) ? command.delta.y : 0;
      if (movable.size === 0 || (dx === 0 && dy === 0)) return state;
      const nodes = state.document.nodes.map((node) =>
        movable.has(node.id)
          ? {
              ...node,
              position: { x: node.position.x + dx, y: node.position.y + dy },
            }
          : node
      );
      return recordDocument(state, { ...state.document, nodes }, command.history);
    }

    case 'group/create': {
      if (findCanvasGraphNode(state.document, command.groupId)) return state;
      const result = groupCanvasNodes(
        state.document,
        command.nodeIds ?? state.selection.nodeIds,
        {
          groupId: command.groupId,
          title: command.title,
          padding: command.padding,
        }
      );
      if (!result.ok) return state;
      return recordDocument(state, result.document, command.history, {
        ...emptySelection(),
        nodeIds: [result.group.id],
      });
    }

    case 'group/ungroup': {
      const memberIds = state.document.nodes
        .filter((node) => node.groupId === command.groupId)
        .map((node) => node.id);
      const document = ungroupCanvasNodes(state.document, command.groupId);
      if (!document) return state;
      return recordDocument(state, document, command.history, {
        ...emptySelection(),
        nodeIds: memberIds,
      });
    }

    case 'edge/connect': {
      if (state.document.connections.some((edge) => edge.id === command.edge.id)) return state;
      const validation = validateCanvasConnection(state.document, command.edge);
      if (!validation.ok) return state;
      return recordDocument(
        state,
        {
          ...state.document,
          connections: [...state.document.connections, structuredClone(command.edge)],
        },
        command.history,
        {
          ...emptySelection(),
          edgeIds: [command.edge.id],
        }
      );
    }

    case 'edge/delete': {
      const ids = new Set(command.edgeIds ?? state.selection.edgeIds);
      if (ids.size === 0) return state;
      const connections = state.document.connections.filter((edge) => !ids.has(edge.id));
      if (connections.length === state.document.connections.length) return state;
      return recordDocument(
        state,
        { ...state.document, connections },
        command.history,
        {
          ...state.selection,
          edgeIds: state.selection.edgeIds.filter((id) => !ids.has(id)),
        }
      );
    }

    case 'selection/delete':
      return deleteSelection(state, command);

    case 'selection/set': {
      const selected = validSelection(
        state,
        command.nodeIds ?? [],
        command.edgeIds ?? []
      );
      return {
        ...state,
        selection: { ...emptySelection(), ...selected },
      };
    }

    case 'selection/toggle-node': {
      if (!findCanvasGraphNode(state.document, command.nodeId)) return state;
      const selected = new Set(state.selection.nodeIds);
      if (selected.has(command.nodeId)) selected.delete(command.nodeId);
      else selected.add(command.nodeId);
      return {
        ...state,
        selection: {
          ...state.selection,
          nodeIds: [...selected],
          edgeIds: [],
        },
      };
    }

    case 'selection/clear':
      return { ...state, selection: emptySelection() };

    case 'selection/box-start': {
      const initialNodeIds = command.mode === 'replace' ? [] : [...state.selection.nodeIds];
      return {
        ...state,
        selection: {
          nodeIds: initialNodeIds,
          edgeIds: [],
          marquee: { x: command.anchor.x, y: command.anchor.y, width: 0, height: 0 },
          box: {
            anchor: { ...command.anchor },
            current: { ...command.anchor },
            mode: command.mode ?? 'replace',
            initialNodeIds,
          },
        },
      };
    }

    case 'selection/box-update': {
      const box = state.selection.box;
      if (!box) return state;
      const marquee = normalizeSelectionRect(box.anchor, command.current);
      const intersecting = state.document.nodes
        .filter((node) => graphNodeIntersectsRect(node, marquee))
        .map((node) => node.id);
      const selected = new Set(box.initialNodeIds);
      if (box.mode === 'replace') selected.clear();
      for (const id of intersecting) {
        if (box.mode === 'toggle' && selected.has(id)) selected.delete(id);
        else selected.add(id);
      }
      return {
        ...state,
        selection: {
          ...state.selection,
          nodeIds: [...selected],
          marquee,
          box: { ...box, current: { ...command.current } },
        },
      };
    }

    case 'selection/box-end':
      if (!state.selection.box && !state.selection.marquee) return state;
      return {
        ...state,
        selection: { ...state.selection, marquee: null, box: null },
      };

    case 'viewport/set':
      return { ...state, viewport: normalizeCanvasViewport(command.viewport) };

    case 'viewport/pan':
      return { ...state, viewport: panViewport(state.viewport, command.delta) };

    case 'viewport/zoom-at':
      return {
        ...state,
        viewport: zoomViewportAtPoint(state.viewport, command.zoom, command.clientAnchor),
      };

    case 'clipboard/copy': {
      const clipboard = copyCanvasFragment(
        state.document,
        command.nodeIds ?? state.selection.nodeIds
      );
      return clipboard ? { ...state, clipboard } : state;
    }

    case 'clipboard/paste': {
      const nodeIds = new Set(state.document.nodes.map((node) => node.id));
      const edgeIds = new Set(state.document.connections.map((edge) => edge.id));
      if (
        command.fragment.nodes.some((node) => nodeIds.has(node.id)) ||
        command.fragment.connections.some((edge) => edgeIds.has(edge.id))
      ) {
        return state;
      }
      let document: CanvasDocument = {
        nodes: [...state.document.nodes, ...structuredClone(command.fragment.nodes)],
        connections: [...state.document.connections],
      };
      for (const connection of command.fragment.connections) {
        if (!validateCanvasConnection(document, connection).ok) continue;
        document = {
          ...document,
          connections: [...document.connections, structuredClone(connection)],
        };
      }
      return recordDocument(state, document, command.history, {
        ...emptySelection(),
        nodeIds: command.fragment.selectedIds,
      });
    }

    case 'history/undo':
      return undo(state);

    case 'history/redo':
      return redo(state);
  }
}

export function canUndoCanvas(state: CanvasState): boolean {
  return state.history.past.length > 0;
}

export function canRedoCanvas(state: CanvasState): boolean {
  return state.history.future.length > 0;
}

/** Utility for controllers that need group-expanded ids before starting a gesture. */
export function selectedCanvasNodeIds(state: CanvasState): Set<string> {
  return expandCanvasNodeIds(state.document, state.selection.nodeIds);
}
