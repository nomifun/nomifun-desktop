/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { parseCreationTaskId } from '@/common/types/ids';

import {
  boundsForGraphNodes,
  clampCanvasZoom,
  createInitialCanvasState,
  type CanvasState,
  type CanvasViewport,
} from '../core';
import type {
  CreativeCanvasBackground,
  CreativeChatSessionReference,
  CreativeProjectDetail,
  CreativeProjectDocument,
  CreativeSize,
  CreativeStudioPanelState,
} from '../../domain';
import { parseCreativeProjectDocument } from '../../domain';
import { isCreativeProjectRepositoryError } from '../../services';
import type { CanvasBackgroundMode } from '../components';
import type { CanvasCasSaveSnapshot } from './casSaveController';

export type CreativeCanvasLoadState = 'loading' | 'not-found' | 'error' | 'ready';

export function classifyCreativeCanvasLoadState(input: {
  projectId: string;
  detail: CreativeProjectDetail | undefined;
  isLoading: boolean;
  error: Error | undefined;
}): CreativeCanvasLoadState {
  if (input.error) {
    return isCreativeProjectRepositoryError(input.error) && input.error.kind === 'not-found'
      ? 'not-found'
      : 'error';
  }
  if (
    input.detail &&
    input.detail.project.projectId === input.projectId &&
    input.detail.document.projectId === input.projectId
  ) {
    return 'ready';
  }
  return input.isLoading ? 'loading' : 'not-found';
}

/**
 * SWR can synchronously expose an old cached revision before its mount
 * revalidation finishes. Accept the authoritative follow-up only while the
 * editor is still idle; never overwrite dirty, saving, failed, or conflicted
 * local work.
 */
export function shouldHydrateCreativeCanvasDetail(input: {
  projectId: string;
  loadedProjectId: string | null;
  loadedRevision: string | null;
  detail: CreativeProjectDetail | undefined;
  save: Pick<CanvasCasSaveSnapshot, 'status' | 'hasPendingChanges'>;
}): boolean {
  const detail = input.detail;
  if (
    !detail ||
    detail.project.projectId !== input.projectId ||
    detail.document.projectId !== input.projectId
  ) {
    return false;
  }
  if (input.loadedProjectId !== input.projectId) return true;
  if (input.loadedRevision === detail.project.revision) return false;
  return input.save.status === 'idle' && !input.save.hasPendingChanges;
}

export function canvasStateFromProjectDocument(
  document: CreativeProjectDocument
): CanvasState {
  return createInitialCanvasState({
    document: {
      nodes: document.nodes,
      connections: document.connections,
    },
    viewport: document.viewport,
  });
}

export function projectDocumentFromCanvasState(
  base: CreativeProjectDocument,
  state: CanvasState
): CreativeProjectDocument {
  return {
    ...base,
    viewport: { ...state.viewport },
    nodes: structuredClone(state.document.nodes),
    connections: structuredClone(state.document.connections),
  };
}

export function creativeStudioPanelStateEqual(
  left: CreativeStudioPanelState,
  right: CreativeStudioPanelState
): boolean {
  return (
    left.left.open === right.left.open &&
    left.left.width === right.left.width &&
    left.left.activeView === right.left.activeView &&
    left.right.open === right.right.open &&
    left.right.width === right.right.width &&
    left.right.activeView === right.right.activeView &&
    left.bottom.open === right.bottom.open &&
    left.bottom.height === right.bottom.height &&
    left.bottom.activeView === right.bottom.activeView
  );
}

/** Merge panel chrome and reducer-owned canvas fields into one canonical save unit. */
export function projectDocumentWithCanvasPanels(
  base: CreativeProjectDocument,
  state: CanvasState,
  panels: CreativeStudioPanelState
): CreativeProjectDocument {
  return projectDocumentFromCanvasState(
    { ...base, panels: structuredClone(panels) },
    state
  );
}

export function canonicalCreativePendingTaskIds(
  taskIds: readonly string[]
): string[] {
  return [...new Set(taskIds.map((taskId) => String(parseCreationTaskId(taskId))))];
}

/** Merge the recovery feed with the latest reducer state as one CAS document. */
export function projectDocumentWithPendingTaskIds(
  base: CreativeProjectDocument,
  state: CanvasState,
  taskIds: readonly string[]
): CreativeProjectDocument {
  return projectDocumentFromCanvasState(
    { ...base, pendingTaskIds: canonicalCreativePendingTaskIds(taskIds) },
    state
  );
}

/** Merge NomiFun-owned Agent references with the latest reducer state as one validated CAS unit. */
export function projectDocumentWithAgentSessions(
  base: CreativeProjectDocument,
  state: CanvasState,
  sessions: readonly CreativeChatSessionReference[],
  activeChatId: string | null
): CreativeProjectDocument {
  return parseCreativeProjectDocument(
    projectDocumentFromCanvasState(
      {
        ...base,
        chatSessions: structuredClone([...sessions]),
        activeChatId,
      },
      state
    ),
    base.projectId
  );
}

export function canvasSurfaceBackground(
  background: CreativeCanvasBackground
): CanvasBackgroundMode {
  return background;
}

export function fitCanvasViewport(
  state: Pick<CanvasState, 'document'>,
  container: CreativeSize,
  padding = 72
): CanvasViewport {
  const width = Number.isFinite(container.width) ? Math.max(1, container.width) : 1;
  const height = Number.isFinite(container.height) ? Math.max(1, container.height) : 1;
  const safePadding = Number.isFinite(padding)
    ? Math.max(0, Math.min(padding, Math.min(width, height) / 2 - 1))
    : 0;
  const bounds = boundsForGraphNodes(state.document.nodes);
  if (!bounds) return { x: width / 2, y: height / 2, zoom: 1 };

  const availableWidth = Math.max(1, width - safePadding * 2);
  const availableHeight = Math.max(1, height - safePadding * 2);
  const zoom = clampCanvasZoom(
    Math.min(
      1,
      availableWidth / Math.max(bounds.width, 1),
      availableHeight / Math.max(bounds.height, 1)
    )
  );
  return {
    x: (width - bounds.width * zoom) / 2 - bounds.x * zoom,
    y: (height - bounds.height * zoom) / 2 - bounds.y * zoom,
    zoom,
  };
}

export function isCanvasKeyboardTarget(target: EventTarget | null): boolean {
  if (!(target instanceof HTMLElement)) return false;
  return (
    target.isContentEditable ||
    target.closest('input, textarea, select, [contenteditable="true"]') !== null
  );
}

/** Resolve a delegated Canvas event back to its stable node boundary. */
export function canvasNodeIdFromEventTarget(
  target: EventTarget | null
): string | null {
  if (!(target instanceof Element)) return null;
  return (
    target
      .closest<HTMLElement>('[data-canvas-node-id]')
      ?.dataset.canvasNodeId?.trim() || null
  );
}
