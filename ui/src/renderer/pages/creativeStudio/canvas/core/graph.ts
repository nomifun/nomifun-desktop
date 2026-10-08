/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { findCanvasGraphNode } from './document';
import type { CanvasDocument } from './types';

export type CanvasConnectionErrorCode =
  | 'missing_source'
  | 'missing_target'
  | 'self_connection'
  | 'duplicate_connection'
  | 'group_connection'
  | 'config_to_config';

export type CanvasConnectionValidation =
  | { ok: true }
  | { ok: false; code: CanvasConnectionErrorCode };

export interface CanvasConnectionCandidate {
  sourceNodeId: string;
  targetNodeId: string;
}

/**
 * Validate one directed edge.
 *
 * Groups are visual containers and never participate in the generation graph.
 */
export function validateCanvasConnection(
  document: CanvasDocument,
  candidate: CanvasConnectionCandidate
): CanvasConnectionValidation {
  const source = findCanvasGraphNode(document, candidate.sourceNodeId);
  if (!source) return { ok: false, code: 'missing_source' };
  const target = findCanvasGraphNode(document, candidate.targetNodeId);
  if (!target) return { ok: false, code: 'missing_target' };
  if (source.id === target.id) return { ok: false, code: 'self_connection' };
  if (
    document.connections.some(
      (edge) =>
        edge.sourceNodeId === source.id && edge.targetNodeId === target.id
    )
  ) {
    return { ok: false, code: 'duplicate_connection' };
  }
  if (source.type === 'group' || target.type === 'group') {
    return { ok: false, code: 'group_connection' };
  }
  if (source.type === 'config' && target.type === 'config') {
    return { ok: false, code: 'config_to_config' };
  }
  return { ok: true };
}
