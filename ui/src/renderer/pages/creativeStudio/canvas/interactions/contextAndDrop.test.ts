/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { describe, expect, test } from 'bun:test';
import { canvasReducer, createInitialCanvasState } from '../core';
import { sequentialTestIdFactory, testDocument, testNode } from '../core/testFixtures';
import { resolveCanvasContextAction, resolveCanvasDoubleClick } from './context';
import { validateCanvasDropImport } from './drop';

describe('canvas context, double-click and drop intents', () => {
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
    const state = createInitialCanvasState({ document: testDocument([node]), viewport: { x: 20, y: 10, zoom: 2 } });
    expect(resolveCanvasDoubleClick(state, { kind: 'node', nodeId: node.id }, { x: 0, y: 0 }, state.viewport).intents).toEqual([
      { type: 'node/open', nodeId: node.id, mode: 'compose' },
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
