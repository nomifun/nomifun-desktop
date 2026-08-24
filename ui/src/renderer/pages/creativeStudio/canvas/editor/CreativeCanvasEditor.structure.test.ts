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
const cullingSource = readFileSync(new URL('./viewportCulling.ts', import.meta.url), 'utf8');
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
      'data-editor-disabled',
    ]) {
      expect(editorSource.includes(token)).toBe(true);
    }
    expect(editorSource.includes('CreativeNodeView')).toBe(false);
    expect(editorStyles.includes(".editor[data-editor-disabled='true']")).toBe(true);
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
      'onPendingTaskCommandBlocked',
      'persistAgentSessions',
      'getAgentSessions',
      'getActiveAgentSessionId',
      'onAgentSessionsChange',
      'projectDocumentWithAgentSessions',
      'canonicalCreativePendingTaskIds',
      'await saveController.flush()',
      'canvasSaveRequiresUnloadGuard',
      "event.returnValue = ''",
      'saveController.queue(projectDocumentFromCanvasState(nextBase, stateRef.current))',
      'flush: flushCanvasPersistence',
      'reloadRemote',
      "throw new Error('Creative canvas is read-only')",
      'showSaveState',
      "saveSnapshot.status === 'conflict'",
      'creativeStudio.canvas.save.reloadRemote',
    ]) {
      expect(editorSource.includes(token)).toBe(true);
    }
    expect(saveSource.includes("kind === 'revision-conflict'")).toBe(true);
    expect(saveSource.includes('expectedRevision')).toBe(true);
  });

  test('coalesces interaction persistence and memoizes graph render layers', () => {
    for (const token of [
      'shouldDeferPersistence',
      'schedulePersistence',
      'flushScheduledPersistence',
      'INTERACTION_SAVE_IDLE_MS',
      'const nodeLayer = useMemo',
      'const edgeLayer = useMemo',
    ]) {
      expect(editorSource.includes(token)).toBe(true);
    }
    expect(
      editorSource.includes(
        'saveController.queue(projectDocumentFromCanvasState(base, next));'
      )
    ).toBe(true);
  });

  test('culls only mounted world layers after measuring the viewport', () => {
    for (const token of [
      'computeCanvasViewportCulling',
      'ResizeObserver',
      'surfaceSize',
      'containerSize: surfaceSize',
      'requiredNodeIds: gestureRequiredNodeId',
      'const renderedNodes = useMemo',
      'const renderedConnections = useMemo',
      'renderedNodes.map',
      'renderedConnections.flatMap',
      'miniMap={resolveSlot(miniMap, context)}',
    ]) {
      expect(editorSource.includes(token)).toBe(true);
    }
    expect(cullingSource.includes('DEFAULT_CANVAS_VIEWPORT_OVERSCAN_PX')).toBe(true);
    expect(cullingSource.includes('return allItems(nodes, connections)')).toBe(true);
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

  test('keeps connection gestures separate from explicit node activation', () => {
    const nodePointerStart = editorSource.slice(
      editorSource.indexOf('const beginNodePointer'),
      editorSource.indexOf('const beginNodeResize')
    );
    const connectionStart = editorSource.slice(
      editorSource.indexOf('const beginConnectionDrag'),
      editorSource.indexOf('const handleSurfacePointerDown')
    );

    expect(nodePointerStart.includes('canvasCommands.setSelection([node.id])')).toBe(true);
    expect(connectionStart.includes('canvasCommands.clearSelection()')).toBe(true);
    expect(connectionStart.includes('canvasCommands.setSelection([node.id])')).toBe(false);
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
