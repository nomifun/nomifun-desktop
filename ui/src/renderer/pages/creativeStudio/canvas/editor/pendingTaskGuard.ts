/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import type { CanvasCommand, CanvasState } from '../core';
import type { CreativeConfigOperation } from '../../domain';
import { canvasReducer } from '../core';

type CanvasMediaAssetNode = Extract<
  CanvasState['document']['nodes'][number],
  { type: 'image' | 'panorama' }
>;

const isCanvasMediaAssetNode = (
  node: CanvasState['document']['nodes'][number]
): node is CanvasMediaAssetNode => node.type === 'image' || node.type === 'panorama';

const sameOperation = (
  left: CreativeConfigOperation | null,
  right: CreativeConfigOperation | null
): boolean => {
  if (left === null || right === null) return left === right;
  if (
    left.kind !== right.kind ||
    left.sourceNodeId !== right.sourceNodeId ||
    left.sourceAssetId !== right.sourceAssetId
  ) {
    return false;
  }
  return left.kind !== 'image-mask-edit' ||
    (right.kind === 'image-mask-edit' &&
      left.markedReferenceAssetId === right.markedReferenceAssetId);
};

export interface PendingTaskCommandGuardResult {
  allowed: boolean;
  orphanedTaskIds: readonly string[];
}

export function pendingTaskCommandGuard(
  state: CanvasState,
  command: CanvasCommand,
  pendingTaskIds: readonly string[]
): PendingTaskCommandGuardResult {
  if (pendingTaskIds.length === 0) return { allowed: true, orphanedTaskIds: [] };
  const next = canvasReducer(state, command);
  const orphanedTaskIds = [...new Set(pendingTaskIds)].filter(
    (taskId) => {
      const currentOwners = state.document.nodes.filter(
        (node) => node.type === 'config' && node.data.taskId === taskId
      );
      const nextOwners = next.document.nodes.filter(
        (node) => node.type === 'config' && node.data.taskId === taskId
      );
      if (currentOwners.length !== 1 || nextOwners.length !== 1) return true;
      const currentOwner = currentOwners[0];
      const nextOwner = nextOwners[0];
      if (currentOwner.type !== 'config' || nextOwner.type !== 'config') {
        return true;
      }
      const operation = currentOwner.data.operation;
      if (!sameOperation(operation, nextOwner.data.operation)) return true;
      if (!operation) return false;
      const sourceNodeId = operation.sourceNodeId;
      const sourceType =
        operation.kind === 'video-node-compose'
          ? 'video'
          : operation.kind === 'audio-node-compose'
            ? 'audio'
            : 'image';
      const currentSource = state.document.nodes.find(
        (node) => node.id === sourceNodeId && node.type === sourceType
      );
      const nextSource = next.document.nodes.find(
        (node) => node.id === sourceNodeId && node.type === sourceType
      );
      if (!currentSource || !nextSource) return true;
      const authoritativeSourceReconcile =
        operation.sourceAssetId === null &&
        command.type === 'node/reconcile-runtime' &&
        command.node.id === sourceNodeId;
      if (
        !authoritativeSourceReconcile &&
        'assetId' in nextSource.data &&
        'assetId' in currentSource.data &&
        nextSource.data.assetId !== currentSource.data.assetId
      ) {
        return true;
      }

      for (const assetId of currentOwner.data.inputAssetIds) {
        const currentInputNodes = state.document.nodes.filter(
          (node): node is CanvasMediaAssetNode =>
            isCanvasMediaAssetNode(node) && node.data.assetId === assetId
        );
        if (currentInputNodes.length === 0) return true;
        for (const currentInput of currentInputNodes) {
          const nextInput = next.document.nodes.find(
            (node): node is CanvasMediaAssetNode =>
              node.id === currentInput.id && isCanvasMediaAssetNode(node)
          );
          if (!nextInput || nextInput.data.assetId !== assetId) return true;
          if (currentInput.id === sourceNodeId) continue;
          const currentReferenceEdge = state.document.connections.find(
            (edge) =>
              edge.sourceNodeId === currentInput.id &&
              edge.targetNodeId === sourceNodeId
          );
          if (
            currentReferenceEdge &&
            !next.document.connections.some(
              (edge) => edge.id === currentReferenceEdge.id
            )
          ) {
            return true;
          }
        }
      }
      return false;
    }
  );
  return { allowed: orphanedTaskIds.length === 0, orphanedTaskIds };
}

/**
 * A durable pending task must always retain exactly one config-node owner.
 * Simulating the pure reducer here covers every command source (keyboard,
 * context menus, toolbar actions, and imperative product integrations).
 */
export function canvasCommandPreservesPendingTaskOwners(
  state: CanvasState,
  pendingTaskIds: readonly string[],
  command: CanvasCommand,
): boolean {
  return pendingTaskCommandGuard(state, command, pendingTaskIds).allowed;
}
