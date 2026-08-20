/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import React, {
  useCallback,
  useEffect,
  useImperativeHandle,
  useReducer,
  useRef,
  useState,
} from 'react';

import type {
  CreativeCanvasBackground,
  CreativeCanvasConnection,
  CreativeCanvasNode,
  CreativeProjectDetail,
  CreativeStudioPanelState,
} from '../../domain';
import {
  type CreativeProjectRepository,
  useCreativeProject,
} from '../../services';
import {
  canvasCommands,
  canvasReducer,
  clientToCanvas,
  createInitialCanvasState,
  type CanvasCommand,
  type CanvasState,
} from '../core';
import {
  CanvasSurface,
  type CanvasInteractionTool,
} from '../components';
import {
  type CanvasCasFlushResult,
  type CanvasCasSaveSnapshot,
} from './casSaveController';
import {
  canvasStateFromProjectDocument,
  canvasSurfaceBackground,
  canonicalCreativePendingTaskIds,
  classifyCreativeCanvasLoadState,
  creativeStudioPanelStateEqual,
  fitCanvasViewport,
  isCanvasKeyboardTarget,
  projectDocumentFromCanvasState,
  projectDocumentWithCanvasPanels,
  projectDocumentWithPendingTaskIds,
} from './editorModel';
import {
  canvasEditorInteractionReducer,
  INITIAL_CANVAS_EDITOR_INTERACTION,
  type CanvasEditorInteractionAction,
  type CanvasEditorInteractionState,
} from './interactionReducer';
import { useCanvasCasSave } from './useCanvasCasSave';
import styles from './CreativeCanvasEditor.module.css';

export interface CreativeCanvasNodeRenderContext {
  node: CreativeCanvasNode;
  selected: boolean;
  onActivate(): void;
  dragHandleProps: {
    onPointerDown: React.PointerEventHandler<HTMLElement>;
  };
}

export interface CreativeCanvasEdgeRenderContext {
  connection: CreativeCanvasConnection;
  source: CreativeCanvasNode;
  target: CreativeCanvasNode;
  selected: boolean;
  onActivate(): void;
}

export interface CreativeCanvasEditorContext {
  state: CanvasState;
  save: CanvasCasSaveSnapshot;
  tool: CanvasInteractionTool;
  /** Authoritative task ids persisted in the canonical project document. */
  pendingTaskIds: readonly string[];
  flush(): Promise<CanvasCasFlushResult>;
  reloadRemote(): Promise<boolean>;
}

export type CreativeCanvasEditorSlot =
  | React.ReactNode
  | ((context: CreativeCanvasEditorContext) => React.ReactNode);

export interface CreativeCanvasEditorHandle {
  /** Apply one canonical reducer command and queue persisted document changes. */
  dispatch(command: CanvasCommand): CanvasState;
  /** Update the canonical document background and queue it through the same CAS controller. */
  setBackground(background: CreativeCanvasBackground): void;
  /** Update persisted product panel state through the same canonical CAS document. */
  setPanels(panels: CreativeStudioPanelState): void;
  /** Durably persist a task id before submission so a later mount can recover it. */
  addPendingTask(taskId: string): Promise<void>;
  /** Durably remove a terminal or confirmed-orphan task id from the canonical feed. */
  removePendingTask(taskId: string): Promise<void>;
  /** Route guards must await this before leaving the editor. */
  flush(): Promise<CanvasCasFlushResult>;
  /** Explicitly discard local state and reload the authoritative remote revision. */
  reloadRemote(): Promise<boolean>;
  getState(): CanvasState;
  getSaveState(): CanvasCasSaveSnapshot;
  getPendingTaskIds(): readonly string[];
}

export interface CreativeCanvasEditorProps {
  projectId: string;
  tool: CanvasInteractionTool;
  renderNode(context: CreativeCanvasNodeRenderContext): React.ReactNode;
  renderEdge(context: CreativeCanvasEdgeRenderContext): React.ReactNode;
  repository?: CreativeProjectRepository;
  saveDebounceMs?: number;
  ariaLabel?: string;
  className?: string;
  topDock?: CreativeCanvasEditorSlot;
  leftPanel?: CreativeCanvasEditorSlot;
  rightPanel?: CreativeCanvasEditorSlot;
  bottomPanel?: CreativeCanvasEditorSlot;
  screenOverlay?: CreativeCanvasEditorSlot;
  miniMap?: CreativeCanvasEditorSlot;
  isMiniMapOpen?: boolean;
  onToggleMiniMap?: () => void;
  showZoomControls?: boolean;
  renderLoading?: () => React.ReactNode;
  renderNotFound?: (projectId: string) => React.ReactNode;
  renderError?: (error: Error, retry: () => Promise<CreativeProjectDetail | undefined>) => React.ReactNode;
  onStateChange?: (state: CanvasState) => void;
  onSaveStateChange?: (save: CanvasCasSaveSnapshot) => void;
  /** Fires after hydration and after each canonical task-feed mutation. */
  onPendingTaskIdsChange?: (taskIds: readonly string[]) => void;
}

const resolveSlot = (
  slot: CreativeCanvasEditorSlot | undefined,
  context: CreativeCanvasEditorContext
): React.ReactNode => (typeof slot === 'function' ? slot(context) : slot);

const defaultLoading = () => (
  <div className={styles.centerState} data-creative-canvas-state='loading' role='status'>
    正在载入画布…
  </div>
);

const defaultNotFound = (projectId: string) => (
  <div className={styles.centerState} data-creative-canvas-state='not-found' role='status'>
    <strong>画布不存在</strong>
    <span>{projectId}</span>
  </div>
);

const defaultError = (error: Error, retry: () => Promise<CreativeProjectDetail | undefined>) => (
  <div className={styles.centerState} data-creative-canvas-state='error' role='alert'>
    <strong>画布载入失败</strong>
    <span>{error.message}</span>
    <button type='button' onClick={() => void retry()}>
      重试
    </button>
  </div>
);

const CreativeCanvasEditor = React.forwardRef<CreativeCanvasEditorHandle, CreativeCanvasEditorProps>(
  (
    {
      projectId,
      tool,
      renderNode,
      renderEdge,
      repository,
      saveDebounceMs,
      ariaLabel = '创意工坊无限画布',
      className,
      topDock,
      leftPanel,
      rightPanel,
      bottomPanel,
      screenOverlay,
      miniMap,
      isMiniMapOpen = false,
      onToggleMiniMap,
      showZoomControls = true,
      renderLoading = defaultLoading,
      renderNotFound = defaultNotFound,
      renderError = defaultError,
      onStateChange,
      onSaveStateChange,
      onPendingTaskIdsChange,
    },
    ref
  ) => {
    const project = useCreativeProject(projectId, repository);
    const { controller: saveController, snapshot: saveSnapshot } = useCanvasCasSave(
      project.save,
      saveDebounceMs,
      projectId
    );
    const [state, setState] = useState<CanvasState>(() => createInitialCanvasState());
    const [background, setBackgroundState] = useState<CreativeCanvasBackground>('lines');
    const [pendingTaskIds, setPendingTaskIdsState] = useState<readonly string[] | null>(null);
    const stateRef = useRef(state);
    const pendingTaskIdsRef = useRef<readonly string[]>([]);
    const baseDocumentRef = useRef<CreativeProjectDetail['document'] | null>(null);
    const loadedProjectIdRef = useRef<string | null>(null);
    const hydratedSaveControllerRef = useRef<typeof saveController | null>(null);
    const surfaceRef = useRef<HTMLDivElement>(null);
    const pasteSequenceRef = useRef(0);
    const gestureSequenceRef = useRef(0);
    const [interaction, dispatchInteraction] = useReducer(
      canvasEditorInteractionReducer,
      INITIAL_CANVAS_EDITOR_INTERACTION
    );
    const interactionRef = useRef<CanvasEditorInteractionState>(interaction);

    const setInteraction = useCallback((action: CanvasEditorInteractionAction) => {
      interactionRef.current = canvasEditorInteractionReducer(interactionRef.current, action);
      dispatchInteraction(action);
    }, []);

    const hydrate = useCallback(
      (detail: CreativeProjectDetail) => {
        const next = canvasStateFromProjectDocument(detail.document);
        baseDocumentRef.current = structuredClone(detail.document);
        loadedProjectIdRef.current = detail.project.projectId;
        hydratedSaveControllerRef.current = saveController;
        stateRef.current = next;
        setState(next);
        setBackgroundState(detail.document.background);
        pendingTaskIdsRef.current = [...detail.document.pendingTaskIds];
        setPendingTaskIdsState(pendingTaskIdsRef.current);
        saveController.reset(detail.project.revision, detail.document);
        pasteSequenceRef.current = 0;
        setInteraction({ type: 'gesture/end' });
      },
      [saveController, setInteraction]
    );

    useEffect(() => {
      loadedProjectIdRef.current = null;
      baseDocumentRef.current = null;
      hydratedSaveControllerRef.current = null;
      pendingTaskIdsRef.current = [];
      setPendingTaskIdsState(null);
    }, [projectId]);

    useEffect(() => {
      const detail = project.detail;
      if (
        !detail ||
        detail.project.projectId !== projectId ||
        (loadedProjectIdRef.current === projectId &&
          hydratedSaveControllerRef.current === saveController)
      ) {
        return;
      }
      hydrate(detail);
    }, [hydrate, project.detail, projectId]);

    const reloadRemote = useCallback(async (): Promise<boolean> => {
      const detail = await project.refresh();
      if (!detail || detail.project.projectId !== projectId) return false;
      hydrate(detail);
      return true;
    }, [hydrate, project, projectId]);

    const applyCommand = useCallback(
      (command: CanvasCommand): CanvasState => {
        const current = stateRef.current;
        const next = canvasReducer(current, command);
        if (next === current) return current;
        stateRef.current = next;
        setState(next);

        const persistedChanged =
          next.document !== current.document || next.viewport !== current.viewport;
        const base = baseDocumentRef.current;
        if (persistedChanged && base) {
          saveController.queue(projectDocumentFromCanvasState(base, next));
        }
        return next;
      },
      [saveController]
    );

    const setBackground = useCallback(
      (nextBackground: CreativeCanvasBackground) => {
        const currentBase = baseDocumentRef.current;
        if (!currentBase || currentBase.background === nextBackground) return;

        const nextBase = {
          ...structuredClone(currentBase),
          background: nextBackground,
        };
        baseDocumentRef.current = nextBase;
        setBackgroundState(nextBackground);
        saveController.queue(projectDocumentFromCanvasState(nextBase, stateRef.current));
      },
      [saveController]
    );

    const setPanels = useCallback(
      (nextPanels: CreativeStudioPanelState) => {
        const currentBase = baseDocumentRef.current;
        if (!currentBase || creativeStudioPanelStateEqual(currentBase.panels, nextPanels)) {
          return;
        }

        const nextDocument = projectDocumentWithCanvasPanels(
          currentBase,
          stateRef.current,
          nextPanels
        );
        baseDocumentRef.current = nextDocument;
        saveController.queue(nextDocument);
      },
      [saveController]
    );

    const setCanonicalPendingTaskIds = useCallback(
      (requestedTaskIds: readonly string[]) => {
        const currentBase = baseDocumentRef.current;
        if (!currentBase) throw new Error('Creative canvas document is not hydrated');
        const nextTaskIds = canonicalCreativePendingTaskIds(requestedTaskIds);
        if (
          nextTaskIds.length === currentBase.pendingTaskIds.length &&
          nextTaskIds.every((taskId, index) => taskId === currentBase.pendingTaskIds[index])
        ) {
          return;
        }

        const nextDocument = projectDocumentWithPendingTaskIds(
          currentBase,
          stateRef.current,
          nextTaskIds
        );
        baseDocumentRef.current = nextDocument;
        pendingTaskIdsRef.current = nextTaskIds;
        setPendingTaskIdsState(nextTaskIds);
        saveController.queue(nextDocument);
      },
      [saveController]
    );

    const addPendingTask = useCallback(
      async (taskId: string) => {
        setCanonicalPendingTaskIds([
          ...pendingTaskIdsRef.current,
          taskId,
        ]);
        const result = await saveController.flush();
        if (result.status === 'conflict' || result.status === 'error') {
          throw result.error;
        }
      },
      [saveController, setCanonicalPendingTaskIds]
    );

    const removePendingTask = useCallback(
      async (taskId: string) => {
        const [canonicalTaskId] = canonicalCreativePendingTaskIds([taskId]);
        setCanonicalPendingTaskIds(
          pendingTaskIdsRef.current.filter((candidate) => candidate !== canonicalTaskId)
        );
        const result = await saveController.flush();
        if (result.status === 'conflict' || result.status === 'error') {
          throw result.error;
        }
      },
      [saveController, setCanonicalPendingTaskIds]
    );

    useEffect(() => onStateChange?.(state), [onStateChange, state]);
    useEffect(() => onSaveStateChange?.(saveSnapshot), [onSaveStateChange, saveSnapshot]);
    useEffect(() => {
      if (pendingTaskIds !== null) onPendingTaskIdsChange?.([...pendingTaskIds]);
    }, [onPendingTaskIdsChange, pendingTaskIds]);

    useEffect(() => {
      const beforeUnload = () => {
        if (saveController.getSnapshot().hasPendingChanges) void saveController.flush();
      };
      window.addEventListener('beforeunload', beforeUnload);
      return () => window.removeEventListener('beforeunload', beforeUnload);
    }, [saveController]);

    useImperativeHandle(
      ref,
      () => ({
        dispatch: applyCommand,
        setBackground,
        setPanels,
        addPendingTask,
        removePendingTask,
        flush: () => saveController.flush(),
        reloadRemote,
        getState: () => stateRef.current,
        getSaveState: () => saveController.getSnapshot(),
        getPendingTaskIds: () => [...pendingTaskIdsRef.current],
      }),
      [
        addPendingTask,
        applyCommand,
        reloadRemote,
        removePendingTask,
        saveController,
        setBackground,
        setPanels,
      ]
    );

    const localClientPoint = useCallback((clientX: number, clientY: number) => {
      const rect = surfaceRef.current?.getBoundingClientRect();
      return { x: clientX - (rect?.left ?? 0), y: clientY - (rect?.top ?? 0) };
    }, []);

    const capturePointer = useCallback((pointerId: number) => {
      const surface = surfaceRef.current;
      if (surface && !surface.hasPointerCapture(pointerId)) surface.setPointerCapture(pointerId);
    }, []);

    const releasePointer = useCallback((pointerId: number) => {
      const surface = surfaceRef.current;
      if (surface?.hasPointerCapture(pointerId)) surface.releasePointerCapture(pointerId);
    }, []);

    const beginNodePointer = useCallback(
      (node: CreativeCanvasNode, event: React.PointerEvent<HTMLElement>) => {
        if (tool === 'pan') return;
        if (event.button !== 0) return;
        event.preventDefault();
        event.stopPropagation();
        surfaceRef.current?.focus({ preventScroll: true });

        const additive = event.shiftKey || event.ctrlKey || event.metaKey;
        const currentlySelected = stateRef.current.selection.nodeIds.includes(node.id);
        if (additive) applyCommand(canvasCommands.toggleNodeSelection(node.id));
        else if (!currentlySelected) applyCommand(canvasCommands.setSelection([node.id]));

        if (!stateRef.current.selection.nodeIds.includes(node.id) || node.locked) return;
        gestureSequenceRef.current += 1;
        const client = { x: event.clientX, y: event.clientY };
        setInteraction({
          type: 'gesture/start',
          gesture: {
            kind: 'move',
            pointerId: event.pointerId,
            lastClient: client,
            mergeKey: `move:${projectId}:${gestureSequenceRef.current}`,
          },
        });
        capturePointer(event.pointerId);
      },
      [applyCommand, capturePointer, projectId, setInteraction, tool]
    );

    const handleSurfacePointerDown = useCallback(
      (event: React.PointerEvent<HTMLDivElement>) => {
        const shouldPan = event.button === 1 || (event.button === 0 && tool === 'pan');
        if (!shouldPan && (event.button !== 0 || tool !== 'select')) return;
        event.preventDefault();
        event.currentTarget.focus({ preventScroll: true });
        const client = { x: event.clientX, y: event.clientY };

        if (shouldPan) {
          setInteraction({
            type: 'gesture/start',
            gesture: { kind: 'pan', pointerId: event.pointerId, lastClient: client },
          });
        } else {
          const world = clientToCanvas(
            localClientPoint(event.clientX, event.clientY),
            stateRef.current.viewport
          );
          applyCommand(
            canvasCommands.startBoxSelection(
              world,
              event.shiftKey || event.ctrlKey || event.metaKey ? 'add' : 'replace'
            )
          );
          setInteraction({
            type: 'gesture/start',
            gesture: { kind: 'select', pointerId: event.pointerId, lastClient: client },
          });
        }
        capturePointer(event.pointerId);
      },
      [applyCommand, capturePointer, localClientPoint, setInteraction, tool]
    );

    const handleSurfacePointerMove = useCallback(
      (event: React.PointerEvent<HTMLDivElement>) => {
        const gesture = interactionRef.current.gesture;
        if (!gesture || gesture.pointerId !== event.pointerId) return;
        const client = { x: event.clientX, y: event.clientY };
        const dx = client.x - gesture.lastClient.x;
        const dy = client.y - gesture.lastClient.y;

        if (gesture.kind === 'pan') {
          applyCommand(canvasCommands.panViewport({ x: dx, y: dy }));
        } else if (gesture.kind === 'move') {
          const zoom = stateRef.current.viewport.zoom;
          applyCommand(
            canvasCommands.moveNodes(
              { x: dx / zoom, y: dy / zoom },
              { at: event.timeStamp, mergeKey: gesture.mergeKey }
            )
          );
        } else {
          const world = clientToCanvas(
            localClientPoint(event.clientX, event.clientY),
            stateRef.current.viewport
          );
          applyCommand(canvasCommands.updateBoxSelection(world));
        }
        setInteraction({ type: 'gesture/update', pointerId: event.pointerId, client });
      },
      [applyCommand, localClientPoint, setInteraction]
    );

    const finishPointer = useCallback(
      (pointerId: number) => {
        const gesture = interactionRef.current.gesture;
        if (!gesture || gesture.pointerId !== pointerId) return;
        if (gesture.kind === 'select') applyCommand(canvasCommands.endBoxSelection());
        releasePointer(pointerId);
        setInteraction({ type: 'gesture/end', pointerId });
      },
      [applyCommand, releasePointer, setInteraction]
    );

    const handleWheel = useCallback(
      (event: React.WheelEvent<HTMLDivElement>) => {
        event.preventDefault();
        const factor = Math.exp(-event.deltaY * 0.0015);
        applyCommand(
          canvasCommands.zoomViewportAt(
            stateRef.current.viewport.zoom * factor,
            localClientPoint(event.clientX, event.clientY)
          )
        );
      },
      [applyCommand, localClientPoint]
    );

    const zoomAtCenter = useCallback(
      (zoom: number) => {
        const rect = surfaceRef.current?.getBoundingClientRect();
        applyCommand(
          canvasCommands.zoomViewportAt(zoom, {
            x: (rect?.width ?? 0) / 2,
            y: (rect?.height ?? 0) / 2,
          })
        );
      },
      [applyCommand]
    );

    const resetView = useCallback(() => {
      const rect = surfaceRef.current?.getBoundingClientRect();
      applyCommand(
        canvasCommands.setViewport({
          x: (rect?.width ?? 0) / 2,
          y: (rect?.height ?? 0) / 2,
          zoom: 1,
        })
      );
    }, [applyCommand]);

    const fitView = useCallback(() => {
      const rect = surfaceRef.current?.getBoundingClientRect();
      applyCommand(
        canvasCommands.setViewport(
          fitCanvasViewport(stateRef.current, {
            width: rect?.width ?? 1,
            height: rect?.height ?? 1,
          })
        )
      );
    }, [applyCommand]);

    const handleKeyDown = useCallback(
      (event: React.KeyboardEvent<HTMLDivElement>) => {
        if (isCanvasKeyboardTarget(event.target)) return;
        const modifier = event.ctrlKey || event.metaKey;
        const key = event.key.toLowerCase();

        if (modifier && key === 'c') {
          event.preventDefault();
          pasteSequenceRef.current = 0;
          applyCommand(canvasCommands.copySelection());
          return;
        }
        if (modifier && key === 'v') {
          const pasteIndex = pasteSequenceRef.current + 1;
          const paste = canvasCommands.pasteClipboard(stateRef.current, {
            offset: { x: 32 * pasteIndex, y: 32 * pasteIndex },
          });
          if (paste) {
            event.preventDefault();
            pasteSequenceRef.current = pasteIndex;
            applyCommand(paste);
          }
          return;
        }
        if (modifier && (key === 'z' || key === 'y')) {
          event.preventDefault();
          applyCommand(
            key === 'y' || event.shiftKey ? canvasCommands.redo() : canvasCommands.undo()
          );
          return;
        }
        if (event.key === 'Delete' || event.key === 'Backspace') {
          event.preventDefault();
          applyCommand(canvasCommands.deleteSelection());
          return;
        }
        if (event.key === 'Escape') {
          applyCommand(canvasCommands.clearSelection());
        }
      },
      [applyCommand]
    );

    const loadState = classifyCreativeCanvasLoadState({
      projectId,
      detail: project.detail,
      isLoading: project.isLoading,
      error: project.error,
    });
    if (loadState === 'loading') return <>{renderLoading()}</>;
    if (loadState === 'not-found') return <>{renderNotFound(projectId)}</>;
    if (loadState === 'error' && project.error) {
      return <>{renderError(project.error, project.refresh)}</>;
    }

    const baseDocument = baseDocumentRef.current ?? project.detail?.document;
    if (!baseDocument) return <>{renderLoading()}</>;

    const flush = () => saveController.flush();
    const context: CreativeCanvasEditorContext = {
      state,
      save: saveSnapshot,
      tool,
      pendingTaskIds: pendingTaskIds ?? baseDocument.pendingTaskIds,
      flush,
      reloadRemote,
    };
    const nodeById = new Map(state.document.nodes.map((node) => [node.id, node]));
    const selectedNodeIds = new Set(state.selection.nodeIds);
    const selectedEdgeIds = new Set(state.selection.edgeIds);
    const nodeLayer = state.document.nodes.map((node) => {
      const onPointerDown: React.PointerEventHandler<HTMLElement> = (event) =>
        beginNodePointer(node, event);
      return (
        <div
          key={node.id}
          className={styles.nodePlacement}
          style={{
            left: node.position.x,
            top: node.position.y,
            width: node.size.width,
            height: node.size.height,
            zIndex: node.zIndex,
          }}
          data-canvas-node-id={node.id}
          data-canvas-node-kind={node.type}
          data-selected={selectedNodeIds.has(node.id) || undefined}
          onPointerDown={onPointerDown}
        >
          {renderNode({
            node,
            selected: selectedNodeIds.has(node.id),
            onActivate: () => applyCommand(canvasCommands.setSelection([node.id])),
            dragHandleProps: { onPointerDown },
          })}
        </div>
      );
    });
    const edgeLayer = state.document.connections.flatMap((connection) => {
      const source = nodeById.get(connection.sourceNodeId);
      const target = nodeById.get(connection.targetNodeId);
      if (!source || !target) return [];
      return [
        <React.Fragment key={connection.id}>
          {renderEdge({
            connection,
            source,
            target,
            selected: selectedEdgeIds.has(connection.id),
            onActivate: () => applyCommand(canvasCommands.setSelection([], [connection.id])),
          })}
        </React.Fragment>,
      ];
    });

    const saveChrome = saveSnapshot.status === 'idle' ? null : (
      <div
        className={styles.saveState}
        data-canvas-save-status={saveSnapshot.status}
        role={saveSnapshot.status === 'conflict' || saveSnapshot.status === 'error' ? 'alert' : 'status'}
        aria-live='polite'
      >
        {saveSnapshot.status === 'dirty' ? '等待保存' : null}
        {saveSnapshot.status === 'saving' ? '正在保存' : null}
        {saveSnapshot.status === 'saved' ? '已保存' : null}
        {saveSnapshot.status === 'conflict' ? (
          <>
            <span>远端版本已更新，本地更改未覆盖远端。</span>
            <button type='button' onClick={() => void reloadRemote()}>
              放弃本地更改并重新载入
            </button>
          </>
        ) : null}
        {saveSnapshot.status === 'error' ? (
          <>
            <span>{saveSnapshot.error?.message ?? '保存失败'}</span>
            <button type='button' onClick={() => void saveController.flush()}>
              重试保存
            </button>
          </>
        ) : null}
      </div>
    );
    const resolvedTopDock = resolveSlot(topDock, context);

    return (
      <CanvasSurface
        ref={surfaceRef}
        className={`${styles.editor} ${className ?? ''}`.trim()}
        viewport={state.viewport}
        backgroundMode={canvasSurfaceBackground(background)}
        tool={tool}
        isPanning={interaction.isPanning}
        ariaLabel={ariaLabel}
        tabIndex={0}
        data-creative-canvas-editor
        data-editor-save-state={saveSnapshot.status}
        nodeLayer={nodeLayer}
        edgeLayer={edgeLayer}
        selectionRect={state.selection.marquee}
        topDock={
          saveChrome || resolvedTopDock ? (
            <div className={styles.topDockStack}>
              {resolvedTopDock}
              {saveChrome}
            </div>
          ) : undefined
        }
        leftDock={resolveSlot(leftPanel, context)}
        rightDock={resolveSlot(rightPanel, context)}
        bottomDock={resolveSlot(bottomPanel, context)}
        screenOverlay={resolveSlot(screenOverlay, context)}
        miniMap={resolveSlot(miniMap, context)}
        isMiniMapOpen={isMiniMapOpen}
        zoomControls={
          showZoomControls
            ? {
                minZoom: 0.05,
                maxZoom: 5,
                onZoomChange: zoomAtCenter,
                onResetView: resetView,
                onFitView: fitView,
                onToggleMiniMap,
              }
            : false
        }
        onPointerDown={handleSurfacePointerDown}
        onPointerMove={handleSurfacePointerMove}
        onPointerUp={(event) => finishPointer(event.pointerId)}
        onPointerCancel={(event) => finishPointer(event.pointerId)}
        onLostPointerCapture={(event) => finishPointer(event.pointerId)}
        onWheel={handleWheel}
        onKeyDown={handleKeyDown}
      />
    );
  }
);

CreativeCanvasEditor.displayName = 'CreativeCanvasEditor';

export default CreativeCanvasEditor;
