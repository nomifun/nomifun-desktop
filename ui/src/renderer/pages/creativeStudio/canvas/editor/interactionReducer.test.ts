/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { describe, expect, test } from 'bun:test';
import {
  canvasEditorInteractionReducer,
  INITIAL_CANVAS_EDITOR_INTERACTION,
} from './interactionReducer';

describe('canvas editor interaction reducer', () => {
  test('tracks one pointer-owned pan gesture', () => {
    const started = canvasEditorInteractionReducer(INITIAL_CANVAS_EDITOR_INTERACTION, {
      type: 'gesture/start',
      gesture: { kind: 'pan', pointerId: 4, lastClient: { x: 10, y: 20 } },
    });
    expect(started.isPanning).toBe(true);
    expect(
      canvasEditorInteractionReducer(started, {
        type: 'gesture/update',
        pointerId: 9,
        client: { x: 40, y: 60 },
      })
    ).toBe(started);

    const moved = canvasEditorInteractionReducer(started, {
      type: 'gesture/update',
      pointerId: 4,
      client: { x: 40, y: 60 },
    });
    expect(moved.gesture && 'lastClient' in moved.gesture ? moved.gesture.lastClient : null).toEqual({ x: 40, y: 60 });
    expect(
      canvasEditorInteractionReducer(moved, { type: 'gesture/end', pointerId: 9 })
    ).toBe(moved);
    expect(
      canvasEditorInteractionReducer(moved, { type: 'gesture/end', pointerId: 4 })
    ).toEqual(INITIAL_CANVAS_EDITOR_INTERACTION);
  });

  test('preserves the move merge key across pointer updates', () => {
    const started = canvasEditorInteractionReducer(INITIAL_CANVAS_EDITOR_INTERACTION, {
      type: 'gesture/start',
      gesture: {
        kind: 'move',
        pointerId: 2,
        lastClient: { x: 0, y: 0 },
        mergeKey: 'move:project:1',
      },
    });
    const moved = canvasEditorInteractionReducer(started, {
      type: 'gesture/update',
      pointerId: 2,
      client: { x: 8, y: 12 },
    });
    expect(moved.gesture).toEqual({
      kind: 'move',
      pointerId: 2,
      lastClient: { x: 8, y: 12 },
      mergeKey: 'move:project:1',
    });
  });

  test('replaces typed connection gestures without treating them as pan deltas', () => {
    const panning = canvasEditorInteractionReducer(INITIAL_CANVAS_EDITOR_INTERACTION, {
      type: 'gesture/start',
      gesture: { kind: 'pan', pointerId: 3, lastClient: { x: 1, y: 2 } },
    });
    const connection = canvasEditorInteractionReducer(panning, {
      type: 'gesture/replace',
      gesture: {
        kind: 'connection',
        pointerId: 3,
        fixedNodeId: 'node-1',
        fixedHandle: 'source',
        fixedHandleId: 'source',
        clientPosition: { x: 30, y: 40 },
        worldPosition: { x: 20, y: 25 },
      },
    });

    expect(connection.isPanning).toBe(false);
    expect(connection.gesture?.kind).toBe('connection');
    expect(
      canvasEditorInteractionReducer(connection, {
        type: 'gesture/update',
        pointerId: 3,
        client: { x: 90, y: 100 },
      })
    ).toBe(connection);
  });
});
