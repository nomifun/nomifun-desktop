/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { describe, expect, test } from 'bun:test';

import { testEdge, testNode } from '../core/testFixtures';
import { computeCanvasViewportCulling } from './viewportCulling';

const viewport = { x: 0, y: 0, zoom: 1 };
const containerSize = { width: 200, height: 200 };

const sorted = (ids: ReadonlySet<string>): string[] => [...ids].sort();

describe('canvas viewport culling', () => {
  test('retains viewport nodes and only edges that can affect the viewport', () => {
    const visible = testNode('text', 1, { x: 20, y: 20 });
    const far = testNode('image', 2, { x: 800, y: 20 });
    const farther = testNode('video', 3, { x: 1_300, y: 20 });
    const visibleToFar = testEdge(10, visible.id, far.id);
    const farToFarther = testEdge(11, far.id, farther.id);

    const result = computeCanvasViewportCulling({
      nodes: [visible, far, farther],
      connections: [visibleToFar, farToFarther],
      viewport,
      containerSize,
      overscanPx: 0,
    });

    expect(result.renderAll).toBe(false);
    expect(sorted(result.nodeIds)).toEqual([visible.id]);
    expect(sorted(result.connectionIds)).toEqual([visibleToFar.id]);
    expect(result.worldRect).toEqual({ x: 0, y: 0, width: 200, height: 200 });
  });

  test('keeps a coarse crossing edge even when both endpoint nodes are outside the clip rect', () => {
    const left = testNode('text', 1, { x: -300, y: 80 });
    const right = testNode('image', 2, { x: 360, y: 80 });
    const crossing = testEdge(10, left.id, right.id);

    const result = computeCanvasViewportCulling({
      nodes: [left, right],
      connections: [crossing],
      viewport,
      containerSize,
      overscanPx: 0,
    });

    expect(sorted(result.nodeIds)).toEqual([]);
    expect(sorted(result.connectionIds)).toEqual([crossing.id]);
  });

  test('retains selected nodes and edges with their endpoints outside the viewport', () => {
    const source = testNode('text', 1, { x: 1_000, y: 1_000 });
    const target = testNode('image', 2, { x: 1_300, y: 1_000 });
    const selectedEdge = testEdge(10, source.id, target.id);

    const result = computeCanvasViewportCulling({
      nodes: [source, target],
      connections: [selectedEdge],
      viewport,
      containerSize,
      selectedNodeIds: [source.id],
      selectedEdgeIds: [selectedEdge.id],
      overscanPx: 0,
    });

    expect(sorted(result.nodeIds)).toEqual([source.id, target.id].sort());
    expect(sorted(result.connectionIds)).toEqual([selectedEdge.id]);
  });

  test('keeps group containers and their members for a required gesture node', () => {
    const group = testNode('group', 1, {
      x: 1_000,
      y: 1_000,
      width: 500,
      height: 300,
    });
    const requiredMember = testNode('text', 2, {
      x: 1_040,
      y: 1_060,
      groupId: group.id,
    });
    const sibling = testNode('image', 3, {
      x: 1_300,
      y: 1_060,
      groupId: group.id,
    });

    const result = computeCanvasViewportCulling({
      nodes: [group, requiredMember, sibling],
      connections: [],
      viewport,
      containerSize,
      requiredNodeIds: [requiredMember.id],
      overscanPx: 0,
    });

    expect(sorted(result.nodeIds)).toEqual([group.id, requiredMember.id, sibling.id].sort());
  });

  test('uses the large default overscan but falls back to all items without a reliable size', () => {
    const nearby = testNode('text', 1, { x: 880, y: 20 });
    const far = testNode('image', 2, { x: 2_000, y: 20 });
    const connection = testEdge(10, nearby.id, far.id);

    const overscanned = computeCanvasViewportCulling({
      nodes: [nearby, far],
      connections: [connection],
      viewport,
      containerSize,
    });
    expect(overscanned.nodeIds.has(nearby.id)).toBe(true);
    expect(overscanned.nodeIds.has(far.id)).toBe(false);

    const fallback = computeCanvasViewportCulling({
      nodes: [nearby, far],
      connections: [connection],
      viewport,
      containerSize: null,
      overscanPx: 0,
    });
    expect(fallback.renderAll).toBe(true);
    expect(sorted(fallback.nodeIds)).toEqual([nearby.id, far.id].sort());
    expect(sorted(fallback.connectionIds)).toEqual([connection.id]);
  });
});
