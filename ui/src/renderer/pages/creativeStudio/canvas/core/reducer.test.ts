/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { describe, expect, test } from 'bun:test';
import { canvasCommands } from './commands';
import {
  CANVAS_HISTORY_LIMIT,
  canvasReducer,
  canRedoCanvas,
  canUndoCanvas,
  createInitialCanvasState,
} from './reducer';
import {
  sequentialTestIdFactory,
  testDocument,
  testEdge,
  testNode,
} from './testFixtures';

describe('Creative Studio selection and movement reducer', () => {
  test('box-selects intersecting nodes with replace, add, and toggle modes', () => {
    const first = testNode('text', 1, { x: 0, y: 0, width: 100, height: 80 });
    const second = testNode('image', 2, { x: 200, y: 0, width: 100, height: 80 });
    let state = createInitialCanvasState({ document: testDocument([first, second]) });

    state = canvasReducer(state, canvasCommands.startBoxSelection({ x: -10, y: -10 }));
    state = canvasReducer(state, canvasCommands.updateBoxSelection({ x: 120, y: 90 }));
    expect(state.selection.nodeIds).toEqual([first.id]);
    expect(state.selection.marquee).toEqual({ x: -10, y: -10, width: 130, height: 100 });
    state = canvasReducer(state, canvasCommands.endBoxSelection());
    expect(state.selection.marquee).toBeNull();

    state = canvasReducer(state, canvasCommands.setSelection([second.id]));
    state = canvasReducer(
      state,
      canvasCommands.startBoxSelection({ x: -10, y: -10 }, 'add')
    );
    state = canvasReducer(state, canvasCommands.updateBoxSelection({ x: 120, y: 90 }));
    expect(new Set(state.selection.nodeIds)).toEqual(new Set([first.id, second.id]));

    state = canvasReducer(state, canvasCommands.endBoxSelection());
    state = canvasReducer(
      state,
      canvasCommands.startBoxSelection({ x: -10, y: -10 }, 'toggle')
    );
    state = canvasReducer(state, canvasCommands.updateBoxSelection({ x: 120, y: 90 }));
    expect(state.selection.nodeIds).toEqual([second.id]);
  });

  test('moves a multi-selection and expands an unlocked selected group to its members', () => {
    const group = testNode('group', 3, { x: 10, y: 20 });
    const child = testNode('text', 1, { x: 30, y: 40, groupId: group.id });
    const other = testNode('image', 2, { x: 200, y: 100 });
    let state = createInitialCanvasState({ document: testDocument([child, other, group]) });
    state = canvasReducer(state, canvasCommands.setSelection([group.id, other.id]));
    state = canvasReducer(state, canvasCommands.moveNodes({ x: 12, y: -8 }, { at: 1 }));

    expect(state.document.nodes.find((node) => node.id === group.id)?.position).toEqual({
      x: 22,
      y: 12,
    });
    expect(state.document.nodes.find((node) => node.id === child.id)?.position).toEqual({
      x: 42,
      y: 32,
    });
    expect(state.document.nodes.find((node) => node.id === other.id)?.position).toEqual({
      x: 212,
      y: 92,
    });
  });

  test('does not move a locked group or implicitly move its children', () => {
    const group = testNode('group', 3, { x: 10, y: 20, locked: true });
    const child = testNode('text', 1, { x: 30, y: 40, groupId: group.id });
    let state = createInitialCanvasState({ document: testDocument([child, group]) });
    state = canvasReducer(state, canvasCommands.setSelection([group.id]));
    const unchanged = canvasReducer(state, canvasCommands.moveNodes({ x: 20, y: 20 }, { at: 1 }));
    expect(unchanged).toBe(state);
  });
});

describe('Creative Studio delete reducer', () => {
  test('deletes nodes, selected connections, and incident connections atomically', () => {
    const first = testNode('text', 1);
    const second = testNode('image', 2);
    const connection = testEdge(10, first.id, second.id);
    let state = createInitialCanvasState({
      document: testDocument([first, second], [connection]),
    });
    state = canvasReducer(
      state,
      canvasCommands.deleteSelection({ nodeIds: [first.id], at: 1 })
    );
    expect(state.document.nodes.map((node) => node.id)).toEqual([second.id]);
    expect(state.document.connections).toEqual([]);
    expect(state.selection.nodeIds).toEqual([]);
  });

  test('deleting a group releases members by default and can explicitly cascade', () => {
    const group = testNode('group', 3);
    const child = testNode('text', 1, { groupId: group.id });
    let state = createInitialCanvasState({ document: testDocument([child, group]) });
    state = canvasReducer(
      state,
      canvasCommands.deleteSelection({ nodeIds: [group.id], at: 1 })
    );
    expect(state.document.nodes).toHaveLength(1);
    expect(state.document.nodes[0].id).toBe(child.id);
    expect(state.document.nodes[0].groupId).toBeNull();

    state = createInitialCanvasState({ document: testDocument([child, group]) });
    state = canvasReducer(
      state,
      canvasCommands.deleteSelection({
        nodeIds: [group.id],
        deleteGroupMembers: true,
        at: 1,
      })
    );
    expect(state.document.nodes).toEqual([]);
  });
});

describe('Creative Studio clipboard reducer', () => {
  test('copies and pastes a group graph with fresh selected ids', () => {
    const group = testNode('group', 3);
    const first = testNode('text', 1, { groupId: group.id });
    const second = testNode('image', 2, { x: 120, groupId: group.id });
    let state = createInitialCanvasState({
      document: testDocument([first, second, group], [testEdge(10, first.id, second.id)]),
    });
    state = canvasReducer(state, canvasCommands.setSelection([group.id]));
    state = canvasReducer(state, canvasCommands.copySelection());
    const paste = canvasCommands.pasteClipboard(state, {
      at: 10,
      idFactory: sequentialTestIdFactory(100),
    });
    if (!paste) throw new Error('expected paste command');
    state = canvasReducer(state, paste);

    expect(state.document.nodes).toHaveLength(6);
    expect(state.document.connections).toHaveLength(2);
    expect(state.selection.nodeIds).toHaveLength(3);
    expect(state.selection.nodeIds.some((id) => [group.id, first.id, second.id].includes(id))).toBe(false);
  });
});

describe('Creative Studio history reducer', () => {
  test('coalesces the same merge key through a 180ms quiet window', () => {
    const node = testNode('text', 1);
    let state = createInitialCanvasState({ document: testDocument([node]) });
    state = canvasReducer(
      state,
      canvasCommands.moveNodes({ x: 1, y: 0 }, { nodeIds: [node.id], at: 0 })
    );
    state = canvasReducer(
      state,
      canvasCommands.moveNodes({ x: 1, y: 0 }, { nodeIds: [node.id], at: 180 })
    );
    expect(state.history.past).toHaveLength(1);
    expect(state.document.nodes[0].position.x).toBe(2);

    state = canvasReducer(
      state,
      canvasCommands.moveNodes({ x: 1, y: 0 }, { nodeIds: [node.id], at: 361 })
    );
    expect(state.history.past).toHaveLength(2);
    expect(canUndoCanvas(state)).toBe(true);

    state = canvasReducer(state, canvasCommands.undo());
    expect(state.document.nodes[0].position.x).toBe(2);
    state = canvasReducer(state, canvasCommands.undo());
    expect(state.document.nodes[0].position.x).toBe(0);
    expect(canRedoCanvas(state)).toBe(true);
    state = canvasReducer(state, canvasCommands.redo());
    expect(state.document.nodes[0].position.x).toBe(2);
    state = canvasReducer(state, canvasCommands.redo());
    expect(state.document.nodes[0].position.x).toBe(3);
  });

  test('clears redo after a new edit and caps undo at 50 snapshots', () => {
    const node = testNode('text', 1);
    let state = createInitialCanvasState({ document: testDocument([node]) });
    for (let index = 0; index < 60; index += 1) {
      state = canvasReducer(
        state,
        canvasCommands.moveNodes(
          { x: 1, y: 0 },
          { nodeIds: [node.id], at: index * 1_000, mergeKey: `step:${index}` }
        )
      );
    }
    expect(state.history.past).toHaveLength(CANVAS_HISTORY_LIMIT);

    state = canvasReducer(state, canvasCommands.undo());
    expect(state.history.future).toHaveLength(1);
    state = canvasReducer(
      state,
      canvasCommands.moveNodes(
        { x: 5, y: 0 },
        { nodeIds: [node.id], at: 100_000, mergeKey: 'new-branch' }
      )
    );
    expect(state.history.future).toEqual([]);
  });

  test('does not record viewport or selection churn', () => {
    const node = testNode('text', 1);
    let state = createInitialCanvasState({ document: testDocument([node]) });
    state = canvasReducer(state, canvasCommands.setSelection([node.id]));
    state = canvasReducer(state, canvasCommands.panViewport({ x: 20, y: 10 }));
    state = canvasReducer(state, canvasCommands.zoomViewportAt(2, { x: 100, y: 50 }));
    expect(state.history.past).toEqual([]);
  });
});
