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
} from '../core';
import { selectCreativeCanvasAgentContextInputs } from './CreativeCanvasProductRoute';

const source = readFileSync(
  new URL('./CreativeCanvasProductRoute.tsx', import.meta.url),
  'utf8'
);

describe('Creative Canvas product route Agent context projection', () => {
  test('does not expose document or selection dependencies while Assistant is closed', () => {
    const initial = createInitialCanvasState();
    const selected = {
      ...initial,
      selection: {
        ...initial.selection,
        nodeIds: ['selected-node'],
      },
    };

    expect(selectCreativeCanvasAgentContextInputs(initial, false)).toEqual([
      null,
      null,
    ]);
    expect(selectCreativeCanvasAgentContextInputs(selected, false)).toEqual([
      null,
      null,
    ]);
  });

  test('excludes viewport-only state and exposes the latest document when enabled', () => {
    const initial = createInitialCanvasState();
    const panned = canvasReducer(
      initial,
      canvasCommands.panViewport({ x: 24, y: -12 })
    );
    const document = {
      ...panned.document,
      nodes: [...panned.document.nodes],
    };
    const updated = { ...panned, document };

    const initialInputs = selectCreativeCanvasAgentContextInputs(initial, true);
    const pannedInputs = selectCreativeCanvasAgentContextInputs(panned, true);
    const updatedInputs = selectCreativeCanvasAgentContextInputs(updated, true);

    expect(pannedInputs[0]).toBe(initialInputs[0]);
    expect(pannedInputs[1]).toBe(initialInputs[1]);
    expect(updatedInputs[0]).toBe(document);
  });

  test('gates construction on the visible Assistant view and memoizes narrow inputs', () => {
    expect(source.includes("panelViews.right === 'assistant'")).toBe(true);
    expect(source.includes('projectId,\n        nodes: agentContextDocument.nodes')).toBe(
      true
    );
    expect(
      source.includes('selectedNodeIds: agentContextSelectedNodeIds')
    ).toBe(true);
    expect(
      source.includes(
        'agentContextDocument,\n    agentContextSelectedNodeIds,\n    projectId,\n    save.revision,'
      )
    ).toBe(true);
    expect(
      source.includes('[canvasState, project.detail, save.revision]')
    ).toBe(false);
    expect(source.includes('...project.detail.document')).toBe(false);
  });
});
