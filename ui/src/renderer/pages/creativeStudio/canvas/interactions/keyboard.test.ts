/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { describe, expect, test } from 'bun:test';
import { canvasReducer, createInitialCanvasState } from '../core';
import { testDocument, testNode } from '../core/testFixtures';
import { resolveCanvasKeyboardInput } from './keyboard';

describe('canvas keyboard controller', () => {
  const first = testNode('text', 1);
  const second = testNode('image', 2);

  test('selects all canonical nodes with Ctrl/Cmd+A and ignores editable targets', () => {
    const state = createInitialCanvasState({ document: testDocument([first, second]) });
    const resolution = resolveCanvasKeyboardInput(state, { key: 'a', metaKey: true });
    expect(resolution.preventDefault).toBe(true);
    expect(canvasReducer(state, resolution.commands[0]).selection.nodeIds).toEqual([first.id, second.id]);
    expect(resolveCanvasKeyboardInput(state, { key: 'a', ctrlKey: true, editable: true }).handled).toBe(false);
  });

  test('copies and pastes internally, then requests a real clipboard read when empty', () => {
    let state = createInitialCanvasState({ document: testDocument([first]) });
    state = canvasReducer(state, { type: 'selection/set', nodeIds: [first.id], edgeIds: [] });
    const copy = resolveCanvasKeyboardInput(state, { key: 'c', ctrlKey: true });
    state = canvasReducer(state, copy.commands[0]);
    const paste = resolveCanvasKeyboardInput(state, { key: 'v', ctrlKey: true }, { pasteSequence: 2, at: 10 });
    expect(paste.commands[0].type).toBe('clipboard/paste');
    state = canvasReducer(state, paste.commands[0]);
    expect(state.document.nodes[1].position).toEqual({ x: 64, y: 64 });

    const empty = createInitialCanvasState();
    expect(resolveCanvasKeyboardInput(empty, { key: 'v', ctrlKey: true }).intents).toEqual([
      { type: 'system-clipboard/read' },
    ]);
  });

  test('maps delete, undo/redo and escape without stealing unrelated keys', () => {
    let state = createInitialCanvasState({ document: testDocument([first]) });
    state = canvasReducer(state, { type: 'selection/set', nodeIds: [first.id], edgeIds: [] });
    expect(resolveCanvasKeyboardInput(state, { key: 'Delete' }).commands[0].type).toBe('selection/delete');
    expect(resolveCanvasKeyboardInput(state, { key: 'z', ctrlKey: true }).commands[0]).toEqual({ type: 'history/undo' });
    expect(resolveCanvasKeyboardInput(state, { key: 'z', ctrlKey: true, shiftKey: true }).commands[0]).toEqual({ type: 'history/redo' });
    expect(resolveCanvasKeyboardInput(state, { key: 'Escape' }).intents).toEqual([{ type: 'transient-ui/dismiss' }]);
    expect(resolveCanvasKeyboardInput(state, { key: 'q' }).handled).toBe(false);
  });
});
