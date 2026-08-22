/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { describe, expect, test } from 'bun:test';
import { testDocument, testEdge, testNode } from '../core/testFixtures';
import { deriveCanvasGraphHighlight } from './graphHighlights';

describe('canvas graph highlight derivation', () => {
  const a = testNode('text', 1);
  const b = testNode('image', 2);
  const c = testNode('video', 3);
  const d = testNode('audio', 4);
  const ab = testEdge(10, a.id, b.id);
  const bc = testEdge(11, b.id, c.id);
  const db = testEdge(12, d.id, b.id);
  const document = testDocument([a, b, c, d], [ab, bc, db]);

  test('matches source one-hop upstream and downstream related highlighting', () => {
    const highlight = deriveCanvasGraphHighlight(document, [b.id]);
    expect([...highlight.upstreamNodeIds]).toEqual([a.id, d.id]);
    expect([...highlight.downstreamNodeIds]).toEqual([c.id]);
    expect([...highlight.edgeIds]).toEqual([ab.id, db.id, bc.id]);
  });

  test('supports directional transitive derivation without adding missing roots', () => {
    const downstream = deriveCanvasGraphHighlight(document, [a.id, 'missing'], {
      direction: 'downstream',
      maxDepth: Number.POSITIVE_INFINITY,
    });
    expect([...downstream.rootNodeIds]).toEqual([a.id]);
    expect([...downstream.downstreamNodeIds]).toEqual([b.id, c.id]);
    expect([...downstream.edgeIds]).toEqual([ab.id, bc.id]);
  });
});
