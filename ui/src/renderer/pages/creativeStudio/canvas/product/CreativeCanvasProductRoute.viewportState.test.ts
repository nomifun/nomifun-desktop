/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { describe, expect, test } from 'bun:test';
import { readFileSync } from 'node:fs';

import {
  canvasCommands,
  canvasReducer,
  createInitialCanvasState,
  type CanvasState,
} from '../core';
import { shouldPublishCanvasStateToProductRoute } from './CreativeCanvasProductRoute';

const source = readFileSync(
  new URL('./CreativeCanvasProductRoute.tsx', import.meta.url),
  'utf8'
);

describe('Creative Canvas product route state projection', () => {
  test('keeps viewport-only editor updates local to the editor', () => {
    const initial = createInitialCanvasState();
    const panned = canvasReducer(
      initial,
      canvasCommands.panViewport({ x: 24, y: -12 })
    );
    const zoomed = canvasReducer(
      panned,
      canvasCommands.zoomViewportAt(1.5, { x: 320, y: 180 })
    );

    expect(panned.viewport).not.toEqual(initial.viewport);
    expect(zoomed.viewport).not.toEqual(panned.viewport);
    for (const next of [panned, zoomed]) {
      expect(next.document).toBe(initial.document);
      expect(next.selection).toBe(initial.selection);
      expect(next.clipboard).toBe(initial.clipboard);
      expect(next.history).toBe(initial.history);
      expect(shouldPublishCanvasStateToProductRoute(initial, next)).toBe(false);
    }
  });

  test('publishes hydration and every state slice consumed by product chrome or panels', () => {
    const initial = createInitialCanvasState();
    const routeRelevantStates: CanvasState[] = [
      {
        ...initial,
        document: {
          nodes: [...initial.document.nodes],
          connections: initial.document.connections,
        },
      },
      {
        ...initial,
        selection: {
          ...initial.selection,
          nodeIds: ['selected-node'],
        },
      },
      {
        ...initial,
        clipboard: { nodes: [], connections: [] },
      },
      {
        ...initial,
        history: {
          ...initial.history,
          merge: { key: 'test', at: 1 },
        },
      },
    ];

    expect(shouldPublishCanvasStateToProductRoute(null, initial)).toBe(true);
    for (const next of routeRelevantStates) {
      expect(shouldPublishCanvasStateToProductRoute(initial, next)).toBe(true);
    }
  });

  test('wires the filtered callback instead of the raw React setter', () => {
    expect(source.includes('const canvasStateRef = useRef<CanvasState | null>(null);')).toBe(
      true
    );
    expect(source.includes('canvasStateRef.current = nextState')).toBe(true);
    expect(source.includes('if (!shouldPublishCanvasStateToProductRoute(currentState, nextState)) return;')).toBe(
      true
    );
    expect(source.includes('onStateChange={handleCanvasStateChange}')).toBe(true);
    expect(source.includes('onStateChange={setCanvasState}')).toBe(false);
  });
});
