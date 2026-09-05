/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import {
  canvasCommands,
  clientToCanvas,
  createCanvasId,
  findCanvasGraphNode,
  validateCanvasConnection,
  type CanvasConnectionCandidate,
  type CanvasConnectionErrorCode,
  type CanvasDocument,
  type CanvasIdFactory,
  type CanvasPoint,
  type CanvasViewport,
} from '../core';
import {
  canvasInteractionResolution,
  type CanvasInteractionResolution,
} from './types';

export type CanvasConnectionHandleKind = 'source' | 'target';

export interface CanvasConnectionDragGesture {
  kind: 'connection';
  pointerId: number;
  fixedNodeId: string;
  fixedHandle: CanvasConnectionHandleKind;
  fixedHandleId: string | null;
  /** Snapshot of the selected output nodes when the drag began. */
  fixedNodeIds?: readonly string[];
  hoverNodeId?: string | null;
  clientPosition: CanvasPoint;
  worldPosition: CanvasPoint;
}

export type StartCanvasConnectionDragResult =
  | { ok: true; gesture: CanvasConnectionDragGesture }
  | {
      ok: false;
      reason:
        | 'missing_origin'
        | 'locked_origin'
        | 'group_connection'
        | 'director_output_not_supported'
        | 'invalid_pointer';
    };

export interface CanvasConnectionDropTarget {
  nodeId: string | null;
  handleId?: string | null;
  /** True when the pointer hit a node but no valid opposite handle. */
  isNearNode?: boolean;
}

const finitePoint = (point: CanvasPoint): boolean =>
  Number.isFinite(point.x) && Number.isFinite(point.y);

export function startCanvasConnectionDrag(
  document: CanvasDocument,
  input: {
    nodeId: string;
    handle: CanvasConnectionHandleKind;
    handleId?: string | null;
    pointerId: number;
    clientPosition: CanvasPoint;
    viewport: CanvasViewport;
    selectedNodeIds?: readonly string[];
  }
): StartCanvasConnectionDragResult {
  const node = findCanvasGraphNode(document, input.nodeId);
  if (!node) return { ok: false, reason: 'missing_origin' };
  if (node.locked) return { ok: false, reason: 'locked_origin' };
  if (node.type === 'group') return { ok: false, reason: 'group_connection' };
  if (node.type === 'director' && input.handle === 'source') {
    return { ok: false, reason: 'director_output_not_supported' };
  }
  if (
    !Number.isInteger(input.pointerId) ||
    input.pointerId < 0 ||
    !finitePoint(input.clientPosition)
  ) {
    return { ok: false, reason: 'invalid_pointer' };
  }

  return {
    ok: true,
    gesture: {
      kind: 'connection',
      pointerId: input.pointerId,
      fixedNodeId: input.nodeId,
      fixedHandle: input.handle,
      fixedHandleId: input.handleId ?? null,
      fixedNodeIds:
        input.handle === 'source' && input.selectedNodeIds?.includes(node.id)
          ? document.nodes
              .filter((candidate) =>
                input.selectedNodeIds?.includes(candidate.id) &&
                !candidate.locked &&
                candidate.type !== 'group' &&
                candidate.type !== 'director'
              )
              .map((candidate) => candidate.id)
          : [node.id],
      clientPosition: { ...input.clientPosition },
      worldPosition: clientToCanvas(input.clientPosition, input.viewport),
    },
  };
}

/** Use the same validation for hover feedback and the eventual drop. */
export function planCanvasConnectionDrop(
  document: CanvasDocument,
  gesture: Pick<
    CanvasConnectionDragGesture,
    'fixedNodeId' | 'fixedNodeIds' | 'fixedHandle'
  >,
  nodeId: string
): { candidates: CanvasConnectionCandidate[]; rejected: CanvasConnectionErrorCode[] } {
  const candidates: CanvasConnectionCandidate[] = [];
  const rejected: CanvasConnectionErrorCode[] = [];
  for (const fixedNodeId of new Set(gesture.fixedNodeIds ?? [gesture.fixedNodeId])) {
    const candidate = gesture.fixedHandle === 'source'
      ? { sourceNodeId: fixedNodeId, targetNodeId: nodeId }
      : { sourceNodeId: nodeId, targetNodeId: fixedNodeId };
    const validation = validateCanvasConnection(document, candidate);
    if (validation.ok) candidates.push(candidate);
    else rejected.push(validation.code);
  }
  return { candidates, rejected };
}

export function updateCanvasConnectionDrag(
  gesture: CanvasConnectionDragGesture,
  pointerId: number,
  clientPosition: CanvasPoint,
  viewport: CanvasViewport
): CanvasConnectionDragGesture {
  if (gesture.pointerId !== pointerId || !finitePoint(clientPosition)) return gesture;
  return {
    ...gesture,
    clientPosition: { ...clientPosition },
    worldPosition: clientToCanvas(clientPosition, viewport),
  };
}

/**
 * Finish a handle drag. A blank drop emits a product intent because creating a
 * typed canonical node requires the product node factory, which core does not
 * own. A valid node drop emits the existing canonical edge command.
 */
export function finishCanvasConnectionDrag(
  document: CanvasDocument,
  gesture: CanvasConnectionDragGesture,
  pointerId: number,
  target: CanvasConnectionDropTarget,
  options: {
    at?: number;
    edgeId?: string;
    idFactory?: CanvasIdFactory;
    mergeKey?: string;
  } = {}
): CanvasInteractionResolution {
  if (gesture.pointerId !== pointerId) {
    return canvasInteractionResolution({ handled: false, preventDefault: false });
  }

  if (!target.nodeId) {
    return canvasInteractionResolution({
      intents: target.isNearNode
        ? [{ type: 'connection/rejected', code: 'no_valid_drop_target' }]
        : [
            {
              type: 'connection/create-node-menu/open',
              fixedNodeId: gesture.fixedNodeId,
              fixedHandle: gesture.fixedHandle,
              fixedHandleId: gesture.fixedHandleId,
              fixedNodeIds: gesture.fixedNodeIds,
              worldPosition: { ...gesture.worldPosition },
            },
          ],
    });
  }

  const { candidates, rejected } = planCanvasConnectionDrop(document, gesture, target.nodeId);
  if (candidates.length === 0) {
    return canvasInteractionResolution({
      intents: [{ type: 'connection/rejected', code: rejected[0] ?? 'missing_source' }],
    });
  }

  const at = options.at ?? Date.now();
  const firstEdgeId = options.edgeId ?? (options.idFactory ?? createCanvasId)('edge');
  const mergeKey = options.mergeKey ?? `connect:${firstEdgeId}`;
  const batch = (gesture.fixedNodeIds?.length ?? 1) > 1;
  const commands = candidates.map(({ sourceNodeId, targetNodeId }, index) =>
    canvasCommands.connect(sourceNodeId, targetNodeId, {
      at,
      mergeKey,
      edgeId: index === 0 ? firstEdgeId : undefined,
      idFactory: options.idFactory,
      sourceHandle: gesture.fixedHandle === 'source'
        ? gesture.fixedHandleId
        : (target.handleId ?? 'source'),
      targetHandle: gesture.fixedHandle === 'target'
        ? gesture.fixedHandleId
        : (target.handleId ?? 'target'),
    })
  );
  // Each canonical connect command selects its new edge. Restore the batch
  // after all edges exist so the user can connect the same sources again.
  if (batch) commands.push(canvasCommands.setSelection(gesture.fixedNodeIds ?? [gesture.fixedNodeId]));
  return canvasInteractionResolution({
    commands,
    intents: batch
      ? [{ type: 'connection/batch-created', count: candidates.length, skippedCount: rejected.length }]
      : [{ type: 'connection/created', ...candidates[0] }],
  });
}
