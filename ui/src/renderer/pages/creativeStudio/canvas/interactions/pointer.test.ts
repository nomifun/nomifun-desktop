/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { describe, expect, test } from 'bun:test';
import { testDocument, testNode, testUuid } from '../core/testFixtures';
import { finishCanvasConnectionDrag, startCanvasConnectionDrag, updateCanvasConnectionDrag } from './connection';
import { startCanvasResize, updateCanvasResize } from './resize';

describe('canvas pointer controllers', () => {
  test('resizes from a corner in world units and preserves the opposite edge', () => {
    const node = testNode('image', 1, { x: 50, y: 40, width: 320, height: 200 });
    const started = startCanvasResize(node, 7, { x: 100, y: 100 }, 'top-left', { x: 0, y: 0, zoom: 2 });
    expect(started.ok).toBe(true);
    if (!started.ok) return;
    const moved = updateCanvasResize(started.gesture, 7, { x: 60, y: 80 }, 20);
    expect(moved.node?.size).toEqual({ width: 340, height: 210 });
    expect(moved.node?.position).toEqual({ x: 30, y: 30 });
    expect(moved.command?.type).toBe('node/update');
  });

  test('enforces source minimums, optional ratio lock, pointer ownership, and node locks', () => {
    const node = testNode('video', 2, { width: 320, height: 180 });
    const started = startCanvasResize(node, 3, { x: 0, y: 0 }, 'bottom-right', { x: 0, y: 0, zoom: 1 }, { keepAspectRatio: true, aspectRatio: 16 / 9 });
    expect(started.ok).toBe(true);
    if (!started.ok) return;
    expect(updateCanvasResize(started.gesture, 9, { x: 10, y: 10 }).matched).toBe(false);
    const moved = updateCanvasResize(started.gesture, 3, { x: -1000, y: -1000 });
    expect(moved.node?.size.width).toBeCloseTo(284.444444, 5);
    expect(moved.node?.size.height).toBe(160);
    expect(startCanvasResize({ ...node, locked: true }, 3, { x: 0, y: 0 }, 'bottom-right', { x: 0, y: 0, zoom: 1 })).toEqual({ ok: false, reason: 'locked' });
  });

  test('allows only text nodes to collapse to the compact 88px height', () => {
    const text = testNode('text', 20, { width: 340, height: 160 });
    const textResize = startCanvasResize(
      text,
      20,
      { x: 0, y: 0 },
      'bottom-right',
      { x: 0, y: 0, zoom: 1 }
    );
    expect(textResize.ok).toBe(true);
    if (!textResize.ok) return;
    expect(
      updateCanvasResize(textResize.gesture, 20, { x: 0, y: -1_000 }).node
        ?.size
    ).toEqual({ width: 340, height: 88 });

    const image = testNode('image', 21, { width: 340, height: 160 });
    const imageResize = startCanvasResize(
      image,
      21,
      { x: 0, y: 0 },
      'bottom-right',
      { x: 0, y: 0, zoom: 1 }
    );
    expect(imageResize.ok).toBe(true);
    if (!imageResize.ok) return;
    expect(
      updateCanvasResize(imageResize.gesture, 21, { x: 0, y: -1_000 }).node
        ?.size
    ).toEqual({ width: 340, height: 160 });
  });

  test('orients target-handle drags, validates the graph, and emits blank-drop intent', () => {
    const text = testNode('text', 3);
    const image = testNode('image', 4);
    const document = testDocument([text, image]);
    const started = startCanvasConnectionDrag(document, {
      nodeId: image.id,
      handle: 'target',
      handleId: 'in',
      pointerId: 5,
      clientPosition: { x: 100, y: 80 },
      viewport: { x: 20, y: 10, zoom: 2 },
    });
    expect(started.ok).toBe(true);
    if (!started.ok) return;
    const moved = updateCanvasConnectionDrag(started.gesture, 5, { x: 140, y: 90 }, { x: 20, y: 10, zoom: 2 });
    expect(moved.worldPosition).toEqual({ x: 60, y: 40 });
    const connected = finishCanvasConnectionDrag(document, moved, 5, { nodeId: text.id, handleId: 'out' }, { edgeId: testUuid(90), at: 1 });
    expect(connected.commands[0]).toMatchObject({
      type: 'edge/connect',
      edge: { sourceNodeId: text.id, targetNodeId: image.id, sourceHandle: 'out', targetHandle: 'in' },
    });
    expect(connected.intents).toEqual([
      {
        type: 'connection/created',
        sourceNodeId: text.id,
        targetNodeId: image.id,
      },
    ]);
    expect(finishCanvasConnectionDrag(document, moved, 5, { nodeId: null }).intents[0]).toMatchObject({
      type: 'connection/create-node-menu/open',
      worldPosition: { x: 60, y: 40 },
    });
  });
});
