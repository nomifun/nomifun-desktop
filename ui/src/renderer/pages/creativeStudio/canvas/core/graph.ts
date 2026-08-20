/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { findCanvasGraphNode } from './document';
import type { CanvasDocument, CanvasEdge, CanvasIdFactory } from './types';
import { createCanvasId } from './document';

export type CanvasConnectionErrorCode =
  | 'missing_source'
  | 'missing_target'
  | 'self_connection'
  | 'duplicate_connection'
  | 'group_connection'
  | 'config_to_config'
  | 'director_output_not_supported'
  | 'director_requires_image_input';

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
 * A Director is an input-only scene node: it accepts image or panorama nodes.
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
  if (source.type === 'director') {
    return { ok: false, code: 'director_output_not_supported' };
  }
  if (target.type === 'director' && source.type !== 'image' && source.type !== 'panorama') {
    return { ok: false, code: 'director_requires_image_input' };
  }
  return { ok: true };
}

export type ConnectCanvasNodesResult =
  | { ok: true; edge: CanvasEdge }
  | { ok: false; code: CanvasConnectionErrorCode };

export function connectCanvasNodes(
  document: CanvasDocument,
  candidate: CanvasConnectionCandidate,
  options: {
    edgeId?: string;
    sourceHandle?: string | null;
    targetHandle?: string | null;
    idFactory?: CanvasIdFactory;
  } = {}
): ConnectCanvasNodesResult {
  const validation = validateCanvasConnection(document, candidate);
  if (!validation.ok) return validation;
  return {
    ok: true,
    edge: {
      id: options.edgeId ?? (options.idFactory ?? createCanvasId)('edge'),
      sourceNodeId: candidate.sourceNodeId,
      targetNodeId: candidate.targetNodeId,
      sourceHandle: options.sourceHandle ?? null,
      targetHandle: options.targetHandle ?? null,
    },
  };
}

export function incomingCanvasEdges(
  document: CanvasDocument,
  nodeId: string
): CanvasEdge[] {
  return document.connections.filter((edge) => edge.targetNodeId === nodeId);
}

export function outgoingCanvasEdges(
  document: CanvasDocument,
  nodeId: string
): CanvasEdge[] {
  return document.connections.filter((edge) => edge.sourceNodeId === nodeId);
}
