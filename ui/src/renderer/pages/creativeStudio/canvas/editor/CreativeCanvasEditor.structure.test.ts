/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { describe, expect, test } from 'bun:test';
import { readFileSync } from 'node:fs';

const editorSource = readFileSync(
  new URL('./CreativeCanvasEditor.tsx', import.meta.url),
  'utf8'
);
const editorStyles = readFileSync(
  new URL('./CreativeCanvasEditor.module.css', import.meta.url),
  'utf8'
);
const saveSource = readFileSync(new URL('./casSaveController.ts', import.meta.url), 'utf8');

describe('CreativeCanvasEditor composition contract', () => {
  test('composes canonical services, core reducer, and the shared surface', () => {
    for (const token of [
      'useCreativeProject',
      'canvasReducer',
      'CanvasSurface',
      'projectDocumentFromCanvasState',
      'renderNode',
      'renderEdge',
    ]) {
      expect(editorSource.includes(token)).toBe(true);
    }
    expect(editorSource.includes('CreativeNodeView')).toBe(false);
  });

  test('exposes canonical command/background control plus conflict-safe persistence', () => {
    for (const token of [
      'useImperativeHandle',
      'dispatch: applyCommand',
      'setBackground',
      'setPanels',
      'projectDocumentWithCanvasPanels',
      'addPendingTask',
      'removePendingTask',
      'getPendingTaskIds',
      'onPendingTaskIdsChange',
      'persistAgentSessions',
      'getAgentSessions',
      'getActiveAgentSessionId',
      'onAgentSessionsChange',
      'projectDocumentWithAgentSessions',
      'canonicalCreativePendingTaskIds',
      'await saveController.flush()',
      'saveController.queue(projectDocumentFromCanvasState(nextBase, stateRef.current))',
      'flush: () => saveController.flush()',
      'reloadRemote',
      "saveSnapshot.status === 'conflict'",
      '放弃本地更改并重新载入',
    ]) {
      expect(editorSource.includes(token)).toBe(true);
    }
    expect(saveSource.includes("kind === 'revision-conflict'")).toBe(true);
    expect(saveSource.includes('expectedRevision')).toBe(true);
  });

  test('owns pointer, wheel, and keyboard controller wiring', () => {
    for (const token of [
      'onPointerDown',
      'onPointerMove',
      'onPointerUp',
      'onWheel',
      'clientToCanvas',
      "if (tool === 'pan') return",
      'resolveCanvasKeyboardInput',
      'startCanvasResize',
      'updateCanvasResize',
      'startCanvasConnectionDrag',
      'finishCanvasConnectionDrag',
      'validateCanvasDropImport',
      'openCanvasContextMenu',
      'resolveCanvasDoubleClick',
      'onIntegrationIntent',
      'data-canvas-connection-handle',
      'data-connection-dragging',
      'data-resize-corner',
      'onContextMenu',
      'onDoubleClick',
      'onDrop',
    ]) {
      expect(editorSource.includes(token)).toBe(true);
    }
    expect(
      editorStyles.includes(".editor[data-connection-dragging='true'] .connectionHandle")
    ).toBe(true);
    expect(editorStyles.includes('pointer-events: auto')).toBe(true);
  });

  test('does not manufacture assets or invoke generation/model APIs', () => {
    for (const forbidden of [
      'useModelsForTask',
      'createGeneration',
      'invokeModel',
      'generateImage',
      'fakeAsset',
    ]) {
      expect(editorSource.includes(forbidden)).toBe(false);
    }
  });
});
