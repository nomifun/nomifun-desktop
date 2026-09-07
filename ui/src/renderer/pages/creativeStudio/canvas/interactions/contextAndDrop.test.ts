/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { describe, expect, test } from 'bun:test';
import { canvasCommands, canvasReducer, createInitialCanvasState } from '../core';
import { sequentialTestIdFactory, testDocument, testEdge, testNode } from '../core/testFixtures';
import { resolveCanvasContextAction, resolveCanvasDoubleClick } from './context';
import { validateCanvasDropImport } from './drop';

describe('canvas context, double-click and drop intents', () => {
  test('deletes selected connections together and leaves unrelated edges and nodes intact', () => {
    const nodes = [testNode('image', 201), testNode('text', 202), testNode('video', 203)];
    const edges = [testEdge(211, nodes[0].id, nodes[2].id), testEdge(212, nodes[1].id, nodes[2].id), testEdge(213, nodes[0].id, nodes[1].id)];
    let state = createInitialCanvasState({ document: testDocument(nodes, edges) });
    state = canvasReducer(state, canvasCommands.setSelection([nodes[0].id], [edges[0].id, edges[1].id]));
    const result = resolveCanvasContextAction(state, { kind: 'edge', edgeId: edges[1].id }, 'delete');
    expect(result.commands).toHaveLength(1);
    const deleted = canvasReducer(state, result.commands[0]);
    expect(deleted.document.nodes).toEqual(nodes);
    expect(deleted.document.connections).toEqual([edges[2]]);
    expect(canvasReducer(deleted, canvasCommands.undo()).document.connections).toEqual(edges);
    const unselected = resolveCanvasContextAction(state, { kind: 'edge', edgeId: edges[2].id }, 'delete');
    expect(canvasReducer(state, unselected.commands[0]).document.connections).toEqual(edges.slice(0, 2));
  });

  test('uses canonical update/paste commands for lock and duplicate actions', () => {
    const node = testNode('text', 1);
    const state = createInitialCanvasState({ document: testDocument([node]) });
    const lock = resolveCanvasContextAction(state, { kind: 'node', nodeId: node.id }, 'toggle-lock', { at: 1 });
    expect(canvasReducer(state, lock.commands[0]).document.nodes[0].locked).toBe(true);

    const duplicate = resolveCanvasContextAction(state, { kind: 'node', nodeId: node.id }, 'duplicate', {
      at: 2,
      idFactory: sequentialTestIdFactory(100),
    });
    expect(canvasReducer(state, duplicate.commands[0]).document.nodes).toHaveLength(2);
  });

  test('derives typed double-click modes and canvas-world create position', () => {
    const node = testNode('config', 2);
    const text = testNode('text', 3);
    const state = createInitialCanvasState({ document: testDocument([node, text]), viewport: { x: 20, y: 10, zoom: 2 } });
    expect(resolveCanvasDoubleClick(state, { kind: 'node', nodeId: node.id }, { x: 0, y: 0 }, state.viewport).intents).toEqual([
      { type: 'node/open', nodeId: node.id, mode: 'compose' },
    ]);
    expect(resolveCanvasDoubleClick(state, { kind: 'node', nodeId: text.id }, { x: 0, y: 0 }, state.viewport).intents).toEqual([
      { type: 'node/open', nodeId: text.id, mode: 'edit-text' },
    ]);
    expect(resolveCanvasDoubleClick(state, { kind: 'canvas' }, { x: 120, y: 90 }, state.viewport).intents).toEqual([
      { type: 'canvas/create-node-menu/open', worldPosition: { x: 50, y: 40 } },
    ]);
  });

  test('emits only a real-file upload intent and reports current backend limits', () => {
    const image = new File(['image'], 'hero.png', { type: 'image/png' });
    const audio = new File(['audio'], 'voice.wav', { type: '' });
    const video = new File(['video'], 'clip.mp4', { type: 'video/mp4' });
    const result = validateCanvasDropImport([audio, image, video], { x: 120, y: 90 }, { x: 20, y: 10, zoom: 2 });
    expect(result.intent).toMatchObject({
      type: 'asset/import-file',
      file: image,
      kind: 'image',
      worldPosition: { x: 50, y: 40 },
      panoramaChoice: 'after-upload-if-2-to-1',
    });
    expect(result.rejected.map((item) => item.reason)).toEqual(['audio_unsupported']);
    expect(result.ignoredAcceptedFiles).toEqual([video]);
  });
});
