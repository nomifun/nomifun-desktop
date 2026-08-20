/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import {
  canvasCommands,
  clientToCanvas,
  findCanvasGraphNode,
  validateCanvasConnection,
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
      clientPosition: { ...input.clientPosition },
      worldPosition: clientToCanvas(input.clientPosition, input.viewport),
    },
  };
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
  options: { at?: number; edgeId?: string; idFactory?: CanvasIdFactory } = {}
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
              worldPosition: { ...gesture.worldPosition },
            },
          ],
    });
  }

  const sourceNodeId =
    gesture.fixedHandle === 'source' ? gesture.fixedNodeId : target.nodeId;
  const targetNodeId =
    gesture.fixedHandle === 'source' ? target.nodeId : gesture.fixedNodeId;
  const validation = validateCanvasConnection(document, {
    sourceNodeId,
    targetNodeId,
  });
  if (!validation.ok) {
    return canvasInteractionResolution({
      intents: [{ type: 'connection/rejected', code: validation.code }],
    });
  }

  return canvasInteractionResolution({
    commands: [
      canvasCommands.connect(sourceNodeId, targetNodeId, {
        at: options.at,
        edgeId: options.edgeId,
        idFactory: options.idFactory,
        sourceHandle:
          gesture.fixedHandle === 'source'
            ? gesture.fixedHandleId
            : (target.handleId ?? null),
        targetHandle:
          gesture.fixedHandle === 'target'
            ? gesture.fixedHandleId
            : (target.handleId ?? null),
      }),
    ],
  });
}
