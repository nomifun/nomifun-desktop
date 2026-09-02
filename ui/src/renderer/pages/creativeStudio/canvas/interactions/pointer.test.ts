/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { describe, expect, test } from 'bun:test';
import { canvasCommands, canvasReducer, createInitialCanvasState } from '../core';
import { sequentialTestIdFactory, testDocument, testEdge, testNode, testUuid } from '../core/testFixtures';
import { finishCanvasConnectionDrag, planCanvasConnectionDrop, startCanvasConnectionDrag, updateCanvasConnectionDrag } from './connection';
import { startCanvasResize, updateCanvasResize } from './resize';

describe('canvas pointer controllers', () => {
  test('connects a stable multi-selection as one undoable operation and skips duplicates', () => {
    const sources = [testNode('image', 51), testNode('text', 52), testNode('panorama', 53)];
    const target = testNode('image', 54);
    const document = testDocument([...sources, target], [testEdge(60, sources[0].id, target.id)]);
    const selectedNodeIds = sources.map((source) => source.id).reverse();
    const started = startCanvasConnectionDrag(document, {
      nodeId: sources[1].id, handle: 'source', handleId: 'source',
      pointerId: 5, clientPosition: { x: 0, y: 0 },
      viewport: { x: 0, y: 0, zoom: 0.45 }, selectedNodeIds,
    });
    if (!started.ok) throw new Error('Connection did not start');
    selectedNodeIds.length = 0;
    expect(started.gesture.fixedNodeIds).toEqual(sources.map((source) => source.id));
    const result = finishCanvasConnectionDrag(document, started.gesture, 5, { nodeId: target.id }, {
      at: 100, idFactory: sequentialTestIdFactory(100),
    });
    expect(result.intents).toEqual([{ type: 'connection/batch-created', count: 2, skippedCount: 1 }]);
    let state = createInitialCanvasState({ document });
    for (const command of result.commands) state = canvasReducer(state, command);
    expect(state.document.connections.map((edge) => edge.sourceNodeId)).toEqual(sources.map((source) => source.id));
    expect(new Set(state.document.connections.map((edge) => edge.id)).size).toBe(3);
    state = canvasReducer(state, canvasCommands.undo());
    expect(state.document.connections).toEqual(document.connections);
    state = canvasReducer(state, canvasCommands.redo());
    expect(state.document.connections).toHaveLength(3);
    expect(finishCanvasConnectionDrag(document, started.gesture, 9, { nodeId: target.id }).commands).toEqual([]);
    expect(finishCanvasConnectionDrag(document, started.gesture, 5, { nodeId: null }).intents[0]).toMatchObject({
      type: 'connection/create-node-menu/open', fixedNodeIds: sources.map((source) => source.id),
    });
  });

  test('preserves graph restrictions for mixed batches and input-handle drags', () => {
    const image = testNode('image', 61);
    const text = testNode('text', 62);
    const director = testNode('director', 63);
    const group = testNode('group', 64);
    const locked = { ...testNode('image', 65), locked: true };
    const document = testDocument([image, text, director, group, locked]);
    const start = (nodeId: string, handle: 'source' | 'target') => startCanvasConnectionDrag(document, {
      nodeId, handle, pointerId: 1, clientPosition: { x: 0, y: 0 },
      viewport: { x: 0, y: 0, zoom: 1 }, selectedNodeIds: document.nodes.map((node) => node.id),
    });
    const batch = start(image.id, 'source');
    if (!batch.ok) throw new Error('Connection did not start');
    expect(batch.gesture.fixedNodeIds).toEqual([image.id, text.id]);
    expect(planCanvasConnectionDrop(document, batch.gesture, director.id)).toEqual({
      candidates: [{ sourceNodeId: image.id, targetNodeId: director.id }],
      rejected: ['director_requires_image_input'],
    });
    expect(finishCanvasConnectionDrag(document, batch.gesture, 1, { nodeId: group.id }).intents).toEqual([
      { type: 'connection/rejected', code: 'group_connection' },
    ]);
    const reverse = start(director.id, 'target');
    if (!reverse.ok) throw new Error('Connection did not start');
    expect(reverse.gesture.fixedNodeIds).toEqual([director.id]);
    expect(finishCanvasConnectionDrag(document, reverse.gesture, 1, { nodeId: image.id }).commands[0]).toMatchObject({
      edge: { sourceNodeId: image.id, targetNodeId: director.id },
    });
  });

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
