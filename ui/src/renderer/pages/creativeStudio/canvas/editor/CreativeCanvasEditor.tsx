/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import React, {
  useCallback,
  useEffect,
  useImperativeHandle,
  useMemo,
  useReducer,
  useRef,
  useState,
} from 'react';
import { useTranslation } from 'react-i18next';

import type {
  CreativeCanvasBackground,
  CreativeCanvasConnection,
  CreativeCanvasNode,
  CreativeChatSessionReference,
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
  type CanvasPoint,
  type CanvasState,
} from '../core';
import {
  CanvasSurface,
  type CanvasInteractionTool,
} from '../components';
import {
  deriveCanvasGraphHighlight,
  finishCanvasConnectionDrag,
  finishCanvasResize,
  openCanvasContextMenu,
  resolveCanvasContextAction,
  resolveCanvasDoubleClick,
  resolveCanvasKeyboardInput,
  startCanvasConnectionDrag,
  startCanvasResize,
  updateCanvasConnectionDrag,
  updateCanvasResize,
  validateCanvasDropImport,
  type CanvasConnectionDragGesture,
  type CanvasConnectionHandleKind,
  type CanvasIntegrationIntent,
  type CanvasInteractionResolution,
  type CanvasResizeCorner,
} from '../interactions';
import {
  canvasSaveRequiresUnloadGuard,
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
  projectDocumentWithAgentSessions,
  projectDocumentWithCanvasPanels,
  projectDocumentWithPendingTaskIds,
  shouldHydrateCreativeCanvasDetail,
} from './editorModel';
import {
  canvasEditorInteractionReducer,
  INITIAL_CANVAS_EDITOR_INTERACTION,
  type CanvasEditorInteractionAction,
  type CanvasEditorInteractionState,
} from './interactionReducer';
import { pendingTaskCommandGuard } from './pendingTaskGuard';
import { useCanvasCasSave } from './useCanvasCasSave';
import styles from './CreativeCanvasEditor.module.css';

export interface CreativeCanvasNodeRenderContext {
  node: CreativeCanvasNode;
  selected: boolean;
  highlighted: boolean;
  dimmed: boolean;
  onActivate(): void;
  onOpen(): void;
  onToggleLock(): void;
  dragHandleProps: {
    onPointerDown: React.PointerEventHandler<HTMLElement>;
  };
}

export interface CreativeCanvasEdgeRenderContext {
  connection: CreativeCanvasConnection;
  source: CreativeCanvasNode;
  target: CreativeCanvasNode;
  selected: boolean;
  highlighted: boolean;
  dimmed: boolean;
  onActivate(): void;
  onContextMenu: React.MouseEventHandler<SVGElement>;
}

export interface CreativeCanvasEditorContext {
  state: CanvasState;
  save: CanvasCasSaveSnapshot;
  tool: CanvasInteractionTool;
  /** Authoritative task ids persisted in the canonical Canvas document. */
  pendingTaskIds: readonly string[];
  /** Canonical NomiFun Agent session references persisted with the project. */
  agentSessions: readonly CreativeChatSessionReference[];
  activeAgentSessionId: string | null;
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
  /** Validate and durably persist Agent references before or after a transport turn. */
  persistAgentSessions(
    sessions: readonly CreativeChatSessionReference[],
    activeSessionId: string | null
  ): Promise<void>;
  /** Route guards must await this before leaving the editor. */
  flush(): Promise<CanvasCasFlushResult>;
  /** Explicitly discard local state and reload the authoritative remote revision. */
  reloadRemote(): Promise<boolean>;
  getState(): CanvasState;
  getSaveState(): CanvasCasSaveSnapshot;
  getPendingTaskIds(): readonly string[];
  getAgentSessions(): readonly CreativeChatSessionReference[];
  getActiveAgentSessionId(): string | null;
}

export interface CreativeCanvasEditorProps {
  projectId: string;
  tool: CanvasInteractionTool;
  /** Freeze every local mutation while an external CAS writer owns the project. */
  disabled?: boolean;
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
  worldOverlay?: CreativeCanvasEditorSlot;
  screenOverlay?: CreativeCanvasEditorSlot;
  miniMap?: CreativeCanvasEditorSlot;
  isMiniMapOpen?: boolean;
  onToggleMiniMap?: () => void;
  showZoomControls?: boolean;
  /** Keep the generic in-canvas save banner when no product chrome owns it. */
  showSaveState?: boolean;
  renderLoading?: () => React.ReactNode;
  renderNotFound?: (projectId: string) => React.ReactNode;
  renderError?: (error: Error, retry: () => Promise<CreativeProjectDetail | undefined>) => React.ReactNode;
  onStateChange?: (state: CanvasState) => void;
  onSaveStateChange?: (save: CanvasCasSaveSnapshot) => void;
  onIntegrationIntent?: (intent: CanvasIntegrationIntent) => void | Promise<void>;
  /** Fires after hydration and after each canonical task-feed mutation. */
  onPendingTaskIdsChange?: (taskIds: readonly string[]) => void;
  /** Reports a command rejected because it would orphan a durable pending task. */
  onPendingTaskCommandBlocked?: (taskIds: readonly string[]) => void;
  /** Fires after hydration and each durable Agent reference mutation. */
  onAgentSessionsChange?: (
    sessions: readonly CreativeChatSessionReference[],
    activeSessionId: string | null
  ) => void;
}

const resolveSlot = (
  slot: CreativeCanvasEditorSlot | undefined,
  context: CreativeCanvasEditorContext
): React.ReactNode => (typeof slot === 'function' ? slot(context) : slot);

const INTERACTION_SAVE_IDLE_MS = 160;

/**
 * Pointer and wheel updates should stay on the visual path. Persisting every
 * intermediate viewport/node position clones and serializes the whole canvas,
 * so these commands are coalesced until the interaction is idle or flushed.
 */
const shouldDeferPersistence = (command: CanvasCommand): boolean =>
  command.type === 'viewport/pan' ||
  command.type === 'viewport/set' ||
  command.type === 'viewport/zoom-at' ||
  command.type === 'node/move' ||
  command.type === 'node/update';

const defaultLoading = (label: string) => (
  <div className={styles.centerState} data-creative-canvas-state='loading' role='status'>
    {label}
  </div>
);

const defaultNotFound = (projectId: string, title: string) => (
  <div className={styles.centerState} data-creative-canvas-state='not-found' role='status'>
    <strong>{title}</strong>
    <span>{projectId}</span>
  </div>
);

const defaultError = (
  error: Error,
  retry: () => Promise<CreativeProjectDetail | undefined>,
  title: string,
  retryLabel: string
) => (
  <div className={styles.centerState} data-creative-canvas-state='error' role='alert'>
    <strong>{title}</strong>
    <span>{error.message}</span>
    <button type='button' onClick={() => void retry()}>
      {retryLabel}
    </button>
  </div>
);

const connectionAnchor = (
  node: CreativeCanvasNode,
  handle: CanvasConnectionHandleKind
): CanvasPoint => ({
  x: handle === 'source' ? node.position.x + node.size.width : node.position.x,
  y: node.position.y + node.size.height / 2,
});

const connectionPreviewPath = (
  state: CanvasState,
  gesture: CanvasConnectionDragGesture
): string | null => {
  const fixed = state.document.nodes.find((node) => node.id === gesture.fixedNodeId);
  if (!fixed) return null;
  const fixedPoint = connectionAnchor(fixed, gesture.fixedHandle);
  const source = gesture.fixedHandle === 'source' ? fixedPoint : gesture.worldPosition;
  const target = gesture.fixedHandle === 'target' ? fixedPoint : gesture.worldPosition;
  const control = Math.max(40, Math.abs(target.x - source.x) * 0.45);
  return `M ${source.x} ${source.y} C ${source.x + control} ${source.y}, ${target.x - control} ${target.y}, ${target.x} ${target.y}`;
};

const hasDraggedFiles = (dataTransfer: DataTransfer): boolean =>
  Array.from(dataTransfer.types).includes('Files');

const RESIZE_CORNERS: readonly CanvasResizeCorner[] = [
  'top-left',
  'top-right',
  'bottom-left',
  'bottom-right',
];

const RESIZE_CORNER_LABEL_KEYS: Record<CanvasResizeCorner, string> = {
  'top-left': 'creativeStudio.canvas.resizeCorners.topLeft',
  'top-right': 'creativeStudio.canvas.resizeCorners.topRight',
  'bottom-left': 'creativeStudio.canvas.resizeCorners.bottomLeft',
  'bottom-right': 'creativeStudio.canvas.resizeCorners.bottomRight',
};

const CreativeCanvasEditor = React.forwardRef<CreativeCanvasEditorHandle, CreativeCanvasEditorProps>(
  (
    {
      projectId,
      tool,
      disabled = false,
      renderNode,
      renderEdge,
      repository,
      saveDebounceMs,
      ariaLabel,
      className,
      topDock,
      leftPanel,
      rightPanel,
      bottomPanel,
      worldOverlay,
      screenOverlay,
      miniMap,
      isMiniMapOpen = false,
      onToggleMiniMap,
      showZoomControls = true,
      showSaveState = true,
      renderLoading,
      renderNotFound,
      renderError,
      onStateChange,
      onSaveStateChange,
      onIntegrationIntent,
      onPendingTaskIdsChange,
      onPendingTaskCommandBlocked,
      onAgentSessionsChange,
    },
    ref
  ) => {
    const { t } = useTranslation();
    const project = useCreativeProject(projectId, repository);
    const { controller: saveController, snapshot: saveSnapshot } = useCanvasCasSave(
      project.save,
      saveDebounceMs,
      projectId
    );
    const [state, setState] = useState<CanvasState>(() => createInitialCanvasState());
    const [background, setBackgroundState] = useState<CreativeCanvasBackground>('lines');
    const [pendingTaskIds, setPendingTaskIdsState] = useState<readonly string[] | null>(null);
    const [agentSessions, setAgentSessionsState] = useState<
      readonly CreativeChatSessionReference[] | null
    >(null);
    const [activeAgentSessionId, setActiveAgentSessionId] = useState<string | null>(null);
    const stateRef = useRef(state);
    const pendingTaskIdsRef = useRef<readonly string[]>([]);
    const agentSessionsRef = useRef<readonly CreativeChatSessionReference[]>([]);
    const activeAgentSessionIdRef = useRef<string | null>(null);
    const baseDocumentRef = useRef<CreativeProjectDetail['document'] | null>(null);
    const loadedProjectIdRef = useRef<string | null>(null);
    const loadedRevisionRef = useRef<string | null>(null);
    const hydratedSaveControllerRef = useRef<typeof saveController | null>(null);
    const surfaceRef = useRef<HTMLDivElement>(null);
    const pasteSequenceRef = useRef(0);
    const gestureSequenceRef = useRef(0);
    const persistenceTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null);
    const [interaction, dispatchInteraction] = useReducer(
      canvasEditorInteractionReducer,
      INITIAL_CANVAS_EDITOR_INTERACTION
    );
    const interactionRef = useRef<CanvasEditorInteractionState>(interaction);

    const setInteraction = useCallback((action: CanvasEditorInteractionAction) => {
      interactionRef.current = canvasEditorInteractionReducer(interactionRef.current, action);
      dispatchInteraction(action);
    }, []);

    const cancelScheduledPersistence = useCallback(() => {
      if (persistenceTimerRef.current === null) return;
      clearTimeout(persistenceTimerRef.current);
      persistenceTimerRef.current = null;
    }, []);

    const queueLatestPersistence = useCallback(() => {
      const base = baseDocumentRef.current;
      if (!base) return;
      saveController.queue(projectDocumentFromCanvasState(base, stateRef.current));
    }, [saveController]);

    const schedulePersistence = useCallback(() => {
      cancelScheduledPersistence();
      persistenceTimerRef.current = setTimeout(() => {
        persistenceTimerRef.current = null;
        queueLatestPersistence();
      }, INTERACTION_SAVE_IDLE_MS);
    }, [cancelScheduledPersistence, queueLatestPersistence]);

    const flushScheduledPersistence = useCallback(() => {
      if (persistenceTimerRef.current === null) return;
      cancelScheduledPersistence();
      queueLatestPersistence();
    }, [cancelScheduledPersistence, queueLatestPersistence]);

    useEffect(
      () => () => {
        cancelScheduledPersistence();
      },
      [cancelScheduledPersistence]
    );

    const hydrate = useCallback(
      (detail: CreativeProjectDetail) => {
        cancelScheduledPersistence();
        const next = canvasStateFromProjectDocument(detail.document);
        baseDocumentRef.current = structuredClone(detail.document);
        loadedProjectIdRef.current = detail.project.projectId;
        loadedRevisionRef.current = detail.project.revision;
        hydratedSaveControllerRef.current = saveController;
        stateRef.current = next;
        setState(next);
        setBackgroundState(detail.document.background);
        pendingTaskIdsRef.current = [...detail.document.pendingTaskIds];
        setPendingTaskIdsState(pendingTaskIdsRef.current);
        agentSessionsRef.current = structuredClone(detail.document.chatSessions);
        activeAgentSessionIdRef.current = detail.document.activeChatId;
        setAgentSessionsState(agentSessionsRef.current);
        setActiveAgentSessionId(activeAgentSessionIdRef.current);
        saveController.reset(detail.project.revision, detail.document);
        pasteSequenceRef.current = 0;
        setInteraction({ type: 'gesture/end' });
      },
      [cancelScheduledPersistence, saveController, setInteraction]
    );

    useEffect(() => {
      cancelScheduledPersistence();
      loadedProjectIdRef.current = null;
      loadedRevisionRef.current = null;
      baseDocumentRef.current = null;
      hydratedSaveControllerRef.current = null;
      pendingTaskIdsRef.current = [];
      setPendingTaskIdsState(null);
      agentSessionsRef.current = [];
      activeAgentSessionIdRef.current = null;
      setAgentSessionsState(null);
      setActiveAgentSessionId(null);
    }, [cancelScheduledPersistence, projectId]);

    useEffect(() => {
      const detail = project.detail;
      if (hydratedSaveControllerRef.current !== saveController) {
        loadedProjectIdRef.current = null;
        loadedRevisionRef.current = null;
      }
      if (
        !detail ||
        !shouldHydrateCreativeCanvasDetail({
          projectId,
          loadedProjectId: loadedProjectIdRef.current,
          loadedRevision: loadedRevisionRef.current,
          detail,
          save: saveSnapshot,
        })
      ) return;
      hydrate(detail);
    }, [hydrate, project.detail, projectId, saveController, saveSnapshot]);

    const reloadRemote = useCallback(async (): Promise<boolean> => {
      const detail = await project.refresh();
      if (!detail || detail.project.projectId !== projectId) return false;
      hydrate(detail);
      return true;
    }, [hydrate, project, projectId]);

    const flushCanvasPersistence = useCallback(async (): Promise<CanvasCasFlushResult> => {
      flushScheduledPersistence();
      return saveController.flush();
    }, [flushScheduledPersistence, saveController]);

    const applyCommand = useCallback(
      (command: CanvasCommand): CanvasState => {
        const current = stateRef.current;
        if (disabled) return current;
        const guard = pendingTaskCommandGuard(current, command, pendingTaskIdsRef.current);
        if (!guard.allowed) {
          onPendingTaskCommandBlocked?.(guard.orphanedTaskIds);
          return current;
        }
        const next = canvasReducer(current, command);
        if (next === current) return current;
        stateRef.current = next;
        setState(next);

        const persistedChanged =
          next.document !== current.document || next.viewport !== current.viewport;
        const base = baseDocumentRef.current;
        if (persistedChanged && base) {
          if (shouldDeferPersistence(command)) {
            schedulePersistence();
          } else {
            cancelScheduledPersistence();
            saveController.queue(projectDocumentFromCanvasState(base, next));
          }
        }
        return next;
      },
      [
        cancelScheduledPersistence,
        disabled,
        onPendingTaskCommandBlocked,
        saveController,
        schedulePersistence,
      ]
    );

    const applyInteractionResolution = useCallback(
      (resolution: CanvasInteractionResolution) => {
        if (disabled) return;
        if (!resolution.handled) return;
        for (const command of resolution.commands) applyCommand(command);
        if (onIntegrationIntent && resolution.intents.length > 0) {
          void (async () => {
            for (const intent of resolution.intents) {
              await onIntegrationIntent(intent);
            }
          })();
        }
      },
      [applyCommand, disabled, onIntegrationIntent]
    );

    const setBackground = useCallback(
      (nextBackground: CreativeCanvasBackground) => {
        if (disabled) return;
        const currentBase = baseDocumentRef.current;
        if (!currentBase || currentBase.background === nextBackground) return;

        cancelScheduledPersistence();
        const nextBase = {
          ...structuredClone(currentBase),
          background: nextBackground,
        };
        baseDocumentRef.current = nextBase;
        setBackgroundState(nextBackground);
        saveController.queue(projectDocumentFromCanvasState(nextBase, stateRef.current));
      },
      [cancelScheduledPersistence, disabled, saveController]
    );

    const setPanels = useCallback(
      (nextPanels: CreativeStudioPanelState) => {
        if (disabled) return;
        const currentBase = baseDocumentRef.current;
        if (!currentBase || creativeStudioPanelStateEqual(currentBase.panels, nextPanels)) {
          return;
        }

        cancelScheduledPersistence();
        const nextDocument = projectDocumentWithCanvasPanels(
          currentBase,
          stateRef.current,
          nextPanels
        );
        baseDocumentRef.current = nextDocument;
        saveController.queue(nextDocument);
      },
      [cancelScheduledPersistence, disabled, saveController]
    );

    const setCanonicalPendingTaskIds = useCallback(
      (requestedTaskIds: readonly string[]) => {
        if (disabled) throw new Error('Creative canvas is read-only');
        const currentBase = baseDocumentRef.current;
        if (!currentBase) throw new Error('Creative canvas document is not hydrated');
        const nextTaskIds = canonicalCreativePendingTaskIds(requestedTaskIds);
        if (
          nextTaskIds.length === currentBase.pendingTaskIds.length &&
          nextTaskIds.every((taskId, index) => taskId === currentBase.pendingTaskIds[index])
        ) {
          return;
        }

        cancelScheduledPersistence();
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
      [cancelScheduledPersistence, disabled, saveController]
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

    const persistAgentSessions = useCallback(
      async (
        requestedSessions: readonly CreativeChatSessionReference[],
        requestedActiveSessionId: string | null
      ) => {
        if (disabled) throw new Error('Creative canvas is read-only');
        const currentBase = baseDocumentRef.current;
        if (!currentBase) throw new Error('Creative canvas document is not hydrated');
        const nextDocument = projectDocumentWithAgentSessions(
          currentBase,
          stateRef.current,
          requestedSessions,
          requestedActiveSessionId
        );
        baseDocumentRef.current = nextDocument;
        agentSessionsRef.current = structuredClone(nextDocument.chatSessions);
        activeAgentSessionIdRef.current = nextDocument.activeChatId;
        setAgentSessionsState(agentSessionsRef.current);
        setActiveAgentSessionId(activeAgentSessionIdRef.current);
        cancelScheduledPersistence();
        saveController.queue(nextDocument);
        const result = await saveController.flush();
        if (result.status === 'conflict' || result.status === 'error') {
          throw result.error;
        }
      },
      [cancelScheduledPersistence, disabled, saveController]
    );

    useEffect(() => onStateChange?.(state), [onStateChange, state]);
    useEffect(() => onSaveStateChange?.(saveSnapshot), [onSaveStateChange, saveSnapshot]);
    useEffect(() => {
      if (pendingTaskIds !== null) onPendingTaskIdsChange?.([...pendingTaskIds]);
    }, [onPendingTaskIdsChange, pendingTaskIds]);
    useEffect(() => {
      if (agentSessions !== null) {
        onAgentSessionsChange?.(structuredClone(agentSessions), activeAgentSessionId);
      }
    }, [activeAgentSessionId, agentSessions, onAgentSessionsChange]);

    useEffect(() => {
      const beforeUnload = (event: BeforeUnloadEvent) => {
        flushScheduledPersistence();
        if (!canvasSaveRequiresUnloadGuard(saveController.getSnapshot())) return;
        void saveController.flush();
        event.preventDefault();
        event.returnValue = '';
      };
      window.addEventListener('beforeunload', beforeUnload);
      return () => window.removeEventListener('beforeunload', beforeUnload);
    }, [flushScheduledPersistence, saveController]);

    useImperativeHandle(
      ref,
      () => ({
        dispatch: applyCommand,
        setBackground,
        setPanels,
        addPendingTask,
        removePendingTask,
        persistAgentSessions,
        flush: flushCanvasPersistence,
        reloadRemote,
        getState: () => stateRef.current,
        getSaveState: () => saveController.getSnapshot(),
        getPendingTaskIds: () => [...pendingTaskIdsRef.current],
        getAgentSessions: () => structuredClone(agentSessionsRef.current),
        getActiveAgentSessionId: () => activeAgentSessionIdRef.current,
      }),
      [
        addPendingTask,
        applyCommand,
        flushCanvasPersistence,
        persistAgentSessions,
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
      if (!surface || surface.hasPointerCapture(pointerId)) return;
      try {
        surface.setPointerCapture(pointerId);
      } catch {
        // The pointer may have been canceled between the child handler and capture.
      }
    }, []);

    const releasePointer = useCallback((pointerId: number) => {
      const surface = surfaceRef.current;
      if (surface?.hasPointerCapture(pointerId)) surface.releasePointerCapture(pointerId);
    }, []);

    useEffect(() => {
      if (!disabled) return;
      setInteraction({ type: 'gesture/end' });
      surfaceRef.current?.blur();
    }, [disabled, setInteraction]);

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

    const beginNodeResize = useCallback(
      (
        node: CreativeCanvasNode,
        corner: CanvasResizeCorner,
        event: React.PointerEvent<HTMLButtonElement>
      ) => {
        if (tool !== 'select' || event.button !== 0) return;
        event.preventDefault();
        event.stopPropagation();
        const started = startCanvasResize(
          node,
          event.pointerId,
          { x: event.clientX, y: event.clientY },
          corner,
          stateRef.current.viewport,
          {
            keepAspectRatio: event.shiftKey,
          }
        );
        if (!started.ok) return;
        gestureSequenceRef.current += 1;
        applyCommand(canvasCommands.setSelection([node.id]));
        setInteraction({
          type: 'gesture/start',
          gesture: {
            ...started.gesture,
            mergeKey: `resize:${projectId}:${node.id}:${gestureSequenceRef.current}`,
          },
        });
        capturePointer(event.pointerId);
      },
      [applyCommand, capturePointer, projectId, setInteraction, tool]
    );

    const beginConnectionDrag = useCallback(
      (
        node: CreativeCanvasNode,
        handle: CanvasConnectionHandleKind,
        event: React.PointerEvent<HTMLButtonElement>
      ) => {
        if (tool !== 'select' || event.button !== 0) return;
        event.preventDefault();
        event.stopPropagation();
        const started = startCanvasConnectionDrag(stateRef.current.document, {
          nodeId: node.id,
          handle,
          handleId: handle,
          pointerId: event.pointerId,
          clientPosition: localClientPoint(event.clientX, event.clientY),
          viewport: stateRef.current.viewport,
        });
        if (!started.ok) return;
        applyCommand(canvasCommands.setSelection([node.id]));
        setInteraction({ type: 'gesture/start', gesture: started.gesture });
        capturePointer(event.pointerId);
      },
      [applyCommand, capturePointer, localClientPoint, setInteraction, tool]
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

        if (gesture.kind === 'resize') {
          const update = updateCanvasResize(gesture, event.pointerId, client, event.timeStamp);
          if (update.command) applyCommand(update.command);
          return;
        }
        if (gesture.kind === 'connection') {
          const next = updateCanvasConnectionDrag(
            gesture,
            event.pointerId,
            localClientPoint(event.clientX, event.clientY),
            stateRef.current.viewport
          );
          if (next !== gesture) {
            setInteraction({ type: 'gesture/replace', gesture: next });
          }
          return;
        }

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

    const connectionDropTarget = useCallback(
      (gesture: CanvasConnectionDragGesture, clientX: number, clientY: number) => {
        const element = document.elementFromPoint(clientX, clientY);
        const handleElement = element?.closest<HTMLElement>('[data-canvas-connection-handle]');
        const nodeElement = element?.closest<HTMLElement>('[data-canvas-node-id]');
        const nodeId = handleElement?.dataset.canvasNodeId?.trim() ?? null;
        const handle = handleElement?.dataset.canvasConnectionHandle;
        const opposite = gesture.fixedHandle === 'source' ? 'target' : 'source';
        if (nodeId && handle === opposite) {
          return {
            nodeId,
            handleId: handleElement?.dataset.canvasHandleId ?? handle,
            isNearNode: true,
          };
        }
        return { nodeId: null, isNearNode: Boolean(nodeElement) };
      },
      []
    );

    const finishPointer = useCallback(
      (event: React.PointerEvent<HTMLDivElement>, canceled = false) => {
        const pointerId = event.pointerId;
        const gesture = interactionRef.current.gesture;
        if (!gesture || gesture.pointerId !== pointerId) return;

        if (gesture.kind === 'select') {
          applyCommand(canvasCommands.endBoxSelection());
        } else if (gesture.kind === 'resize') {
          finishCanvasResize(gesture, pointerId);
        } else if (gesture.kind === 'connection' && !canceled) {
          const latest = updateCanvasConnectionDrag(
            gesture,
            pointerId,
            localClientPoint(event.clientX, event.clientY),
            stateRef.current.viewport
          );
          applyInteractionResolution(
            finishCanvasConnectionDrag(
              stateRef.current.document,
              latest,
              pointerId,
              connectionDropTarget(latest, event.clientX, event.clientY),
              { at: event.timeStamp }
            )
          );
        }

        setInteraction({ type: 'gesture/end', pointerId });
        flushScheduledPersistence();
        releasePointer(pointerId);
      },
      [
        applyCommand,
        applyInteractionResolution,
        connectionDropTarget,
        flushScheduledPersistence,
        localClientPoint,
        releasePointer,
        setInteraction,
      ]
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
        const modifier = event.ctrlKey || event.metaKey;
        const key = event.key.toLowerCase();
        const pasteSequence = pasteSequenceRef.current + 1;
        const rect = surfaceRef.current?.getBoundingClientRect();
        const resolution = resolveCanvasKeyboardInput(
          stateRef.current,
          {
            key: event.key,
            ctrlKey: event.ctrlKey,
            metaKey: event.metaKey,
            shiftKey: event.shiftKey,
            altKey: event.altKey,
            editable: isCanvasKeyboardTarget(event.target),
          },
          {
            pasteSequence,
            at: event.timeStamp,
            clipboardWorldPosition: clientToCanvas(
              { x: (rect?.width ?? 0) / 2, y: (rect?.height ?? 0) / 2 },
              stateRef.current.viewport
            ),
          }
        );
        if (!resolution.handled) return;
        if (resolution.preventDefault) event.preventDefault();
        if (modifier && key === 'c') pasteSequenceRef.current = 0;
        if (
          modifier &&
          key === 'v' &&
          resolution.commands.some((command) => command.type === 'clipboard/paste')
        ) {
          pasteSequenceRef.current = pasteSequence;
        }
        applyInteractionResolution(resolution);
      },
      [applyInteractionResolution]
    );

    const handleCanvasContextMenu = useCallback(
      (event: React.MouseEvent<HTMLDivElement>) => {
        event.preventDefault();
        applyInteractionResolution(
          openCanvasContextMenu(
            { kind: 'canvas' },
            localClientPoint(event.clientX, event.clientY)
          )
        );
      },
      [applyInteractionResolution, localClientPoint]
    );

    const handleCanvasDoubleClick = useCallback(
      (event: React.MouseEvent<HTMLDivElement>) => {
        event.preventDefault();
        applyInteractionResolution(
          resolveCanvasDoubleClick(
            stateRef.current,
            { kind: 'canvas' },
            localClientPoint(event.clientX, event.clientY),
            stateRef.current.viewport
          )
        );
      },
      [applyInteractionResolution, localClientPoint]
    );

    const handleDragOver = useCallback((event: React.DragEvent<HTMLDivElement>) => {
      if (!hasDraggedFiles(event.dataTransfer)) return;
      event.preventDefault();
      event.dataTransfer.dropEffect = 'copy';
    }, []);

    const handleDrop = useCallback(
      (event: React.DragEvent<HTMLDivElement>) => {
        if (!hasDraggedFiles(event.dataTransfer)) return;
        event.preventDefault();
        const validation = validateCanvasDropImport(
          Array.from(event.dataTransfer.files),
          localClientPoint(event.clientX, event.clientY),
          stateRef.current.viewport
        );
        const intents: CanvasIntegrationIntent[] = [];
        if (validation.intent) intents.push(validation.intent);
        if (validation.rejected.length > 0 || validation.ignoredAcceptedFiles.length > 0) {
          intents.push({
            type: 'asset/import-feedback',
            rejected: validation.rejected.map(({ file, reason }) => ({
              fileName: file.name,
              reason,
            })),
            ignoredAcceptedFileNames: validation.ignoredAcceptedFiles.map((file) => file.name),
          });
        }
        applyInteractionResolution({
          handled: intents.length > 0,
          preventDefault: true,
          commands: [],
          intents,
        });
      },
      [applyInteractionResolution, localClientPoint]
    );

    const loadState = classifyCreativeCanvasLoadState({
      projectId,
      detail: project.detail,
      isLoading: project.isLoading,
      error: project.error,
    });
    const nodeById = useMemo(
      () => new Map(state.document.nodes.map((node) => [node.id, node])),
      [state.document.nodes]
    );
    const selectedNodeIds = useMemo(
      () => new Set(state.selection.nodeIds),
      [state.selection.nodeIds]
    );
    const selectedEdgeIds = useMemo(
      () => new Set(state.selection.edgeIds),
      [state.selection.edgeIds]
    );
    const graphHighlight = useMemo(
      () => deriveCanvasGraphHighlight(state.document, state.selection.nodeIds),
      [state.document, state.selection.nodeIds]
    );
    const hasGraphHighlight = graphHighlight.rootNodeIds.size > 0;
    const nodeLayer = useMemo(
      () =>
        state.document.nodes.map((node) => {
          const onPointerDown: React.PointerEventHandler<HTMLElement> = (event) =>
            beginNodePointer(node, event);
          const highlighted = graphHighlight.nodeIds.has(node.id);
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
              data-highlighted={highlighted || undefined}
              data-dimmed={hasGraphHighlight && !highlighted ? true : undefined}
              onPointerDown={onPointerDown}
              onContextMenu={(event) => {
                event.preventDefault();
                event.stopPropagation();
                applyInteractionResolution(
                  openCanvasContextMenu(
                    { kind: 'node', nodeId: node.id },
                    localClientPoint(event.clientX, event.clientY)
                  )
                );
              }}
              onDoubleClick={(event) => {
                event.preventDefault();
                event.stopPropagation();
                applyInteractionResolution(
                  resolveCanvasDoubleClick(
                    stateRef.current,
                    { kind: 'node', nodeId: node.id },
                    localClientPoint(event.clientX, event.clientY),
                    stateRef.current.viewport
                  )
                );
              }}
            >
              {renderNode({
                node,
                selected: selectedNodeIds.has(node.id),
                highlighted,
                dimmed: hasGraphHighlight && !highlighted,
                onActivate: () => applyCommand(canvasCommands.setSelection([node.id])),
                onOpen: () =>
                  applyInteractionResolution(
                    resolveCanvasDoubleClick(
                      stateRef.current,
                      { kind: 'node', nodeId: node.id },
                      { x: 0, y: 0 },
                      stateRef.current.viewport
                    )
                  ),
                onToggleLock: () =>
                  applyInteractionResolution(
                    resolveCanvasContextAction(
                      stateRef.current,
                      { kind: 'node', nodeId: node.id },
                      'toggle-lock'
                    )
                  ),
                dragHandleProps: { onPointerDown },
              })}
              {node.type !== 'group' ? (
                <button
                  type='button'
                  className={`${styles.connectionHandle} ${styles.connectionHandleInput}`}
                  aria-label={t('creativeStudio.canvas.editor.connectionInput')}
                  data-canvas-connection-handle='target'
                  data-canvas-handle-id='target'
                  data-canvas-node-id={node.id}
                  onPointerDown={(event) => beginConnectionDrag(node, 'target', event)}
                />
              ) : null}
              {node.type !== 'group' && node.type !== 'director' ? (
                <button
                  type='button'
                  className={`${styles.connectionHandle} ${styles.connectionHandleOutput}`}
                  aria-label={t('creativeStudio.canvas.editor.connectionOutput')}
                  data-canvas-connection-handle='source'
                  data-canvas-handle-id='source'
                  data-canvas-node-id={node.id}
                  onPointerDown={(event) => beginConnectionDrag(node, 'source', event)}
                />
              ) : null}
              {selectedNodeIds.has(node.id) && !node.locked
                ? RESIZE_CORNERS.map((corner) => (
                    <button
                      key={corner}
                      type='button'
                      className={styles.resizeHandle}
                      data-resize-corner={corner}
                      aria-label={t('creativeStudio.canvas.editor.resizeHandle', {
                        corner: t(RESIZE_CORNER_LABEL_KEYS[corner]),
                      })}
                      onPointerDown={(event) => beginNodeResize(node, corner, event)}
                    />
                  ))
                : null}
            </div>
          );
        }),
      [
        applyCommand,
        applyInteractionResolution,
        beginConnectionDrag,
        beginNodePointer,
        beginNodeResize,
        graphHighlight,
        hasGraphHighlight,
        localClientPoint,
        renderNode,
        selectedNodeIds,
        state.document.nodes,
        t,
      ]
    );
    const edgeLayer = useMemo(
      () =>
        state.document.connections.flatMap((connection) => {
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
                highlighted: graphHighlight.edgeIds.has(connection.id),
                dimmed:
                  hasGraphHighlight && !graphHighlight.edgeIds.has(connection.id),
                onActivate: () =>
                  applyCommand(canvasCommands.setSelection([], [connection.id])),
                onContextMenu: (event) => {
                  event.preventDefault();
                  event.stopPropagation();
                  applyInteractionResolution(
                    openCanvasContextMenu(
                      { kind: 'edge', edgeId: connection.id },
                      localClientPoint(event.clientX, event.clientY)
                    )
                  );
                },
              })}
            </React.Fragment>,
          ];
        }),
      [
        applyCommand,
        applyInteractionResolution,
        graphHighlight,
        hasGraphHighlight,
        localClientPoint,
        nodeById,
        renderEdge,
        selectedEdgeIds,
        state.document.connections,
      ]
    );
    if (loadState === 'loading') {
      return (
        <>
          {renderLoading?.() ??
            defaultLoading(t('creativeStudio.canvas.editor.loading'))}
        </>
      );
    }
    if (loadState === 'not-found') {
      return (
        <>
          {renderNotFound?.(projectId) ??
            defaultNotFound(
              projectId,
              t('creativeStudio.canvas.editor.notFound')
            )}
        </>
      );
    }
    if (loadState === 'error' && project.error) {
      return (
        <>
          {renderError?.(project.error, project.refresh) ??
            defaultError(
              project.error,
              project.refresh,
              t('creativeStudio.canvas.editor.loadFailed'),
              t('creativeStudio.canvas.actions.retry')
            )}
        </>
      );
    }

    const baseDocument = baseDocumentRef.current ?? project.detail?.document;
    if (!baseDocument) {
      return (
        <>
          {renderLoading?.() ??
            defaultLoading(t('creativeStudio.canvas.editor.loading'))}
        </>
      );
    }

    const flush = flushCanvasPersistence;
    const context: CreativeCanvasEditorContext = {
      state,
      save: saveSnapshot,
      tool,
      pendingTaskIds: pendingTaskIds ?? baseDocument.pendingTaskIds,
      agentSessions: agentSessions ?? baseDocument.chatSessions,
      activeAgentSessionId:
        agentSessions === null ? baseDocument.activeChatId : activeAgentSessionId,
      flush,
      reloadRemote,
    };
    const saveChrome = !showSaveState || saveSnapshot.status === 'idle' ? null : (
      <div
        className={styles.saveState}
        data-canvas-save-status={saveSnapshot.status}
        role={saveSnapshot.status === 'conflict' || saveSnapshot.status === 'error' ? 'alert' : 'status'}
        aria-live='polite'
      >
        {saveSnapshot.status === 'dirty'
          ? t('creativeStudio.canvas.save.pending')
          : null}
        {saveSnapshot.status === 'saving'
          ? t('creativeStudio.canvas.save.status.saving')
          : null}
        {saveSnapshot.status === 'saved'
          ? t('creativeStudio.canvas.save.status.saved')
          : null}
        {saveSnapshot.status === 'conflict' ? (
          <>
            <span>{t('creativeStudio.canvas.save.conflictMessage')}</span>
            <button type='button' onClick={() => void reloadRemote()}>
              {t('creativeStudio.canvas.save.reloadRemote')}
            </button>
          </>
        ) : null}
        {saveSnapshot.status === 'error' ? (
          <>
            <span>
              {saveSnapshot.error?.message ??
                t('creativeStudio.canvas.save.status.error')}
            </span>
            <button type='button' onClick={() => void saveController.flush()}>
              {t('creativeStudio.canvas.save.retry')}
            </button>
          </>
        ) : null}
      </div>
    );
    const resolvedTopDock = resolveSlot(topDock, context);
    const previewPath =
      interaction.gesture?.kind === 'connection'
        ? connectionPreviewPath(state, interaction.gesture)
        : null;
    const resolvedWorldOverlay = resolveSlot(worldOverlay, context);

    return (
      <CanvasSurface
        ref={surfaceRef}
        className={`${styles.editor} ${className ?? ''}`.trim()}
        viewport={state.viewport}
        backgroundMode={canvasSurfaceBackground(background)}
        tool={tool}
        isPanning={interaction.isPanning}
        ariaLabel={ariaLabel ?? t('creativeStudio.canvas.editor.label')}
        tabIndex={disabled ? -1 : 0}
        aria-disabled={disabled}
        data-creative-canvas-editor
        data-editor-disabled={disabled || undefined}
        data-editor-save-state={saveSnapshot.status}
        data-connection-dragging={interaction.gesture?.kind === 'connection' || undefined}
        nodeLayer={nodeLayer}
        edgeLayer={edgeLayer}
        worldOverlay={
          resolvedWorldOverlay || previewPath ? (
            <>
              {resolvedWorldOverlay}
              {previewPath ? (
                <svg className={styles.connectionPreview} aria-hidden='true'>
                  <path d={previewPath} vectorEffect='non-scaling-stroke' />
                </svg>
              ) : null}
            </>
          ) : undefined
        }
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
                disabled,
              }
            : false
        }
        onPointerDown={handleSurfacePointerDown}
        onPointerMove={handleSurfacePointerMove}
        onPointerUp={(event) => finishPointer(event)}
        onPointerCancel={(event) => finishPointer(event, true)}
        onLostPointerCapture={(event) => finishPointer(event, true)}
        onWheel={handleWheel}
        onKeyDown={handleKeyDown}
        onContextMenu={handleCanvasContextMenu}
        onDoubleClick={handleCanvasDoubleClick}
        onDragOver={handleDragOver}
        onDrop={handleDrop}
      />
    );
  }
);

CreativeCanvasEditor.displayName = 'CreativeCanvasEditor';

export default CreativeCanvasEditor;
