/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import {
  canvasCommands,
  copyCanvasFragment,
  findCanvasGraphNode,
  type CanvasIdFactory,
  type CanvasPoint,
  type CanvasState,
  type CanvasViewport,
  clientToCanvas,
} from '../core';
import {
  canvasInteractionResolution,
  type CanvasContextTarget,
  type CanvasInteractionResolution,
  type CanvasNodeOpenMode,
  unhandledCanvasInteraction,
} from './types';

export type CanvasContextAction = 'open' | 'duplicate' | 'delete' | 'toggle-lock';

const openMode = (type: CanvasState['document']['nodes'][number]['type']): CanvasNodeOpenMode => {
  switch (type) {
    case 'text': return 'edit-text';
    case 'config': return 'compose';
    case 'director': return 'open-director';
    case 'group': return 'inspect-group';
    default: return 'preview-media';
  }
};

export function openCanvasContextMenu(
  target: CanvasContextTarget,
  clientPosition: CanvasPoint
): CanvasInteractionResolution {
  return canvasInteractionResolution({
    intents: [{ type: 'context-menu/open', target, clientPosition: { ...clientPosition } }],
  });
}

export function resolveCanvasDoubleClick(
  state: CanvasState,
  target: CanvasContextTarget,
  localClientPosition: CanvasPoint,
  viewport: CanvasViewport
): CanvasInteractionResolution {
  if (target.kind === 'canvas') {
    return canvasInteractionResolution({
      intents: [{
        type: 'canvas/create-node-menu/open',
        worldPosition: clientToCanvas(localClientPosition, viewport),
      }],
    });
  }
  if (target.kind !== 'node') return unhandledCanvasInteraction();
  const node = findCanvasGraphNode(state.document, target.nodeId);
  if (!node) return unhandledCanvasInteraction();
  return canvasInteractionResolution({
    intents: [{ type: 'node/open', nodeId: node.id, mode: openMode(node.type) }],
  });
}

export function toggleCanvasNodeLock(
  state: CanvasState,
  nodeId: string,
  at?: number
): CanvasInteractionResolution {
  const node = findCanvasGraphNode(state.document, nodeId);
  if (!node) return unhandledCanvasInteraction();
  return canvasInteractionResolution({
    commands: [canvasCommands.updateNode({ ...structuredClone(node), locked: !node.locked }, { at })],
  });
}

export function resolveCanvasContextAction(
  state: CanvasState,
  target: CanvasContextTarget,
  action: CanvasContextAction,
  options: { at?: number; idFactory?: CanvasIdFactory; offset?: CanvasPoint } = {}
): CanvasInteractionResolution {
  if (action === 'open') {
    if (target.kind !== 'node') return unhandledCanvasInteraction();
    const node = findCanvasGraphNode(state.document, target.nodeId);
    return node
      ? canvasInteractionResolution({
          intents: [{ type: 'node/open', nodeId: node.id, mode: openMode(node.type) }],
        })
      : unhandledCanvasInteraction();
  }
  if (action === 'toggle-lock') {
    return target.kind === 'node'
      ? toggleCanvasNodeLock(state, target.nodeId, options.at)
      : unhandledCanvasInteraction();
  }
  if (action === 'delete') {
    if (target.kind === 'node') {
      return canvasInteractionResolution({
        commands: [canvasCommands.deleteSelection({ nodeIds: [target.nodeId], at: options.at })],
      });
    }
    if (target.kind === 'edge') {
      const edgeIds = state.selection.edgeIds.includes(target.edgeId)
        ? state.selection.edgeIds
        : [target.edgeId];
      return canvasInteractionResolution({
        commands: [canvasCommands.deleteEdges(edgeIds, { at: options.at })],
      });
    }
    return unhandledCanvasInteraction();
  }
  if (target.kind !== 'node') return unhandledCanvasInteraction();

  const clipboard = copyCanvasFragment(state.document, [target.nodeId]);
  if (!clipboard) return unhandledCanvasInteraction();
  const paste = canvasCommands.pasteClipboard(
    { ...state, clipboard },
    { offset: options.offset ?? { x: 32, y: 32 }, at: options.at, idFactory: options.idFactory }
  );
  return paste
    ? canvasInteractionResolution({ commands: [paste] })
    : unhandledCanvasInteraction();
}
