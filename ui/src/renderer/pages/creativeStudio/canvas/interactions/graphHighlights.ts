/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import type { CanvasDocument } from '../core';

export type CanvasGraphHighlightDirection = 'upstream' | 'downstream' | 'both';

export interface CanvasGraphHighlight {
  rootNodeIds: ReadonlySet<string>;
  upstreamNodeIds: ReadonlySet<string>;
  downstreamNodeIds: ReadonlySet<string>;
  nodeIds: ReadonlySet<string>;
  edgeIds: ReadonlySet<string>;
}

const normalizedDepth = (depth: number | undefined): number => {
  if (depth === undefined) return 1;
  if (depth === Number.POSITIVE_INFINITY) return depth;
  return Number.isFinite(depth) ? Math.max(0, Math.trunc(depth)) : 1;
};

function traverse(
  document: CanvasDocument,
  roots: ReadonlySet<string>,
  direction: 'upstream' | 'downstream',
  maxDepth: number
): { nodes: Set<string>; edges: Set<string> } {
  const nodes = new Set<string>();
  const edges = new Set<string>();
  let frontier = new Set(roots);
  const visited = new Set(roots);
  let depth = 0;

  while (frontier.size > 0 && depth < maxDepth) {
    const next = new Set<string>();
    for (const edge of document.connections) {
      const currentId = direction === 'upstream' ? edge.targetNodeId : edge.sourceNodeId;
      if (!frontier.has(currentId)) continue;
      const relatedId = direction === 'upstream' ? edge.sourceNodeId : edge.targetNodeId;
      edges.add(edge.id);
      if (!roots.has(relatedId)) nodes.add(relatedId);
      if (!visited.has(relatedId)) {
        visited.add(relatedId);
        next.add(relatedId);
      }
    }
    frontier = next;
    depth += 1;
  }

  return { nodes, edges };
}

/** Defaults to the source product's one-hop related-node highlight. */
export function deriveCanvasGraphHighlight(
  document: CanvasDocument,
  rootNodeIds: readonly string[],
  options: { direction?: CanvasGraphHighlightDirection; maxDepth?: number } = {}
): CanvasGraphHighlight {
  const existing = new Set(document.nodes.map((node) => node.id));
  const roots = new Set([...new Set(rootNodeIds)].filter((id) => existing.has(id)));
  const depth = normalizedDepth(options.maxDepth);
  const direction = options.direction ?? 'both';
  const upstream =
    direction === 'upstream' || direction === 'both'
      ? traverse(document, roots, 'upstream', depth)
      : { nodes: new Set<string>(), edges: new Set<string>() };
  const downstream =
    direction === 'downstream' || direction === 'both'
      ? traverse(document, roots, 'downstream', depth)
      : { nodes: new Set<string>(), edges: new Set<string>() };

  return {
    rootNodeIds: roots,
    upstreamNodeIds: upstream.nodes,
    downstreamNodeIds: downstream.nodes,
    nodeIds: new Set([...roots, ...upstream.nodes, ...downstream.nodes]),
    edgeIds: new Set([...upstream.edges, ...downstream.edges]),
  };
}
