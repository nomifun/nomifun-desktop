/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import type { CanvasCommand, CanvasState } from '../core';
import { canvasReducer } from '../core';

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
      if (
        currentOwner.type !== 'config' ||
        nextOwner.type !== 'config' ||
        currentOwner.data.parameters.canvasOperation !== 'image-node-compose'
      ) {
        return false;
      }
      const sourceNodeId = currentOwner.data.parameters.sourceNodeId;
      if (
        typeof sourceNodeId !== 'string' ||
        !sourceNodeId.trim() ||
        nextOwner.data.parameters.sourceNodeId !== sourceNodeId
      ) {
        return true;
      }
      const currentSource = state.document.nodes.find(
        (node): node is Extract<(typeof state.document.nodes)[number], { type: 'image' }> =>
          node.id === sourceNodeId && node.type === 'image'
      );
      const nextSource = next.document.nodes.find(
        (node): node is Extract<(typeof next.document.nodes)[number], { type: 'image' }> =>
          node.id === sourceNodeId && node.type === 'image'
      );
      if (!currentSource || !nextSource) return true;
      const authoritativeSourceReconcile =
        command.type === 'node/reconcile-runtime' && command.node.id === sourceNodeId;
      return (
        !authoritativeSourceReconcile &&
        nextSource.data.assetId !== currentSource.data.assetId
      );
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
