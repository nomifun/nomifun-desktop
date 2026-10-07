/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import type { CreativeCanvasNode, CreativeProjectDocument } from '../../domain';

type ConfigNode = Extract<CreativeCanvasNode, { type: 'config' }>;
type CanvasTaskDocument = Pick<CreativeProjectDocument, 'nodes'>;

/** All generation operations on an authored node share its serial task boundary. */
export function pendingCanvasNodeGenerationConfigs(
  document: CanvasTaskDocument,
  nodeId: string,
  pendingTaskIds: readonly string[] = []
): ConfigNode[] {
  const pendingIds = new Set(pendingTaskIds);
  return document.nodes.filter((node): node is ConfigNode => {
    if (node.type !== 'config' || node.data.operation?.sourceNodeId !== nodeId) return false;
    return node.data.status === 'queued' || node.data.status === 'running' ||
      (node.data.taskId !== null && pendingIds.has(node.data.taskId));
  });
}

export function canvasNodeGenerationTaskSummary(
  document: CanvasTaskDocument,
  nodeId: string,
  latestConfig: ConfigNode | null,
  pendingTaskIds: readonly string[] = [],
  runtimeBusy = false
) {
  const pending = pendingCanvasNodeGenerationConfigs(document, nodeId, pendingTaskIds);
  return {
    state: pending.some((node) => node.data.status === 'running')
      ? 'running' as const
      : pending.some((node) => node.data.status === 'queued')
        ? 'queued' as const
        : latestConfig?.data.status ?? 'idle' as const,
    pendingCount: Math.max(pending.length, runtimeBusy ? 1 : 0),
    message: latestConfig?.data.errorMessage ?? undefined,
  };
}
