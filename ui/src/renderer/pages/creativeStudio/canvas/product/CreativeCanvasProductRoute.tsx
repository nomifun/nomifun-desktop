/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { Delete, Group, Loading, Refresh, Ungroup } from '@icon-park/react';
import { Tooltip } from '@arco-design/web-react';
import React, {
  useCallback,
  useEffect,
  useLayoutEffect,
  useMemo,
  useRef,
  useState,
} from 'react';
import { useTranslation } from 'react-i18next';
import { useNavigate, useParams } from 'react-router-dom';

import {
  type CreativeAsset,
  type CreativeAssetKind,
  useCreativeAssets,
} from '../../assets';
import { CREATIVE_STUDIO_PROJECTS_PATH } from '../../app/routes';
import {
  DEFAULT_CREATIVE_STUDIO_PANELS,
  type CreativeCanvasBackground,
  type CreativeCanvasNode,
  type CreativeCanvasNodeKind,
  type CreativeSize,
  type CreativeStudioPanelState,
} from '../../domain';
import type { PromptLibrarySelection } from '../../prompts';
import { useCreativeProject } from '../../services';
import { CreativeCanvasChrome } from '../chrome';
import type { CanvasInteractionTool } from '../components';
import {
  canRedoCanvas,
  canUndoCanvas,
  canvasCommands,
  type CanvasState,
} from '../core';
import {
  CreativeCanvasEditor,
  fitCanvasViewport,
  type CanvasCasSaveSnapshot,
  type CreativeCanvasEditorHandle,
} from '../editor';
import {
  CanvasMiniMap,
  type CanvasMiniMapNavigationRequest,
} from '../graph';
import { CreativeNodeView } from '../nodes';
import CreativeCanvasConnectionEdge from './CreativeCanvasConnectionEdge';
import {
  CreativeCanvasAssistantUnwiredPanel,
  CreativeCanvasHistoryPanel,
  CreativeCanvasOutlinePanel,
  CreativeCanvasPropertiesPanel,
  CreativeCanvasTimelineUnwiredPanel,
  CreativeCanvasUnavailablePanel,
  CreativeCanvasWorkflowUnwiredPanel,
} from './CreativeCanvasPanels';
import {
  CreativeCanvasProductAssetLibrary,
  CreativeCanvasProductPromptLibrary,
  type CreativeCanvasAssetKindFilter,
} from './CreativeCanvasProductLibraries';
import {
  createCreativeCanvasProductNode,
  creativeNodeFromAsset,
  creativeTextNodeFromPrompt,
} from './nodeFactory';
import {
  canLeaveCreativeCanvasAfterFlush,
  creativeCanvasProductPanelViews,
  creativeCanvasProductSelectionCapabilities,
  resolveCreativeNodeAssetPresentation,
  withCreativeCanvasBottomView,
  withCreativeCanvasLeftView,
  withCreativeCanvasRightView,
} from './productController';
import { registerCreativeCanvasProductBeforeLeave } from './beforeLeave';
import styles from './CreativeCanvasProductRoute.module.css';

const INITIAL_SAVE: CanvasCasSaveSnapshot = {
  status: 'idle',
  revision: null,
  hasPendingChanges: false,
  error: null,
};

const FALLBACK_VIEWPORT_SIZE: CreativeSize = { width: 1, height: 1 };

const iconProps = {
  theme: 'outline' as const,
  size: 17,
  fill: 'currentColor',
  strokeWidth: 2.5,
};

function measuredSize(element: HTMLElement | null): CreativeSize {
  const rect = element?.getBoundingClientRect();
  return {
    width: rect && Number.isFinite(rect.width) && rect.width > 0 ? rect.width : 1,
    height: rect && Number.isFinite(rect.height) && rect.height > 0 ? rect.height : 1,
  };
}

interface ProductToolbarButtonProps {
  label: string;
  disabled?: boolean;
  danger?: boolean;
  icon: React.ReactNode;
  onClick(): void;
}

const ProductToolbarButton: React.FC<ProductToolbarButtonProps> = ({
  label,
  disabled,
  danger,
  icon,
  onClick,
}) => (
  <Tooltip content={label} position='top' mini>
    <button
      type='button'
      className={styles.toolbarButton}
      data-danger={danger || undefined}
      aria-label={label}
      disabled={disabled}
      onClick={onClick}
    >
      {icon}
    </button>
  </Tooltip>
);

const SaveRecoveryAction: React.FC<{
  save: CanvasCasSaveSnapshot;
  busy: boolean;
  notice: string | null;
  onReload(): void;
  onRetry(): void;
}> = ({ save, busy, notice, onReload, onRetry }) => (
  <>
    {notice ? (
      <span className={styles.notice} role='status' title={notice}>
        {notice}
      </span>
    ) : null}
    {save.status === 'conflict' ? (
      <button
        type='button'
        className={styles.recoveryButton}
        disabled={busy}
        onClick={onReload}
      >
        {busy ? <Loading className={styles.spin} {...iconProps} /> : <Refresh {...iconProps} />}
        重新载入远端
      </button>
    ) : null}
    {save.status === 'error' ? (
      <button
        type='button'
        className={styles.recoveryButton}
        disabled={busy}
        onClick={onRetry}
      >
        {busy ? <Loading className={styles.spin} {...iconProps} /> : <Refresh {...iconProps} />}
        重试保存
      </button>
    ) : null}
  </>
);

/**
 * Route-level product composition. CreativeCanvasEditor remains the only
 * reducer and CAS owner; this component mirrors state solely to drive chrome.
 */
const CreativeCanvasProductRoute: React.FC = () => {
  const { projectId: routeProjectId } = useParams<{ projectId: string }>();
  const projectId = routeProjectId?.trim() ?? '';
  const navigate = useNavigate();
  const { i18n } = useTranslation();
  const locale = i18n.resolvedLanguage ?? i18n.language ?? 'zh-CN';
  const project = useCreativeProject(projectId || null);

  const editorRef = useRef<CreativeCanvasEditorHandle>(null);
  const canvasHostRef = useRef<HTMLDivElement>(null);
  const panelsRef = useRef<CreativeStudioPanelState>(
    structuredClone(DEFAULT_CREATIVE_STUDIO_PANELS)
  );
  const hydratedPanelsRef = useRef<{ projectId: string; revision: string } | null>(null);
  const hydratedBackgroundRef = useRef<{ projectId: string; revision: string } | null>(null);
  const knownAssetsRef = useRef<ReadonlyMap<string, CreativeAsset>>(new Map());

  const [canvasState, setCanvasState] = useState<CanvasState | null>(null);
  const [save, setSave] = useState<CanvasCasSaveSnapshot>(INITIAL_SAVE);
  const [tool, setTool] = useState<CanvasInteractionTool>('select');
  const [background, setBackground] = useState<CreativeCanvasBackground>('lines');
  const [viewportSize, setViewportSize] = useState<CreativeSize>(FALLBACK_VIEWPORT_SIZE);
  const [miniMapOpen, setMiniMapOpen] = useState(false);
  const [miniMapDragging, setMiniMapDragging] = useState(false);
  const [panels, setPanels] = useState<CreativeStudioPanelState>(() =>
    structuredClone(DEFAULT_CREATIVE_STUDIO_PANELS)
  );
  const [nodeMenuOpen, setNodeMenuOpen] = useState(false);
  const [backgroundMenuOpen, setBackgroundMenuOpen] = useState(false);
  const [recoveryBusy, setRecoveryBusy] = useState(false);
  const [notice, setNotice] = useState<string | null>(null);
  const [assetSearch, setAssetSearch] = useState('');
  const [assetKind, setAssetKind] = useState<CreativeCanvasAssetKindFilter>('all');
  const [selectedAssetIds, setSelectedAssetIds] = useState<ReadonlySet<string>>(new Set());
  const [selectedPromptId, setSelectedPromptId] = useState<string | null>(null);

  const assetQuery = useMemo(
    () => ({
      ...(assetSearch.trim() ? { search: assetSearch.trim() } : {}),
      ...(assetKind !== 'all' ? { kind: assetKind as CreativeAssetKind } : {}),
      sort: 'updated_desc' as const,
    }),
    [assetKind, assetSearch]
  );
  const assets = useCreativeAssets({ enabled: Boolean(projectId), query: assetQuery });

  const knownAssetsById = useMemo(() => {
    const merged = new Map(knownAssetsRef.current);
    for (const asset of assets.assets) merged.set(asset.id, asset);
    knownAssetsRef.current = merged;
    return merged;
  }, [assets.assets]);

  useLayoutEffect(() => {
    const host = canvasHostRef.current;
    if (!host) return;
    const update = () => setViewportSize(measuredSize(host));
    update();
    if (typeof ResizeObserver === 'undefined') return;
    const observer = new ResizeObserver(update);
    observer.observe(host);
    return () => observer.disconnect();
  }, []);

  useLayoutEffect(() => {
    const defaultPanels = structuredClone(DEFAULT_CREATIVE_STUDIO_PANELS);
    panelsRef.current = defaultPanels;
    setPanels(defaultPanels);
    hydratedPanelsRef.current = null;
    hydratedBackgroundRef.current = null;
    setCanvasState(null);
    setSave(INITIAL_SAVE);
    setNotice(null);
    knownAssetsRef.current = new Map();
  }, [projectId]);

  useEffect(() => {
    const detail = project.detail;
    if (!detail || detail.project.projectId !== projectId) return;

    const hydratedBackground = hydratedBackgroundRef.current;
    if (
      !hydratedBackground ||
      hydratedBackground.projectId !== projectId ||
      hydratedBackground.revision !== detail.project.revision
    ) {
      setBackground(detail.document.background);
      hydratedBackgroundRef.current = {
        projectId,
        revision: detail.project.revision,
      };
    }

    const hydrated = hydratedPanelsRef.current;
    const shouldHydratePanels =
      !hydrated ||
      hydrated.projectId !== projectId ||
      (save.status === 'idle' && hydrated.revision !== detail.project.revision);
    if (!shouldHydratePanels) return;

    const nextPanels = structuredClone(detail.document.panels);
    panelsRef.current = nextPanels;
    setPanels(nextPanels);
    hydratedPanelsRef.current = {
      projectId,
      revision: detail.project.revision,
    };
  }, [project.detail, projectId, save.status]);

  const dispatch = useCallback((command: Parameters<CreativeCanvasEditorHandle['dispatch']>[0]) => {
    return editorRef.current?.dispatch(command) ?? null;
  }, []);

  const persistPanels = useCallback((nextPanels: CreativeStudioPanelState) => {
    panelsRef.current = nextPanels;
    setPanels(nextPanels);
    editorRef.current?.setPanels(nextPanels);
  }, []);

  const handleLeftViewChange = useCallback(
    (view: CreativeStudioPanelState['left']['activeView']) => {
      persistPanels(withCreativeCanvasLeftView(panelsRef.current, view));
    },
    [persistPanels]
  );

  const handleRightViewChange = useCallback(
    (view: CreativeStudioPanelState['right']['activeView'] | null) => {
      persistPanels(withCreativeCanvasRightView(panelsRef.current, view));
    },
    [persistPanels]
  );

  const handleBottomViewChange = useCallback(
    (view: CreativeStudioPanelState['bottom']['activeView'] | null) => {
      persistPanels(withCreativeCanvasBottomView(panelsRef.current, view));
    },
    [persistPanels]
  );

  const addNode = useCallback(
    (kind: CreativeCanvasNodeKind) => {
      const editor = editorRef.current;
      if (!editor || save.revision === null) return;
      const state = editor.getState();
      const node = createCreativeCanvasProductNode(
        kind,
        state,
        measuredSize(canvasHostRef.current)
      );
      editor.dispatch(canvasCommands.addNode(node));
      setNotice(null);
    },
    [save.revision]
  );

  const handleBackgroundChange = useCallback((next: CreativeCanvasBackground) => {
    setBackground(next);
    editorRef.current?.setBackground(next);
  }, []);

  const handleFit = useCallback(() => {
    const editor = editorRef.current;
    if (!editor) return;
    editor.dispatch(
      canvasCommands.setViewport(
        fitCanvasViewport(editor.getState(), measuredSize(canvasHostRef.current))
      )
    );
  }, []);

  const handleMiniMapNavigate = useCallback((request: CanvasMiniMapNavigationRequest) => {
    setMiniMapDragging(request.phase !== 'end');
    editorRef.current?.dispatch(canvasCommands.setViewport(request.viewport));
  }, []);

  const flushBeforeLeave = useCallback(async (): Promise<boolean> => {
    const editor = editorRef.current;
    if (!editor) return true;
    return canLeaveCreativeCanvasAfterFlush(await editor.flush());
  }, []);

  useEffect(
    () => registerCreativeCanvasProductBeforeLeave(flushBeforeLeave),
    [flushBeforeLeave]
  );

  const handleBackToProjects = useCallback(async () => {
    if (recoveryBusy) return;
    setRecoveryBusy(true);
    try {
      if (await flushBeforeLeave()) {
        navigate(CREATIVE_STUDIO_PROJECTS_PATH);
      }
    } finally {
      setRecoveryBusy(false);
    }
  }, [flushBeforeLeave, navigate, recoveryBusy]);

  const handleReloadRemote = useCallback(async () => {
    if (!editorRef.current || recoveryBusy) return;
    setRecoveryBusy(true);
    try {
      const reloaded = await editorRef.current.reloadRemote();
      setNotice(reloaded ? '已重新载入远端版本。' : '远端版本暂时不可用。');
    } finally {
      setRecoveryBusy(false);
    }
  }, [recoveryBusy]);

  const handleRetrySave = useCallback(async () => {
    if (!editorRef.current || recoveryBusy) return;
    setRecoveryBusy(true);
    try {
      const result = await editorRef.current.flush();
      setNotice(
        result.status === 'saved' || result.status === 'noop'
          ? '保存已完成。'
          : result.error.message
      );
    } finally {
      setRecoveryBusy(false);
    }
  }, [recoveryBusy]);

  const handleSelectOutlineNode = useCallback(
    (nodeId: string, mode: 'replace' | 'toggle') => {
      dispatch(
        mode === 'toggle'
          ? canvasCommands.toggleNodeSelection(nodeId)
          : canvasCommands.setSelection([nodeId])
      );
    },
    [dispatch]
  );

  const handleUpdateNode = useCallback(
    (node: CreativeCanvasNode, field: string) => {
      dispatch(
        canvasCommands.updateNode(node, {
          mergeKey: `property:${node.id}:${field}`,
        })
      );
    },
    [dispatch]
  );

  const handleToggleAsset = useCallback((assetId: string) => {
    setSelectedAssetIds((current) => {
      const next = new Set(current);
      if (next.has(assetId)) next.delete(assetId);
      else next.add(assetId);
      return next;
    });
  }, []);

  const handleInsertAssets = useCallback((selectedAssets: readonly CreativeAsset[]) => {
    const editor = editorRef.current;
    if (!editor) return;
    let state = editor.getState();
    let inserted = 0;
    const errors: string[] = [];
    for (const asset of selectedAssets) {
      try {
        const node = creativeNodeFromAsset(
          asset,
          state,
          measuredSize(canvasHostRef.current),
          { cascadeIndex: state.document.nodes.length }
        );
        state = editor.dispatch(canvasCommands.addNode(node));
        inserted += 1;
      } catch (error) {
        errors.push(error instanceof Error ? error.message : String(error));
      }
    }
    setSelectedAssetIds(new Set());
    setNotice(
      errors.length > 0
        ? `${inserted} 项已插入；${errors.length} 项未插入：${errors[0]}`
        : `${inserted} 项素材已插入画布。`
    );
  }, []);

  const handleInsertPrompt = useCallback((selection: PromptLibrarySelection) => {
    const editor = editorRef.current;
    if (!editor) return;
    const state = editor.getState();
    editor.dispatch(
      canvasCommands.addNode(
        creativeTextNodeFromPrompt(
          selection,
          state,
          measuredSize(canvasHostRef.current)
        )
      )
    );
    setSelectedPromptId(selection.id);
    setNotice(`已将“${selection.title}”插入为文本节点。`);
  }, []);

  const selection = useMemo(
    () => creativeCanvasProductSelectionCapabilities(canvasState),
    [canvasState]
  );
  const productDisabled = save.revision === null;
  const projectTitle = project.detail?.project.title ?? (project.isLoading ? '正在载入项目…' : '画布项目');
  const saveMessage = save.error?.message ?? notice ?? undefined;
  const compact = viewportSize.width < 760;
  const panelViews = creativeCanvasProductPanelViews(panels);
  const canvasLayoutStyle = {
    '--creative-canvas-left-panel-width': `${panels.left.open ? panels.left.width : 0}px`,
    '--creative-canvas-right-panel-width': `${panels.right.width}px`,
    '--creative-canvas-bottom-panel-height': `${panels.bottom.height}px`,
  } as React.CSSProperties;

  const renderCanvasState = canvasState;
  const canvasOutline = renderCanvasState ? (
    <CreativeCanvasOutlinePanel
      state={renderCanvasState}
      onSelectNode={handleSelectOutlineNode}
      onClearSelection={() => dispatch(canvasCommands.clearSelection())}
    />
  ) : (
    <CreativeCanvasUnavailablePanel
      kind='generic'
      title='正在载入画布结构'
      description='等待项目文档通过 canonical v1 校验。'
    />
  );

  const properties = renderCanvasState ? (
    <CreativeCanvasPropertiesPanel
      state={renderCanvasState}
      onSelectNode={(nodeId) => dispatch(canvasCommands.setSelection([nodeId]))}
      onUpdateNode={handleUpdateNode}
    />
  ) : (
    <CreativeCanvasUnavailablePanel
      kind='generic'
      title='正在载入属性'
      description='选择真实节点后才能查看 canonical 属性。'
    />
  );

  const history = renderCanvasState ? (
    <CreativeCanvasHistoryPanel
      state={renderCanvasState}
      onUndo={() => dispatch(canvasCommands.undo())}
      onRedo={() => dispatch(canvasCommands.redo())}
    />
  ) : (
    <CreativeCanvasUnavailablePanel
      kind='generic'
      title='正在载入撤销状态'
      description='历史面板仅展示当前编辑会话的真实撤销栈。'
    />
  );

  return (
    <main
      className={styles.root}
      style={canvasLayoutStyle}
      data-creative-canvas-product-route
      data-project-id={projectId}
    >
      <CreativeCanvasChrome
        projectTitle={projectTitle}
        saveStatus={save.status}
        saveMessage={saveMessage}
        tool={tool}
        background={background}
        canUndo={Boolean(canvasState && canUndoCanvas(canvasState))}
        canRedo={Boolean(canvasState && canRedoCanvas(canvasState))}
        isMiniMapOpen={miniMapOpen}
        leftView={panelViews.left}
        rightView={panelViews.right}
        bottomView={panelViews.bottom}
        nodeMenuOpen={nodeMenuOpen}
        backgroundMenuOpen={backgroundMenuOpen}
        compact={compact}
        disabled={productDisabled}
        onBackToProjects={() => void handleBackToProjects()}
        onToolChange={setTool}
        onAddNode={addNode}
        onNodeMenuOpenChange={setNodeMenuOpen}
        onBackgroundChange={handleBackgroundChange}
        onBackgroundMenuOpenChange={setBackgroundMenuOpen}
        onUndo={() => dispatch(canvasCommands.undo())}
        onRedo={() => dispatch(canvasCommands.redo())}
        onFitView={handleFit}
        onToggleMiniMap={() => setMiniMapOpen((open) => !open)}
        onLeftViewChange={handleLeftViewChange}
        onRightViewChange={handleRightViewChange}
        onBottomViewChange={handleBottomViewChange}
        slots={{
          canvas: (
            <div ref={canvasHostRef} className={styles.canvasHost}>
              <CreativeCanvasEditor
                ref={editorRef}
                projectId={projectId}
                tool={tool}
                showZoomControls={false}
                isMiniMapOpen={miniMapOpen}
                onToggleMiniMap={() => setMiniMapOpen((open) => !open)}
                onStateChange={setCanvasState}
                onSaveStateChange={setSave}
                renderNode={({ node, selected, onActivate, dragHandleProps }) => (
                  <CreativeNodeView
                    node={node}
                    selected={selected}
                    placement='contained'
                    asset={resolveCreativeNodeAssetPresentation(node, knownAssetsById) ?? undefined}
                    onActivate={onActivate}
                    onPointerDown={dragHandleProps.onPointerDown}
                  />
                )}
                renderEdge={(context) => <CreativeCanvasConnectionEdge {...context} />}
                miniMap={({ state }) => (
                  <CanvasMiniMap
                    nodes={state.document.nodes}
                    viewport={state.viewport}
                    viewportSize={viewportSize}
                    selectedNodeIds={new Set(state.selection.nodeIds)}
                    dragging={miniMapDragging}
                    onNavigate={handleMiniMapNavigate}
                  />
                )}
              />
            </div>
          ),
          topActions: (
            <SaveRecoveryAction
              save={save}
              busy={recoveryBusy}
              notice={notice}
              onReload={() => void handleReloadRemote()}
              onRetry={() => void handleRetrySave()}
            />
          ),
          toolbarTrailing: (
            <>
              <ProductToolbarButton
                label='将所选节点分组'
                icon={<Group {...iconProps} />}
                disabled={productDisabled || !selection.canGroup}
                onClick={() =>
                  dispatch(
                    canvasCommands.groupNodes({
                      nodeIds: canvasState?.selection.nodeIds,
                      title: '节点组',
                    })
                  )
                }
              />
              <ProductToolbarButton
                label='取消所选分组'
                icon={<Ungroup {...iconProps} />}
                disabled={productDisabled || selection.groupIds.length === 0}
                onClick={() => {
                  for (const groupId of selection.groupIds) {
                    dispatch(canvasCommands.ungroup(groupId));
                  }
                }}
              />
              <ProductToolbarButton
                label='删除所选节点或连接'
                icon={<Delete {...iconProps} />}
                danger
                disabled={productDisabled || !selection.hasSelection}
                onClick={() => dispatch(canvasCommands.deleteSelection())}
              />
            </>
          ),
          left: {
            canvas: canvasOutline,
            assets: (
              <CreativeCanvasProductAssetLibrary
                state={assets}
                search={assetSearch}
                kind={assetKind}
                selectedIds={selectedAssetIds}
                disabled={productDisabled}
                onSearchChange={setAssetSearch}
                onKindChange={setAssetKind}
                onToggleAsset={handleToggleAsset}
                onInsert={handleInsertAssets}
              />
            ),
            prompts: (
              <CreativeCanvasProductPromptLibrary
                locale={locale}
                enabled={!productDisabled}
                selectedId={selectedPromptId}
                onSelect={setSelectedPromptId}
                onInsert={handleInsertPrompt}
              />
            ),
            workflows: <CreativeCanvasWorkflowUnwiredPanel />,
          },
          right: {
            assistant: <CreativeCanvasAssistantUnwiredPanel />,
            properties,
          },
          bottom: {
            history,
            timeline: <CreativeCanvasTimelineUnwiredPanel />,
          },
        }}
      />
    </main>
  );
};

export default CreativeCanvasProductRoute;
