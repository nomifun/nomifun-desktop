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
    (taskId) =>
      next.document.nodes.filter(
        (node) => node.type === 'config' && node.data.taskId === taskId
      ).length !== 1
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
