/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { createCanvasId, materializeCanvasPaste } from './document';
import type {
  CanvasDocument,
  CanvasEdge,
  CanvasGraphNode,
  CanvasHistoryMeta,
  CanvasIdFactory,
  CanvasPoint,
  CanvasSelectionMode,
  CanvasState,
  CanvasViewport,
} from './types';

type WithHistory<T> = T & { history: CanvasHistoryMeta };

export type CanvasCommand =
  | WithHistory<{ type: 'node/add'; node: CanvasGraphNode }>
  | WithHistory<{ type: 'node/update'; node: CanvasGraphNode }>
  | { type: 'node/reconcile-runtime'; node: CanvasGraphNode }
  | WithHistory<{
      type: 'node/move';
      nodeIds?: string[];
      delta: CanvasPoint;
    }>
  | WithHistory<{
      type: 'group/create';
      nodeIds?: string[];
      groupId: string;
      title?: string;
      padding?: number;
    }>
  | WithHistory<{ type: 'group/ungroup'; groupId: string }>
  | WithHistory<{ type: 'edge/connect'; edge: CanvasEdge }>
  | WithHistory<{ type: 'edge/delete'; edgeIds?: string[] }>
  | WithHistory<{
      type: 'selection/delete';
      nodeIds?: string[];
      edgeIds?: string[];
      deleteGroupMembers?: boolean;
    }>
  | { type: 'selection/set'; nodeIds?: string[]; edgeIds?: string[] }
  | { type: 'selection/toggle-node'; nodeId: string }
  | { type: 'selection/clear' }
  | {
      type: 'selection/box-start';
      anchor: CanvasPoint;
      mode?: CanvasSelectionMode;
    }
  | { type: 'selection/box-update'; current: CanvasPoint }
  | { type: 'selection/box-end' }
  | { type: 'viewport/set'; viewport: CanvasViewport }
  | { type: 'viewport/pan'; delta: CanvasPoint }
  | { type: 'viewport/zoom-at'; zoom: number; clientAnchor: CanvasPoint }
  | { type: 'clipboard/copy'; nodeIds?: string[] }
  | WithHistory<{
      type: 'clipboard/paste';
      fragment: ReturnType<typeof materializeCanvasPaste>;
    }>
  | { type: 'document/load'; document: CanvasDocument; viewport?: CanvasViewport }
  | { type: 'history/undo' }
  | { type: 'history/redo' };

function history(at: number | undefined, mergeKey?: string): CanvasHistoryMeta {
  return {
    at: at ?? Date.now(),
    ...(mergeKey ? { mergeKey } : {}),
  };
}

export const canvasCommands = {
  addNode(
    node: CanvasGraphNode,
    options: { at?: number; mergeKey?: string } = {}
  ): CanvasCommand {
    return {
      type: 'node/add',
      node,
      history: history(options.at, options.mergeKey),
    };
  },

  /**
   * Replace one canonical node without changing its identity or kind.
   *
   * Requiring a complete discriminated node keeps data fields type-safe and
   * avoids ambiguous deep-merge semantics for the per-kind `data` union.
   */
  updateNode(
    node: CanvasGraphNode,
    options: { at?: number; mergeKey?: string } = {}
  ): CanvasCommand {
    return {
      type: 'node/update',
      node,
      history: history(options.at, options.mergeKey),
    };
  },

  /**
   * Reconcile backend-authoritative runtime fields without creating a user
   * undo step. The reducer carries the new data/lock state into existing
   * history snapshots so undo/redo cannot regress a terminal task.
   */
  reconcileRuntimeNode(node: CanvasGraphNode): CanvasCommand {
    return { type: 'node/reconcile-runtime', node };
  },

  moveNodes(
    delta: CanvasPoint,
    options: { nodeIds?: readonly string[]; at?: number; mergeKey?: string } = {}
  ): CanvasCommand {
    const nodeIds = options.nodeIds ? [...new Set(options.nodeIds)] : undefined;
    const defaultMergeKey = nodeIds?.length
      ? `move:${[...nodeIds].sort().join(',')}`
      : 'move:selection';
    return {
      type: 'node/move',
      ...(nodeIds ? { nodeIds } : {}),
      delta,
      history: history(options.at, options.mergeKey ?? defaultMergeKey),
    };
  },

  groupNodes(
    options: {
      nodeIds?: readonly string[];
      groupId?: string;
      title?: string;
      padding?: number;
      at?: number;
      idFactory?: CanvasIdFactory;
    } = {}
  ): CanvasCommand {
    return {
      type: 'group/create',
      ...(options.nodeIds ? { nodeIds: [...new Set(options.nodeIds)] } : {}),
      groupId: options.groupId ?? (options.idFactory ?? createCanvasId)('group'),
      ...(options.title !== undefined ? { title: options.title } : {}),
      ...(options.padding !== undefined ? { padding: options.padding } : {}),
      history: history(options.at),
    };
  },

  ungroup(groupId: string, options: { at?: number } = {}): CanvasCommand {
    return { type: 'group/ungroup', groupId, history: history(options.at) };
  },

  connect(
    sourceNodeId: string,
    targetNodeId: string,
    options: {
      edgeId?: string;
      sourceHandle?: string | null;
      targetHandle?: string | null;
      at?: number;
      mergeKey?: string;
      idFactory?: CanvasIdFactory;
    } = {}
  ): CanvasCommand {
    return {
      type: 'edge/connect',
      edge: {
        id: options.edgeId ?? (options.idFactory ?? createCanvasId)('edge'),
        sourceNodeId,
        targetNodeId,
        sourceHandle: options.sourceHandle ?? null,
        targetHandle: options.targetHandle ?? null,
      },
      history: history(options.at, options.mergeKey),
    };
  },

  deleteEdges(edgeIds?: readonly string[], options: { at?: number } = {}): CanvasCommand {
    return {
      type: 'edge/delete',
      ...(edgeIds ? { edgeIds: [...new Set(edgeIds)] } : {}),
      history: history(options.at),
    };
  },

  deleteSelection(
    options: {
      nodeIds?: readonly string[];
      edgeIds?: readonly string[];
      deleteGroupMembers?: boolean;
      at?: number;
    } = {}
  ): CanvasCommand {
    return {
      type: 'selection/delete',
      ...(options.nodeIds ? { nodeIds: [...new Set(options.nodeIds)] } : {}),
      ...(options.edgeIds ? { edgeIds: [...new Set(options.edgeIds)] } : {}),
      ...(options.deleteGroupMembers ? { deleteGroupMembers: true } : {}),
      history: history(options.at),
    };
  },

  setSelection(nodeIds: readonly string[] = [], edgeIds: readonly string[] = []): CanvasCommand {
    return {
      type: 'selection/set',
      nodeIds: [...new Set(nodeIds)],
      edgeIds: [...new Set(edgeIds)],
    };
  },

  toggleNodeSelection(nodeId: string): CanvasCommand {
    return { type: 'selection/toggle-node', nodeId };
  },

  clearSelection(): CanvasCommand {
    return { type: 'selection/clear' };
  },

  startBoxSelection(
    anchor: CanvasPoint,
    mode: CanvasSelectionMode = 'replace'
  ): CanvasCommand {
    return { type: 'selection/box-start', anchor, mode };
  },

  updateBoxSelection(current: CanvasPoint): CanvasCommand {
    return { type: 'selection/box-update', current };
  },

  endBoxSelection(): CanvasCommand {
    return { type: 'selection/box-end' };
  },

  setViewport(viewport: CanvasViewport): CanvasCommand {
    return { type: 'viewport/set', viewport };
  },

  panViewport(delta: CanvasPoint): CanvasCommand {
    return { type: 'viewport/pan', delta };
  },

  zoomViewportAt(zoom: number, clientAnchor: CanvasPoint): CanvasCommand {
    return { type: 'viewport/zoom-at', zoom, clientAnchor };
  },

  copySelection(nodeIds?: readonly string[]): CanvasCommand {
    return {
      type: 'clipboard/copy',
      ...(nodeIds ? { nodeIds: [...new Set(nodeIds)] } : {}),
    };
  },

  pasteClipboard(
    state: CanvasState,
    options: {
      offset?: CanvasPoint;
      at?: number;
      idFactory?: CanvasIdFactory;
    } = {}
  ): CanvasCommand | null {
    if (!state.clipboard) return null;
    return {
      type: 'clipboard/paste',
      fragment: materializeCanvasPaste(state.clipboard, options),
      history: history(options.at),
    };
  },

  loadDocument(document: CanvasDocument, viewport?: CanvasViewport): CanvasCommand {
    return { type: 'document/load', document, ...(viewport ? { viewport } : {}) };
  },

  undo(): CanvasCommand {
    return { type: 'history/undo' };
  },

  redo(): CanvasCommand {
    return { type: 'history/redo' };
  },
};
