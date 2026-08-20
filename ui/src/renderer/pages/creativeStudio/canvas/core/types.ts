/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import type {
  CreativeCanvasConnection,
  CreativeCanvasNode,
  CreativeCanvasNodeKind,
  CreativePoint,
  CreativeProjectDocument,
  CreativeSize,
  CreativeViewport,
} from '../../domain';

/**
 * The core consumes the v1 domain types directly. It never defines a second
 * persistence schema and never accepts the retired WorkshopCanvasDoc shape.
 */
export type CanvasNodeType = CreativeCanvasNodeKind;
export type CanvasContentNodeType = Exclude<CanvasNodeType, 'group'>;
export type CanvasPoint = CreativePoint;
export type CanvasSize = CreativeSize;

export interface CanvasSelectionRect extends CanvasPoint, CanvasSize {}

/** World-to-client transform: client = world * zoom + translation. */
export type CanvasViewport = CreativeViewport;
export type CanvasNode = CreativeCanvasNode;
export type CanvasGroup = Extract<CanvasNode, { type: 'group' }>;
export type CanvasContentNode = Exclude<CanvasNode, { type: 'group' }>;
export type CanvasGraphNode = CanvasNode;
export type CanvasEdge = CreativeCanvasConnection;

/** Content-only state. Viewport and selection are editor state, not graph data. */
export interface CanvasDocument {
  nodes: CreativeProjectDocument['nodes'];
  connections: CreativeProjectDocument['connections'];
}

export type CanvasSelectionMode = 'replace' | 'add' | 'toggle';

export interface CanvasBoxSelection {
  anchor: CanvasPoint;
  current: CanvasPoint;
  mode: CanvasSelectionMode;
  initialNodeIds: string[];
}

export interface CanvasSelection {
  /** May contain both ordinary node ids and group ids. */
  nodeIds: string[];
  edgeIds: string[];
  marquee: CanvasSelectionRect | null;
  box: CanvasBoxSelection | null;
}

export interface CanvasClipboard {
  nodes: CanvasNode[];
  connections: CanvasEdge[];
}

export interface CanvasHistoryMerge {
  key: string;
  at: number;
}

export interface CanvasHistory {
  past: CanvasDocument[];
  future: CanvasDocument[];
  merge: CanvasHistoryMerge | null;
}

export interface CanvasState {
  document: CanvasDocument;
  viewport: CanvasViewport;
  selection: CanvasSelection;
  clipboard: CanvasClipboard | null;
  history: CanvasHistory;
}

export interface CanvasHistoryMeta {
  /** Monotonic event time in milliseconds, supplied by the command creator. */
  at: number;
  /** Commands with the same key inside the quiet window form one undo step. */
  mergeKey?: string;
}

export type CanvasIdFactory = (kind: 'node' | 'group' | 'edge') => string;

export const DEFAULT_CANVAS_VIEWPORT: CanvasViewport = { x: 0, y: 0, zoom: 1 };

export const EMPTY_CANVAS_DOCUMENT: CanvasDocument = {
  nodes: [],
  connections: [],
};

export const EMPTY_CANVAS_SELECTION: CanvasSelection = {
  nodeIds: [],
  edgeIds: [],
  marquee: null,
  box: null,
};
