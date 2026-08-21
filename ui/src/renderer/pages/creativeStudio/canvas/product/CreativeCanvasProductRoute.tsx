/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { uuidv7 } from '@/common/utils/uuidv7';
import {
  CloseOne,
  Delete,
  Group,
  Loading,
  Refresh,
  Ungroup,
} from '@icon-park/react';
import { Button, Modal, Tooltip } from '@arco-design/web-react';
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
  creativeAssetClient,
  type CreativeAsset,
  type CreativeAssetKind,
  useCreativeAssetPickerDialog,
  useCreativeAssets,
} from '../../assets';
import {
  creativeAssetDownloadName,
  manualUploadRejectionMessage,
} from '../../assets/page/model';
import {
  CREATIVE_STUDIO_PROJECTS_PATH,
  CREATIVE_STUDIO_WORKFLOWS_PATH,
  creativeStudioDirectorProjectPath,
} from '../../app/routes';
import {
  DEFAULT_CREATIVE_STUDIO_PANELS,
  type CreativeCanvasBackground,
  type CreativeCanvasNode,
  type CreativeCanvasNodeKind,
  type CreativeChatSessionReference,
  type CreativeSize,
  type CreativeStudioPanelState,
} from '../../domain';
import {
  useNomiCreativeModelCatalog,
  type CreativeModelSelectionRef,
} from '../../models';
import type { PromptLibrarySelection } from '../../prompts';
import { useCreativeProject } from '../../services';
import type { CreativeTaskReference } from '../../tasks';
import {
  exactWorkbenchModelOptions,
  type CreativeWorkbenchRuntimeSnapshot,
  type PreparedCreativeWorkbenchRun,
} from '../../workbenches/runtime';
import type {
  WorkflowDefinitionV1,
  WorkflowRunAggregateV1,
} from '../../workflows/domain';
import {
  WorkflowRunModal,
  type CreativeWorkflowRunnerPort,
} from '../../workflows/page';
import { useCreativeWorkflowRuntime } from '../../workflows/runtime';
import { creativeWorkflowRepository } from '../../workflows/services';
import { CreativeCanvasChrome } from '../chrome';
import type { CanvasInteractionTool } from '../components';
import {
  canRedoCanvas,
  canUndoCanvas,
  canvasToClient,
  canvasCommands,
  clientToCanvas,
  validateCanvasConnection,
  type CanvasPoint,
  type CanvasState,
} from '../core';
import {
  CreativeCanvasEditor,
  fitCanvasViewport,
  type CanvasCasSaveSnapshot,
  type CreativeCanvasEditorHandle,
} from '../editor';
import { CanvasMiniMap, type CanvasMiniMapNavigationRequest } from '../graph';
import {
  resolveCanvasContextAction,
  type CanvasContextAction,
  type CanvasContextTarget,
  type CanvasIntegrationIntent,
} from '../interactions';
import {
  CreativeCanvasImageToolbar,
  CreativeImageCropDialog,
  CreativeImageMaskEditDialog,
  CreativeImageSplitDialog,
  buildCreativeImageMaskReference,
  createCreativeImageSplitCanvasLayout,
  creativeImageSplitColumns,
  creativeImageSplitNodePosition,
  creativeImageSplitRows,
  cropCreativeImageAsset,
  nextDerivedImagePosition,
  removeUploadedCreativeImageSplit,
  removeCreativeImageMaskReference,
  splitCreativeImageAsset,
  uploadCreativeImageCrop,
  uploadCreativeImageMaskReference,
  uploadCreativeImageSplit,
  type CreativeImageCropRect,
  type CreativeImageMaskEditSubmit,
  type CreativeImageSplitParams,
  type UploadedCreativeImageSplitPiece,
} from '../imageTools';
import { CreativeNodeView } from '../nodes';
import CreativeCanvasAgentPanel, {
  type CreativeCanvasAgentPanelHandle,
} from './agent/CreativeCanvasAgentPanel';
import CreativeCanvasConnectionEdge from './CreativeCanvasConnectionEdge';
import CreativeCanvasInteractionOverlays, {
  type CreativeCanvasContextMenuState,
} from './CreativeCanvasInteractionOverlays';
import {
  CreativeCanvasHistoryPanel,
  CreativeCanvasOutlinePanel,
  CreativeCanvasPropertiesPanel,
  CreativeCanvasUnavailablePanel,
} from './CreativeCanvasPanels';
import CreativeCanvasTimelinePanel from './CreativeCanvasTimelinePanel';
import CreativeCanvasWorkflowPanel from './CreativeCanvasWorkflowPanel';
import {
  CreativeCanvasProductAssetLibrary,
  CreativeCanvasProductPromptLibrary,
  type CreativeCanvasAssetKindFilter,
} from './CreativeCanvasProductLibraries';
import {
  createCreativeCanvasProductNode,
  CREATIVE_CANVAS_PRODUCT_NODE_SIZES,
  creativeCanvasProductInsertionViewport,
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
import CanvasImageMaskEditRuntimeBridge, {
  canvasImageMaskEditReferenceFromPlan,
  type CanvasImageMaskEditRuntimeBridgeHandle,
} from './CanvasImageMaskEditRuntimeBridge';
import {
  preferredCanvasImageMaskEditModel,
  prepareCanvasImageMaskEdit,
} from './imageMaskEditCanvas';
import { orphanCanvasImageMaskEditTask } from './imageMaskEditRuntime';
import { registerCreativeCanvasProductBeforeLeave } from './beforeLeave';
import styles from './CreativeCanvasProductRoute.module.css';

const INITIAL_SAVE: CanvasCasSaveSnapshot = {
  status: 'idle',
  revision: null,
  hasPendingChanges: false,
  error: null,
};

const INITIAL_IMAGE_MASK_RUNTIME: CreativeWorkbenchRuntimeSnapshot = {
  state: 'idle',
  entries: [],
  submissionFailures: [],
  submittingCount: 0,
  recoveringCount: 0,
  requestError: null,
};

const FALLBACK_VIEWPORT_SIZE: CreativeSize = { width: 1, height: 1 };

type ConnectionCreateNodeIntent = Extract<
  CanvasIntegrationIntent,
  { type: 'connection/create-node-menu/open' }
>;

interface ProductCreateNodeMenuState {
  worldPosition: CanvasPoint;
  clientPosition: CanvasPoint;
  connection: ConnectionCreateNodeIntent | null;
}

interface PendingPanoramaChoice {
  asset: CreativeAsset;
  worldPosition: CanvasPoint;
}

interface PendingImageCrop {
  nodeId: string;
  asset: CreativeAsset;
}

interface PendingImageSplit {
  nodeId: string;
  asset: CreativeAsset;
}

interface PendingImageMaskSubmission {
  plan: PreparedCreativeWorkbenchRun;
  reference: CreativeTaskReference;
  failureOrder: number;
}

interface PendingImageMaskEdit {
  nodeId: string;
  asset: CreativeAsset;
  submission: PendingImageMaskSubmission | null;
}

interface AgentDocumentState {
  sessions: readonly CreativeChatSessionReference[];
  activeSessionId: string | null;
}

const iconProps = {
  theme: 'outline' as const,
  size: 17,
  fill: 'currentColor',
  strokeWidth: 2.5,
};

function measuredSize(element: HTMLElement | null): CreativeSize {
  const rect = element?.getBoundingClientRect();
  return {
    width:
      rect && Number.isFinite(rect.width) && rect.width > 0 ? rect.width : 1,
    height:
      rect && Number.isFinite(rect.height) && rect.height > 0 ? rect.height : 1,
  };
}

const centeredNodePosition = (
  kind: CreativeCanvasNodeKind,
  worldPosition: CanvasPoint
): CanvasPoint => {
  const size = CREATIVE_CANVAS_PRODUCT_NODE_SIZES[kind];
  return {
    x: worldPosition.x - size.width / 2,
    y: worldPosition.y - size.height / 2,
  };
};

const isTwoToOneImage = (asset: CreativeAsset): boolean =>
  asset.kind === 'image' &&
  asset.width !== null &&
  asset.height !== null &&
  asset.width > 0 &&
  asset.height > 0 &&
  Math.abs(asset.width / asset.height - 2) <= 0.03;

const connectionErrorMessage = (
  code: Extract<
    CanvasIntegrationIntent,
    { type: 'connection/rejected' }
  >['code']
): string => {
  switch (code) {
    case 'missing_source':
    case 'missing_target':
      return '连接端点已经不存在';
    case 'self_connection':
      return '节点不能连接到自身';
    case 'duplicate_connection':
      return '这两个节点已经连接';
    case 'group_connection':
      return '节点组不能参与生成连接';
    case 'config_to_config':
      return '两个配置节点不能直接连接';
    case 'director_output_not_supported':
      return '导演节点只能接收输入';
    case 'director_requires_image_input':
      return '导演节点只接受图片或全景图输入';
    case 'no_valid_drop_target':
      return '请将连接拖到对端节点的有效连接点';
  }
};

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
  <Tooltip content={label} position="top" mini>
    <button
      type="button"
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
      <span className={styles.notice} role="status" title={notice}>
        {notice}
      </span>
    ) : null}
    {save.status === 'conflict' ? (
      <button
        type="button"
        className={styles.recoveryButton}
        disabled={busy}
        onClick={onReload}
      >
        {busy ? (
          <Loading className={styles.spin} {...iconProps} />
        ) : (
          <Refresh {...iconProps} />
        )}
        重新载入远端
      </button>
    ) : null}
    {save.status === 'error' ? (
      <button
        type="button"
        className={styles.recoveryButton}
        disabled={busy}
        onClick={onRetry}
      >
        {busy ? (
          <Loading className={styles.spin} {...iconProps} />
        ) : (
          <Refresh {...iconProps} />
        )}
        重试保存
      </button>
    ) : null}
  </>
);

const ImageMaskRuntimeAction: React.FC<{
  snapshot: CreativeWorkbenchRuntimeSnapshot;
  busy: boolean;
  onCancel(taskId: string): void;
  onRetry(taskId: string): void;
}> = ({ snapshot, busy, onCancel, onRetry }) => {
  const requestError = snapshot.entries.find(
    (entry) => entry.requestError !== null
  );
  const active = snapshot.entries.find(
    (entry) => entry.task.status === 'queued' || entry.task.status === 'running'
  );
  if (requestError) {
    return (
      <>
        <span
          className={styles.notice}
          role="alert"
          title={requestError.requestError?.message}
        >
          局部编辑同步中断
        </span>
        <button
          type="button"
          className={styles.recoveryButton}
          disabled={busy}
          onClick={() => onRetry(requestError.task.taskId)}
        >
          {busy ? (
            <Loading className={styles.spin} {...iconProps} />
          ) : (
            <Refresh {...iconProps} />
          )}
          重试任务同步
        </button>
      </>
    );
  }
  if (active) {
    return (
      <>
        <span className={styles.notice} role="status">
          {active.task.status === 'queued'
            ? '局部编辑等待执行'
            : '局部编辑生成中'}
        </span>
        <button
          type="button"
          className={styles.recoveryButton}
          disabled={busy}
          onClick={() => onCancel(active.task.taskId)}
        >
          {busy ? (
            <Loading className={styles.spin} {...iconProps} />
          ) : (
            <CloseOne {...iconProps} />
          )}
          取消任务
        </button>
      </>
    );
  }
  if (snapshot.recoveringCount > 0) {
    return (
      <span className={styles.notice} role="status">
        正在恢复局部编辑任务…
      </span>
    );
  }
  return null;
};

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
  const modelCatalog = useNomiCreativeModelCatalog();
  const workflowRuntime = useCreativeWorkflowRuntime();
  const workflowAssetPicker = useCreativeAssetPickerDialog();

  const editorRef = useRef<CreativeCanvasEditorHandle>(null);
  const imageMaskRuntimeRef =
    useRef<CanvasImageMaskEditRuntimeBridgeHandle>(null);
  const agentPanelRef = useRef<CreativeCanvasAgentPanelHandle>(null);
  const canvasHostRef = useRef<HTMLDivElement>(null);
  const panelsRef = useRef<CreativeStudioPanelState>(
    structuredClone(DEFAULT_CREATIVE_STUDIO_PANELS)
  );
  const hydratedPanelsRef = useRef<{
    projectId: string;
    revision: string;
  } | null>(null);
  const hydratedBackgroundRef = useRef<{
    projectId: string;
    revision: string;
  } | null>(null);
  const knownAssetsRef = useRef<ReadonlyMap<string, CreativeAsset>>(new Map());
  const assetImportBusyRef = useRef(false);
  const imageToolBusyRef = useRef(false);
  const imageToolAbortRef = useRef<AbortController | null>(null);
  const activeProjectIdRef = useRef(projectId);
  const workflowRequestRef = useRef(0);

  const [canvasState, setCanvasState] = useState<CanvasState | null>(null);
  const [save, setSave] = useState<CanvasCasSaveSnapshot>(INITIAL_SAVE);
  const [tool, setTool] = useState<CanvasInteractionTool>('select');
  const [background, setBackground] =
    useState<CreativeCanvasBackground>('lines');
  const [viewportSize, setViewportSize] = useState<CreativeSize>(
    FALLBACK_VIEWPORT_SIZE
  );
  const [miniMapOpen, setMiniMapOpen] = useState(false);
  const [miniMapDragging, setMiniMapDragging] = useState(false);
  const [panels, setPanels] = useState<CreativeStudioPanelState>(() =>
    structuredClone(DEFAULT_CREATIVE_STUDIO_PANELS)
  );
  const [backgroundMenuOpen, setBackgroundMenuOpen] = useState(false);
  const [recoveryBusy, setRecoveryBusy] = useState(false);
  const [notice, setNotice] = useState<string | null>(null);
  const [assetSearch, setAssetSearch] = useState('');
  const [assetKind, setAssetKind] =
    useState<CreativeCanvasAssetKindFilter>('all');
  const [selectedAssetIds, setSelectedAssetIds] = useState<ReadonlySet<string>>(
    new Set()
  );
  const [selectedPromptId, setSelectedPromptId] = useState<string | null>(null);
  const [contextMenu, setContextMenu] =
    useState<CreativeCanvasContextMenuState | null>(null);
  const [createNodeMenu, setCreateNodeMenu] =
    useState<ProductCreateNodeMenuState | null>(null);
  const [pendingPanoramaChoice, setPendingPanoramaChoice] =
    useState<PendingPanoramaChoice | null>(null);
  const [assetImportBusy, setAssetImportBusy] = useState(false);
  const [pendingImageCrop, setPendingImageCrop] =
    useState<PendingImageCrop | null>(null);
  const [imageCropBusy, setImageCropBusy] = useState(false);
  const [imageCropProgress, setImageCropProgress] = useState<number | null>(
    null
  );
  const [imageCropError, setImageCropError] = useState<string | null>(null);
  const [pendingImageSplit, setPendingImageSplit] =
    useState<PendingImageSplit | null>(null);
  const [imageSplitBusy, setImageSplitBusy] = useState(false);
  const [imageSplitProgress, setImageSplitProgress] = useState<number | null>(
    null
  );
  const [imageSplitError, setImageSplitError] = useState<string | null>(null);
  const [pendingImageMaskEdit, setPendingImageMaskEdit] =
    useState<PendingImageMaskEdit | null>(null);
  const [imageMaskModel, setImageMaskModel] =
    useState<CreativeModelSelectionRef | null>(null);
  const [imageMaskBusy, setImageMaskBusy] = useState(false);
  const [imageMaskProgress, setImageMaskProgress] = useState<number | null>(
    null
  );
  const [imageMaskError, setImageMaskError] = useState<string | null>(null);
  const [imageMaskRuntime, setImageMaskRuntime] =
    useState<CreativeWorkbenchRuntimeSnapshot>(INITIAL_IMAGE_MASK_RUNTIME);
  const [imageMaskRuntimeReady, setImageMaskRuntimeReady] = useState(false);
  const [imageMaskRuntimeEpoch, setImageMaskRuntimeEpoch] = useState(0);
  const [imageMaskRuntimeActionBusy, setImageMaskRuntimeActionBusy] =
    useState(false);
  const [agentDocumentState, setAgentDocumentState] =
    useState<AgentDocumentState | null>(null);
  const [workflows, setWorkflows] = useState<WorkflowDefinitionV1[]>([]);
  const [workflowLoading, setWorkflowLoading] = useState(false);
  const [workflowError, setWorkflowError] = useState<string | null>(null);
  const [workflowToRun, setWorkflowToRun] =
    useState<WorkflowDefinitionV1 | null>(null);
  const [workflowInsertingRunId, setWorkflowInsertingRunId] = useState<
    string | null
  >(null);

  const loadWorkflows = useCallback(async () => {
    const request = ++workflowRequestRef.current;
    setWorkflowLoading(true);
    setWorkflowError(null);
    try {
      const loaded = await creativeWorkflowRepository.list();
      if (request !== workflowRequestRef.current) return;
      setWorkflows(
        [...loaded].sort(
          (left, right) => right.metadata.updatedAt - left.metadata.updatedAt
        )
      );
    } catch (error) {
      if (request !== workflowRequestRef.current) return;
      setWorkflowError(error instanceof Error ? error.message : String(error));
    } finally {
      if (request === workflowRequestRef.current) setWorkflowLoading(false);
    }
  }, []);

  useEffect(() => {
    void loadWorkflows();
    return () => {
      workflowRequestRef.current += 1;
    };
  }, [loadWorkflows]);

  const assetQuery = useMemo(
    () => ({
      ...(assetSearch.trim() ? { search: assetSearch.trim() } : {}),
      ...(assetKind !== 'all' ? { kind: assetKind as CreativeAssetKind } : {}),
      sort: 'updated_desc' as const,
    }),
    [assetKind, assetSearch]
  );
  const assets = useCreativeAssets({
    enabled: Boolean(projectId),
    query: assetQuery,
  });
  const imageMaskModelOptions = useMemo(
    () => exactWorkbenchModelOptions(modelCatalog, 'image_edit'),
    [modelCatalog]
  );

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
    activeProjectIdRef.current = projectId;
    imageToolAbortRef.current?.abort();
    imageToolAbortRef.current = null;
    const defaultPanels = structuredClone(DEFAULT_CREATIVE_STUDIO_PANELS);
    panelsRef.current = defaultPanels;
    setPanels(defaultPanels);
    hydratedPanelsRef.current = null;
    hydratedBackgroundRef.current = null;
    setCanvasState(null);
    setSave(INITIAL_SAVE);
    setNotice(null);
    setContextMenu(null);
    setCreateNodeMenu(null);
    setPendingPanoramaChoice(null);
    setAssetImportBusy(false);
    setPendingImageCrop(null);
    setImageCropBusy(false);
    setImageCropProgress(null);
    setImageCropError(null);
    setPendingImageSplit(null);
    setImageSplitBusy(false);
    setImageSplitProgress(null);
    setImageSplitError(null);
    setPendingImageMaskEdit(null);
    setImageMaskModel(null);
    setImageMaskBusy(false);
    setImageMaskProgress(null);
    setImageMaskError(null);
    setImageMaskRuntime(INITIAL_IMAGE_MASK_RUNTIME);
    setImageMaskRuntimeReady(false);
    setImageMaskRuntimeEpoch(0);
    setImageMaskRuntimeActionBusy(false);
    setAgentDocumentState(null);
    assetImportBusyRef.current = false;
    imageToolBusyRef.current = false;
    knownAssetsRef.current = new Map();
    return () => {
      imageToolAbortRef.current?.abort();
    };
  }, [projectId]);

  useEffect(() => {
    if (imageMaskRuntimeReady) return;
    const detail = project.detail;
    if (!detail || detail.project.projectId !== projectId || !canvasState)
      return;
    const currentNodeIds = new Set(
      canvasState.document.nodes.map((node) => node.id)
    );
    if (detail.document.nodes.every((node) => currentNodeIds.has(node.id))) {
      setImageMaskRuntimeReady(true);
    }
  }, [canvasState, imageMaskRuntimeReady, project.detail, projectId]);

  useEffect(() => {
    if (!imageMaskModel || pendingImageMaskEdit?.submission) return;
    const available = imageMaskModelOptions.some(
      (option) =>
        option.providerId === imageMaskModel.providerId &&
        option.model === imageMaskModel.model
    );
    if (!available) setImageMaskModel(null);
  }, [imageMaskModel, imageMaskModelOptions, pendingImageMaskEdit?.submission]);

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

  const dispatch = useCallback(
    (command: Parameters<CreativeCanvasEditorHandle['dispatch']>[0]) => {
      return editorRef.current?.dispatch(command) ?? null;
    },
    []
  );

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

  const prepareCenteredInsertion = useCallback(() => {
    const editor = editorRef.current;
    if (!editor) return null;
    const viewportSize = measuredSize(canvasHostRef.current);
    let state = editor.getState();
    const viewport = creativeCanvasProductInsertionViewport(state, viewportSize);
    if (
      viewport.x !== state.viewport.x ||
      viewport.y !== state.viewport.y ||
      viewport.zoom !== state.viewport.zoom
    ) {
      state = editor.dispatch(canvasCommands.setViewport(viewport));
    }
    return { editor, state, viewportSize };
  }, []);

  const addNode = useCallback(
    (kind: CreativeCanvasNodeKind) => {
      if (save.revision === null) return;
      const insertion = prepareCenteredInsertion();
      if (!insertion) return;
      const { editor, state, viewportSize } = insertion;
      if (kind === 'director') {
        const directors = state.document.nodes.filter(
          (node) => node.type === 'director'
        );
        if (directors.length > 0) {
          editor.dispatch(
            canvasCommands.setSelection(directors.map((node) => node.id))
          );
          handleBottomViewChange('timeline');
          setNotice(
            directors.length === 1
              ? '项目已有唯一导演节点，已为你选中。'
              : '项目存在多个导演节点，请在时间线面板中处理冲突。'
          );
          return;
        }
      }
      const node = createCreativeCanvasProductNode(
        kind,
        state,
        viewportSize
      );
      editor.dispatch(canvasCommands.addNode(node));
      if (kind === 'director') {
        handleBottomViewChange('timeline');
        setNotice('已创建项目唯一的导演节点。');
      } else {
        setNotice(null);
      }
    },
    [handleBottomViewChange, prepareCenteredInsertion, save.revision]
  );

  const handleBackgroundChange = useCallback(
    (next: CreativeCanvasBackground) => {
      setBackground(next);
      editorRef.current?.setBackground(next);
    },
    []
  );

  const handleFit = useCallback(() => {
    const editor = editorRef.current;
    if (!editor) return;
    editor.dispatch(
      canvasCommands.setViewport(
        fitCanvasViewport(
          editor.getState(),
          measuredSize(canvasHostRef.current)
        )
      )
    );
  }, []);

  const handleMiniMapNavigate = useCallback(
    (request: CanvasMiniMapNavigationRequest) => {
      setMiniMapDragging(request.phase !== 'end');
      editorRef.current?.dispatch(canvasCommands.setViewport(request.viewport));
    },
    []
  );

  const flushBeforeLeave = useCallback(async (): Promise<boolean> => {
    if (imageToolBusyRef.current) {
      setNotice('图片工具仍在处理，请等待完成后再离开。');
      return false;
    }
    if (!((await agentPanelRef.current?.prepareToLeave()) ?? true))
      return false;
    const editor = editorRef.current;
    if (!editor) return true;
    return canLeaveCreativeCanvasAfterFlush(await editor.flush());
  }, []);

  const handlePersistAgentSessions = useCallback(
    async (
      sessions: readonly CreativeChatSessionReference[],
      activeSessionId: string | null
    ) => {
      const editor = editorRef.current;
      if (!editor) throw new Error('画布尚未载入，无法保存 Agent 会话。');
      await editor.persistAgentSessions(sessions, activeSessionId);
    },
    []
  );

  const handleAgentSessionsChange = useCallback(
    (
      sessions: readonly CreativeChatSessionReference[],
      activeSessionId: string | null
    ) => {
      setAgentDocumentState({
        sessions: structuredClone([...sessions]),
        activeSessionId,
      });
    },
    []
  );

  const handleOpenModelSettings = useCallback(async () => {
    if (await flushBeforeLeave()) navigate('/models?section=models');
  }, [flushBeforeLeave, navigate]);

  const handleOpenWorkflowCenter = useCallback(async () => {
    if (await flushBeforeLeave()) navigate(CREATIVE_STUDIO_WORKFLOWS_PATH);
  }, [flushBeforeLeave, navigate]);

  const workflowRunner = useMemo<CreativeWorkflowRunnerPort>(
    () => ({
      async start(input) {
        await workflowRuntime.controller.start(input);
      },
    }),
    [workflowRuntime.controller]
  );

  const dismissInteractionOverlays = useCallback(() => {
    setContextMenu(null);
    setCreateNodeMenu(null);
  }, []);

  const openCreateNodeMenu = useCallback(
    (
      worldPosition: CanvasPoint,
      connection: ConnectionCreateNodeIntent | null = null
    ) => {
      const editor = editorRef.current;
      if (!editor) return;
      setContextMenu(null);
      setCreateNodeMenu({
        worldPosition: { ...worldPosition },
        clientPosition: canvasToClient(
          worldPosition,
          editor.getState().viewport
        ),
        connection,
      });
    },
    []
  );

  const insertAssetAtWorld = useCallback(
    (asset: CreativeAsset, worldPosition: CanvasPoint, asPanorama = false) => {
      const editor = editorRef.current;
      if (!editor) throw new Error('画布尚未载入，无法插入素材。');
      const state = editor.getState();
      const kind = asPanorama ? 'panorama' : asset.kind;
      const position = centeredNodePosition(kind, worldPosition);
      const node = asPanorama
        ? {
            ...createCreativeCanvasProductNode(
              'panorama',
              state,
              measuredSize(canvasHostRef.current),
              { position }
            ),
            data: {
              assetId: asset.id,
              projection: 'equirectangular' as const,
              yaw: 0,
              pitch: 0,
              fieldOfView: 75,
            },
          }
        : creativeNodeFromAsset(
            asset,
            state,
            measuredSize(canvasHostRef.current),
            { position }
          );
      knownAssetsRef.current = new Map(knownAssetsRef.current).set(
        asset.id,
        asset
      );
      editor.dispatch(canvasCommands.addNode(node));
      setNotice(
        `已将“${asset.title}”插入为${asPanorama ? '全景图' : '素材'}节点。`
      );
      void assets.reload();
    },
    [assets]
  );

  const importCanvasFile = useCallback(
    async (
      file: File,
      worldPosition: CanvasPoint,
      panoramaChoice: 'after-upload-if-2-to-1' | 'not-applicable'
    ) => {
      if (assetImportBusyRef.current) {
        setNotice('已有素材正在上传，请等待完成。');
        return;
      }
      assetImportBusyRef.current = true;
      setAssetImportBusy(true);
      setNotice(`正在上传“${file.name}”…`);
      try {
        const asset = await creativeAssetClient.upload(
          file,
          { title: file.name, inLibrary: true, tags: ['canvas-import'] },
          undefined,
          (progress) =>
            setNotice(`正在上传“${file.name}” ${Math.round(progress)}%`)
        );
        if (
          panoramaChoice === 'after-upload-if-2-to-1' &&
          isTwoToOneImage(asset)
        ) {
          setPendingPanoramaChoice({
            asset,
            worldPosition: { ...worldPosition },
          });
          setNotice('检测到真实 2:1 图片，请选择普通图片或全景图节点。');
          return;
        }
        insertAssetAtWorld(asset, worldPosition);
      } catch (error) {
        setNotice(error instanceof Error ? error.message : String(error));
      } finally {
        assetImportBusyRef.current = false;
        setAssetImportBusy(false);
      }
    },
    [insertAssetAtWorld]
  );

  const resolveCanvasImageAsset = useCallback(
    async (node: Extract<CreativeCanvasNode, { type: 'image' }>) => {
      const assetId = node.data.assetId?.trim();
      if (!assetId) throw new Error('该图片节点尚未关联真实素材。');
      const cached = knownAssetsRef.current.get(assetId);
      const asset = cached ?? (await creativeAssetClient.get(assetId));
      if (asset.kind !== 'image') {
        throw new Error('该节点关联的素材不是图片，已停止图片操作。');
      }
      knownAssetsRef.current = new Map(knownAssetsRef.current).set(
        asset.id,
        asset
      );
      return asset;
    },
    []
  );

  const handleOpenImageCrop = useCallback(
    async (node: Extract<CreativeCanvasNode, { type: 'image' }>) => {
      if (imageToolBusyRef.current || assetImportBusyRef.current) {
        setNotice('已有图片或素材操作正在进行，请等待完成。');
        return;
      }
      setImageCropError(null);
      try {
        const asset = await resolveCanvasImageAsset(node);
        if (activeProjectIdRef.current !== projectId) return;
        setImageCropProgress(null);
        setPendingImageCrop({ nodeId: node.id, asset });
      } catch (error) {
        if (activeProjectIdRef.current !== projectId) return;
        setNotice(error instanceof Error ? error.message : String(error));
      }
    },
    [projectId, resolveCanvasImageAsset]
  );

  const handleDownloadImage = useCallback(
    async (node: Extract<CreativeCanvasNode, { type: 'image' }>) => {
      try {
        const asset = await resolveCanvasImageAsset(node);
        if (activeProjectIdRef.current !== projectId) return;
        const anchor = document.createElement('a');
        anchor.href = asset.originalUrl;
        anchor.download = creativeAssetDownloadName(asset);
        anchor.rel = 'noopener noreferrer';
        anchor.click();
      } catch (error) {
        if (activeProjectIdRef.current !== projectId) return;
        setNotice(error instanceof Error ? error.message : String(error));
      }
    },
    [projectId, resolveCanvasImageAsset]
  );

  const handleOpenImageSplit = useCallback(
    async (node: Extract<CreativeCanvasNode, { type: 'image' }>) => {
      if (imageToolBusyRef.current || assetImportBusyRef.current) {
        setNotice('已有图片或素材操作正在进行，请等待完成。');
        return;
      }
      setImageSplitError(null);
      try {
        const asset = await resolveCanvasImageAsset(node);
        if (activeProjectIdRef.current !== projectId) return;
        setImageSplitProgress(null);
        setPendingImageSplit({ nodeId: node.id, asset });
      } catch (error) {
        if (activeProjectIdRef.current !== projectId) return;
        setNotice(error instanceof Error ? error.message : String(error));
      }
    },
    [projectId, resolveCanvasImageAsset]
  );

  const closeImageCrop = useCallback(() => {
    if (imageToolBusyRef.current) return;
    setPendingImageCrop(null);
    setImageCropProgress(null);
    setImageCropError(null);
  }, []);

  const handleConfirmImageCrop = useCallback(
    async (crop: CreativeImageCropRect) => {
      const request = pendingImageCrop;
      const editor = editorRef.current;
      if (!request || !editor || imageToolBusyRef.current) return;
      if (assetImportBusyRef.current) {
        setImageCropError('另一个素材上传仍在进行，请等待完成后重试。');
        return;
      }

      const controller = new AbortController();
      imageToolAbortRef.current = controller;
      imageToolBusyRef.current = true;
      setImageCropBusy(true);
      setImageCropProgress(0);
      setImageCropError(null);
      let uploadedAsset: CreativeAsset | null = null;

      try {
        const cropped = await cropCreativeImageAsset({
          asset: request.asset,
          crop,
          signal: controller.signal,
        });
        const uploaded = await uploadCreativeImageCrop({
          port: creativeAssetClient,
          source: request.asset,
          file: cropped.file,
          operationId: uuidv7(),
          signal: controller.signal,
          onProgress: setImageCropProgress,
        });
        uploadedAsset = uploaded.asset;
        controller.signal.throwIfAborted();
        if (activeProjectIdRef.current !== projectId) {
          throw new DOMException('Project changed', 'AbortError');
        }

        const current = editor.getState();
        const source = current.document.nodes.find(
          (node): node is Extract<CreativeCanvasNode, { type: 'image' }> =>
            node.id === request.nodeId && node.type === 'image'
        );
        if (!source || source.data.assetId !== request.asset.id) {
          throw new Error(
            '原图片节点已被删除或替换；裁剪素材已保存在素材库中。'
          );
        }

        const position = nextDerivedImagePosition(
          current.document,
          source,
          CREATIVE_CANVAS_PRODUCT_NODE_SIZES.image
        );
        const derived = creativeNodeFromAsset(
          uploaded.asset,
          current,
          measuredSize(canvasHostRef.current),
          { position }
        );
        if (derived.type !== 'image') {
          throw new Error('裁剪结果未能构造成图片节点。');
        }
        const connection = {
          sourceNodeId: source.id,
          targetNodeId: derived.id,
        };
        const validation = validateCanvasConnection(
          {
            ...current.document,
            nodes: [...current.document.nodes, derived],
          },
          connection
        );
        if (!validation.ok) {
          throw new Error(
            `无法连接裁剪结果：${connectionErrorMessage(validation.code)}。`
          );
        }

        knownAssetsRef.current = new Map(knownAssetsRef.current).set(
          uploaded.asset.id,
          uploaded.asset
        );
        const at = Date.now();
        const mergeKey = `image-crop:${source.id}:${uploaded.asset.id}`;
        editor.dispatch(canvasCommands.addNode(derived, { at, mergeKey }));
        editor.dispatch(
          canvasCommands.connect(source.id, derived.id, {
            sourceHandle: 'source',
            targetHandle: 'target',
            at,
            mergeKey,
          })
        );
        editor.dispatch(canvasCommands.setSelection([derived.id]));
        setPendingImageCrop(null);
        setImageCropProgress(null);
        void assets.reload();

        const flush = await editor.flush();
        if (flush.status === 'saved' || flush.status === 'noop') {
          setNotice(
            uploaded.recoveredAfterResponseLoss
              ? '上传响应中断后已找回真实裁剪素材，并将派生节点保存到画布。'
              : '已裁剪真实原图，创建派生图片节点并保存连线。'
          );
        } else {
          setNotice(`裁剪素材已上传，但画布保存失败：${flush.error.message}`);
        }
      } catch (error) {
        const aborted =
          controller.signal.aborted ||
          (error instanceof Error && error.name === 'AbortError');
        if (!aborted && activeProjectIdRef.current === projectId) {
          const message =
            error instanceof Error ? error.message : String(error);
          if (uploadedAsset) {
            setPendingImageCrop(null);
            setImageCropProgress(null);
            void assets.reload();
            setNotice(message);
          } else {
            setImageCropError(message);
          }
        }
      } finally {
        if (imageToolAbortRef.current === controller) {
          imageToolAbortRef.current = null;
          imageToolBusyRef.current = false;
          setImageCropBusy(false);
        }
      }
    },
    [assets, pendingImageCrop, projectId]
  );

  const closeImageSplit = useCallback(() => {
    if (imageToolBusyRef.current) return;
    setPendingImageSplit(null);
    setImageSplitProgress(null);
    setImageSplitError(null);
  }, []);

  const handleConfirmImageSplit = useCallback(
    async (params: CreativeImageSplitParams) => {
      const request = pendingImageSplit;
      const editor = editorRef.current;
      if (!request || !editor || imageToolBusyRef.current) return;
      if (assetImportBusyRef.current) {
        setImageSplitError('另一个素材上传仍在进行，请等待完成后重试。');
        return;
      }

      const controller = new AbortController();
      const operationId = uuidv7();
      imageToolAbortRef.current = controller;
      imageToolBusyRef.current = true;
      setImageSplitBusy(true);
      setImageSplitProgress(0);
      setImageSplitError(null);
      let uploadedPieces: readonly UploadedCreativeImageSplitPiece[] | null =
        null;
      let canvasMutated = false;

      try {
        const files = await splitCreativeImageAsset({
          asset: request.asset,
          params,
          signal: controller.signal,
        });
        const uploaded = await uploadCreativeImageSplit({
          port: creativeAssetClient,
          source: request.asset,
          pieces: files,
          operationId,
          signal: controller.signal,
          onProgress: setImageSplitProgress,
        });
        uploadedPieces = uploaded;
        controller.signal.throwIfAborted();
        if (activeProjectIdRef.current !== projectId) {
          throw new DOMException('Project changed', 'AbortError');
        }

        const current = editor.getState();
        const source = current.document.nodes.find(
          (node): node is Extract<CreativeCanvasNode, { type: 'image' }> =>
            node.id === request.nodeId && node.type === 'image'
        );
        if (!source || source.data.assetId !== request.asset.id) {
          throw new Error('原图片节点已被删除或替换，未向画布写入切图结果。');
        }

        const rows = creativeImageSplitRows(params);
        const columns = creativeImageSplitColumns(params);
        const layout = createCreativeImageSplitCanvasLayout(
          current.document,
          source,
          rows,
          columns
        );
        const derivedNodes: Extract<CreativeCanvasNode, { type: 'image' }>[] =
          [];
        let factoryState = current;
        for (const piece of uploaded) {
          const derived = creativeNodeFromAsset(
            piece.asset,
            factoryState,
            measuredSize(canvasHostRef.current),
            {
              position: creativeImageSplitNodePosition(
                layout,
                piece.row,
                piece.column
              ),
              size: layout.cellSize,
            }
          );
          if (derived.type !== 'image') {
            throw new Error('切图结果未能构造成图片节点。');
          }
          derivedNodes.push(derived);
          factoryState = {
            ...factoryState,
            document: {
              ...factoryState.document,
              nodes: [...factoryState.document.nodes, derived],
            },
          };
        }

        const prospectiveDocument = {
          ...current.document,
          nodes: [...current.document.nodes, ...derivedNodes],
        };
        for (const derived of derivedNodes) {
          const validation = validateCanvasConnection(prospectiveDocument, {
            sourceNodeId: source.id,
            targetNodeId: derived.id,
          });
          if (!validation.ok) {
            throw new Error(
              `无法连接切图结果：${connectionErrorMessage(validation.code)}。`
            );
          }
        }

        const nextAssets = new Map(knownAssetsRef.current);
        for (const piece of uploaded)
          nextAssets.set(piece.asset.id, piece.asset);
        knownAssetsRef.current = nextAssets;

        const at = Date.now();
        const mergeKey = `image-split:${source.id}:${operationId}`;
        canvasMutated = true;
        for (const derived of derivedNodes) {
          editor.dispatch(canvasCommands.addNode(derived, { at, mergeKey }));
        }
        for (const derived of derivedNodes) {
          editor.dispatch(
            canvasCommands.connect(source.id, derived.id, {
              sourceHandle: 'source',
              targetHandle: 'target',
              at,
              mergeKey,
            })
          );
        }
        editor.dispatch(
          canvasCommands.setSelection(derivedNodes.map((node) => node.id))
        );
        setPendingImageSplit(null);
        setImageSplitProgress(null);
        void assets.reload();

        const flush = await editor.flush();
        if (flush.status === 'saved' || flush.status === 'noop') {
          setNotice(
            uploaded.some((piece) => piece.recoveredAfterResponseLoss)
              ? `上传响应中断后已找回切图素材，创建并保存 ${derivedNodes.length} 个图片子节点。`
              : `已切分真实原图，创建并保存 ${derivedNodes.length} 个图片子节点及连线。`
          );
        } else {
          setNotice(`切图素材已上传，但画布保存失败：${flush.error.message}`);
        }
      } catch (error) {
        const aborted =
          controller.signal.aborted ||
          (error instanceof Error && error.name === 'AbortError');
        let message = error instanceof Error ? error.message : String(error);
        if (uploadedPieces && !canvasMutated) {
          try {
            await removeUploadedCreativeImageSplit(
              creativeAssetClient,
              uploadedPieces
            );
          } catch (cleanupError) {
            const cleanupMessage =
              cleanupError instanceof Error
                ? cleanupError.message
                : String(cleanupError);
            message = `${message}；${cleanupMessage}`;
          }
          void assets.reload();
        }
        if (!aborted && activeProjectIdRef.current === projectId) {
          if (canvasMutated) {
            setPendingImageSplit(null);
            setImageSplitProgress(null);
            setNotice(message);
          } else {
            setImageSplitError(message);
          }
        }
      } finally {
        if (imageToolAbortRef.current === controller) {
          imageToolAbortRef.current = null;
          imageToolBusyRef.current = false;
          setImageSplitBusy(false);
        }
      }
    },
    [assets, pendingImageSplit, projectId]
  );

  const handleOpenImageMaskEdit = useCallback(
    async (node: Extract<CreativeCanvasNode, { type: 'image' }>) => {
      const runtime = imageMaskRuntimeRef.current?.snapshot();
      const runtimeBlocked =
        !imageMaskRuntimeReady ||
        !runtime ||
        runtime.submittingCount > 0 ||
        runtime.recoveringCount > 0 ||
        runtime.submissionFailures.length > 0 ||
        runtime.requestError !== null ||
        runtime.entries.some(
          (entry) =>
            entry.task.status === 'queued' || entry.task.status === 'running'
        );
      if (
        imageToolBusyRef.current ||
        assetImportBusyRef.current ||
        runtimeBlocked
      ) {
        setNotice(
          runtimeBlocked
            ? '已有局部编辑任务正在运行、恢复或等待确认，请先处理该任务。'
            : '已有图片或素材操作正在进行，请等待完成。'
        );
        return;
      }
      setImageMaskError(null);
      try {
        const asset = await resolveCanvasImageAsset(node);
        if (activeProjectIdRef.current !== projectId) return;
        setImageMaskProgress(null);
        setImageMaskModel((previous) =>
          preferredCanvasImageMaskEditModel(
            imageMaskModelOptions,
            previous,
            asset
          )
        );
        setPendingImageMaskEdit({ nodeId: node.id, asset, submission: null });
      } catch (error) {
        if (activeProjectIdRef.current !== projectId) return;
        setNotice(error instanceof Error ? error.message : String(error));
      }
    },
    [
      imageMaskModelOptions,
      imageMaskRuntimeReady,
      projectId,
      resolveCanvasImageAsset,
    ]
  );

  const closeImageMaskEdit = useCallback(() => {
    if (imageToolBusyRef.current || pendingImageMaskEdit?.submission) return;
    setPendingImageMaskEdit(null);
    setImageMaskProgress(null);
    setImageMaskError(null);
  }, [pendingImageMaskEdit?.submission]);

  const applyImageMaskAdmission = useCallback(
    (
      result: Awaited<
        ReturnType<CanvasImageMaskEditRuntimeBridgeHandle['submit']>
      >,
      plan: PreparedCreativeWorkbenchRun
    ) => {
      if (result.kind === 'admitted') {
        setPendingImageMaskEdit(null);
        setImageMaskProgress(null);
        setImageMaskError(null);
        setNotice('局部编辑任务已安全提交；配置节点会持续显示真实后端状态。');
        return;
      }
      setPendingImageMaskEdit((current) =>
        current
          ? {
              ...current,
              submission: {
                plan,
                reference: canvasImageMaskEditReferenceFromPlan(plan),
                failureOrder: result.order,
              },
            }
          : current
      );
      setImageMaskError(
        `任务提交结果尚未确认：${result.error.message}。请安全重试同一任务，或确认服务器不存在后放弃。`
      );
    },
    []
  );

  const handleConfirmImageMaskEdit = useCallback(
    async (input: CreativeImageMaskEditSubmit) => {
      const request = pendingImageMaskEdit;
      const editor = editorRef.current;
      const runtime = imageMaskRuntimeRef.current;
      if (!request || !editor || !runtime || imageToolBusyRef.current) return;

      imageToolBusyRef.current = true;
      setImageMaskBusy(true);
      setImageMaskError(null);

      if (request.submission) {
        try {
          const result = await runtime.retrySubmission(
            request.submission.failureOrder,
            request.submission.plan.input.idempotencyKey
          );
          applyImageMaskAdmission(result, request.submission.plan);
        } catch (error) {
          setImageMaskError(
            error instanceof Error ? error.message : String(error)
          );
        } finally {
          imageToolBusyRef.current = false;
          setImageMaskBusy(false);
        }
        return;
      }

      if (assetImportBusyRef.current) {
        imageToolBusyRef.current = false;
        setImageMaskBusy(false);
        setImageMaskError('另一个素材上传仍在进行，请等待完成后重试。');
        return;
      }

      const controller = new AbortController();
      imageToolAbortRef.current = controller;
      setImageMaskProgress(0);
      let uploadedReference: CreativeAsset | null = null;
      let prepared: ReturnType<typeof prepareCanvasImageMaskEdit> | null = null;
      let canvasOwned = false;

      try {
        const marked = await buildCreativeImageMaskReference({
          asset: request.asset,
          selection: input.selection,
          signal: controller.signal,
        });
        const uploaded = await uploadCreativeImageMaskReference({
          port: creativeAssetClient,
          source: request.asset,
          file: marked.file,
          operationId: uuidv7(),
          signal: controller.signal,
          onProgress: setImageMaskProgress,
        });
        uploadedReference = uploaded.asset;
        controller.signal.throwIfAborted();
        if (activeProjectIdRef.current !== projectId) {
          throw new DOMException('Project changed', 'AbortError');
        }

        const current = editor.getState();
        const source = current.document.nodes.find(
          (node): node is Extract<CreativeCanvasNode, { type: 'image' }> =>
            node.id === request.nodeId && node.type === 'image'
        );
        if (!source || source.data.assetId !== request.asset.id) {
          throw new Error('原图片节点已被删除或替换，未创建局部编辑任务。');
        }
        prepared = prepareCanvasImageMaskEdit({
          projectId,
          state: current,
          viewportSize: measuredSize(canvasHostRef.current),
          sourceNode: source,
          sourceAsset: request.asset,
          markedReference: uploaded.asset,
          referenceDimensions: { width: marked.width, height: marked.height },
          catalog: modelCatalog,
          model: input.model,
          userPrompt: input.prompt,
        });

        const at = Date.now();
        const mergeKey = `image-mask-edit:${source.id}:${prepared.plan.input.idempotencyKey}`;
        editor.dispatch(
          canvasCommands.addNode(prepared.configNode, { at, mergeKey })
        );
        editor.dispatch(
          canvasCommands.connect(source.id, prepared.configNode.id, {
            sourceHandle: prepared.connection.sourceHandle,
            targetHandle: prepared.connection.targetHandle,
            at,
            mergeKey,
          })
        );
        editor.dispatch(canvasCommands.setSelection([prepared.configNode.id]));
        canvasOwned = true;

        const result = await runtime.submit(prepared.plan);
        applyImageMaskAdmission(result, prepared.plan);
        if (uploaded.recoveredAfterResponseLoss && result.kind === 'admitted') {
          setNotice('上传响应中断后已找回标记参考图，并安全提交局部编辑任务。');
        }
      } catch (error) {
        const aborted =
          controller.signal.aborted ||
          (error instanceof Error && error.name === 'AbortError');
        let message = error instanceof Error ? error.message : String(error);
        if (uploadedReference && !canvasOwned) {
          try {
            await removeCreativeImageMaskReference(
              creativeAssetClient,
              uploadedReference
            );
          } catch (cleanupError) {
            const cleanupMessage =
              cleanupError instanceof Error
                ? cleanupError.message
                : String(cleanupError);
            message = `${message}；${cleanupMessage}`;
          }
        }
        if (canvasOwned && prepared && !aborted) {
          try {
            // Once the config exists, an unclassified transport outcome must
            // remain recoverable. A later mount will resolve the exact key or
            // clean only an authoritative 404.
            await editor.addPendingTask(prepared.plan.input.idempotencyKey);
            void runtime
              .recoverTask(canvasImageMaskEditReferenceFromPlan(prepared.plan))
              .catch((recoveryError) =>
                setNotice(
                  recoveryError instanceof Error
                    ? recoveryError.message
                    : String(recoveryError)
                )
              );
          } catch (saveError) {
            message = `${message}；${
              saveError instanceof Error ? saveError.message : String(saveError)
            }`;
          }
        }
        if (!aborted && activeProjectIdRef.current === projectId) {
          if (canvasOwned) {
            setPendingImageMaskEdit(null);
            setImageMaskProgress(null);
            setNotice(`任务接收状态未确认，已保留同一任务恢复标记：${message}`);
          } else {
            setImageMaskError(message);
          }
        }
      } finally {
        if (imageToolAbortRef.current === controller)
          imageToolAbortRef.current = null;
        imageToolBusyRef.current = false;
        setImageMaskBusy(false);
      }
    },
    [applyImageMaskAdmission, modelCatalog, pendingImageMaskEdit, projectId]
  );

  const abandonImageMaskSubmission = useCallback(async () => {
    const request = pendingImageMaskEdit;
    const runtime = imageMaskRuntimeRef.current;
    const editor = editorRef.current;
    if (!request?.submission || !runtime || !editor || imageToolBusyRef.current)
      return;
    imageToolBusyRef.current = true;
    setImageMaskBusy(true);
    setImageMaskError(null);
    try {
      const exists = await runtime.taskExists(request.submission.reference);
      if (exists) {
        const result = await runtime.retrySubmission(
          request.submission.failureOrder,
          request.submission.plan.input.idempotencyKey
        );
        applyImageMaskAdmission(result, request.submission.plan);
        if (result.kind === 'admitted') {
          setNotice('服务器已存在该任务，已安全恢复而未重复创建。');
        }
        return;
      }
      await orphanCanvasImageMaskEditTask({
        editor,
        projectId,
        reference: request.submission.reference,
      });
      setImageMaskRuntimeEpoch((value) => value + 1);
      setImageMaskRuntime(INITIAL_IMAGE_MASK_RUNTIME);
      setPendingImageMaskEdit(null);
      setImageMaskProgress(null);
      setNotice('已确认服务器不存在该任务；配置节点记录为失败并清理恢复标记。');
    } catch (error) {
      setImageMaskError(error instanceof Error ? error.message : String(error));
    } finally {
      imageToolBusyRef.current = false;
      setImageMaskBusy(false);
    }
  }, [applyImageMaskAdmission, pendingImageMaskEdit, projectId]);

  const retryImageMaskRuntimeTask = useCallback(
    async (taskId: string) => {
      const runtime = imageMaskRuntimeRef.current;
      if (!runtime || imageMaskRuntimeActionBusy) return;
      setImageMaskRuntimeActionBusy(true);
      try {
        await runtime.retryTask(taskId);
      } catch (error) {
        setNotice(error instanceof Error ? error.message : String(error));
      } finally {
        setImageMaskRuntimeActionBusy(false);
      }
    },
    [imageMaskRuntimeActionBusy]
  );

  const cancelImageMaskRuntimeTask = useCallback(
    async (taskId: string) => {
      const runtime = imageMaskRuntimeRef.current;
      if (!runtime || imageMaskRuntimeActionBusy) return;
      setImageMaskRuntimeActionBusy(true);
      try {
        await runtime.cancelTask(taskId);
      } catch (error) {
        setNotice(error instanceof Error ? error.message : String(error));
      } finally {
        setImageMaskRuntimeActionBusy(false);
      }
    },
    [imageMaskRuntimeActionBusy]
  );

  const insertClipboardText = useCallback(
    (text: string, worldPosition: CanvasPoint) => {
      const editor = editorRef.current;
      const normalized = text.trim();
      if (!editor || !normalized) return false;
      const state = editor.getState();
      const node = createCreativeCanvasProductNode(
        'text',
        state,
        measuredSize(canvasHostRef.current),
        { position: centeredNodePosition('text', worldPosition) }
      );
      editor.dispatch(
        canvasCommands.addNode({
          ...node,
          data: { ...node.data, text: normalized },
        })
      );
      setNotice('已从真实系统剪贴板插入文本。');
      return true;
    },
    []
  );

  const readSystemClipboard = useCallback(
    async (worldPosition: CanvasPoint) => {
      try {
        if (typeof navigator === 'undefined' || !navigator.clipboard) {
          throw new Error('当前运行环境不提供系统剪贴板读取能力。');
        }
        if (typeof navigator.clipboard.read === 'function') {
          const items = await navigator.clipboard.read();
          for (const item of items) {
            const mediaType = item.types.find(
              (type) => type.startsWith('image/') || type.startsWith('video/')
            );
            if (mediaType) {
              const blob = await item.getType(mediaType);
              const extension = mediaType.split('/')[1]?.split('+')[0] || 'bin';
              const file = new File(
                [blob],
                `clipboard-${new Date().toISOString().replace(/[:.]/g, '-')}.${extension}`,
                { type: mediaType }
              );
              await importCanvasFile(
                file,
                worldPosition,
                mediaType.startsWith('image/')
                  ? 'after-upload-if-2-to-1'
                  : 'not-applicable'
              );
              return;
            }
            if (item.types.includes('text/plain')) {
              const text = await (await item.getType('text/plain')).text();
              if (insertClipboardText(text, worldPosition)) return;
            }
          }
        }
        if (typeof navigator.clipboard.readText === 'function') {
          const text = await navigator.clipboard.readText();
          if (insertClipboardText(text, worldPosition)) return;
        }
        setNotice('系统剪贴板中没有可插入的真实文本、图片或视频。');
      } catch (error) {
        setNotice(error instanceof Error ? error.message : String(error));
      }
    },
    [importCanvasFile, insertClipboardText]
  );

  const handleOpenDirector = useCallback(
    async (requestedNodeId?: string) => {
      const editor = editorRef.current;
      if (!editor || save.revision === null) return;
      const directors = editor
        .getState()
        .document.nodes.filter((node) => node.type === 'director');
      handleBottomViewChange('timeline');
      if (directors.length === 0) {
        setNotice('请先添加导演节点，再进入 3D 导演台。');
        return;
      }
      if (directors.length > 1) {
        editor.dispatch(
          canvasCommands.setSelection(directors.map((node) => node.id))
        );
        setNotice('项目存在多个导演节点。请只保留一个，再进入 3D 导演台。');
        return;
      }
      const director = directors[0];
      if (requestedNodeId && requestedNodeId !== director.id) {
        setNotice('请求的导演节点已不存在，请从时间线面板重新打开。');
        return;
      }
      editor.dispatch(canvasCommands.setSelection([director.id]));
      if (await flushBeforeLeave()) {
        navigate(creativeStudioDirectorProjectPath(projectId));
      }
    },
    [
      flushBeforeLeave,
      handleBottomViewChange,
      navigate,
      projectId,
      save.revision,
    ]
  );

  const handleIntegrationIntent = useCallback(
    async (intent: CanvasIntegrationIntent) => {
      switch (intent.type) {
        case 'transient-ui/dismiss':
          dismissInteractionOverlays();
          return;
        case 'context-menu/open': {
          const nodeId =
            intent.target.kind === 'node' ? intent.target.nodeId : null;
          const node = nodeId
            ? editorRef.current
                ?.getState()
                .document.nodes.find((candidate) => candidate.id === nodeId)
            : null;
          setCreateNodeMenu(null);
          setContextMenu({
            target: intent.target,
            clientPosition: { ...intent.clientPosition },
            ...(node ? { nodeLocked: node.locked } : {}),
          });
          return;
        }
        case 'canvas/create-node-menu/open':
          openCreateNodeMenu(intent.worldPosition);
          return;
        case 'connection/create-node-menu/open':
          openCreateNodeMenu(intent.worldPosition, intent);
          return;
        case 'connection/rejected':
          setNotice(`无法创建连接：${connectionErrorMessage(intent.code)}。`);
          return;
        case 'connection/created':
          setNotice('已创建连接。');
          return;
        case 'node/open':
          dispatch(canvasCommands.setSelection([intent.nodeId]));
          dismissInteractionOverlays();
          if (intent.mode === 'open-director') {
            await handleOpenDirector(intent.nodeId);
            return;
          }
          persistPanels(
            withCreativeCanvasRightView(panelsRef.current, 'properties')
          );
          setNotice('已在属性面板打开所选节点。');
          return;
        case 'system-clipboard/read': {
          const editor = editorRef.current;
          if (!editor) return;
          const worldPosition =
            intent.worldPosition ??
            clientToCanvas(
              { x: viewportSize.width / 2, y: viewportSize.height / 2 },
              editor.getState().viewport
            );
          await readSystemClipboard(worldPosition);
          return;
        }
        case 'asset/import-file':
          await importCanvasFile(
            intent.file,
            intent.worldPosition,
            intent.panoramaChoice
          );
          return;
        case 'asset/import-feedback': {
          const first = intent.rejected[0];
          const rejected = first
            ? `${intent.rejected.length} 个文件未导入（${first.fileName}: ${manualUploadRejectionMessage(first.reason)}）`
            : '';
          const ignored = intent.ignoredAcceptedFileNames.length
            ? `${intent.ignoredAcceptedFileNames.length} 个额外文件按源产品规则未处理`
            : '';
          setNotice([rejected, ignored].filter(Boolean).join('；'));
          return;
        }
      }
    },
    [
      dismissInteractionOverlays,
      dispatch,
      handleOpenDirector,
      importCanvasFile,
      openCreateNodeMenu,
      persistPanels,
      readSystemClipboard,
      viewportSize.height,
      viewportSize.width,
    ]
  );

  const handleContextAction = useCallback(
    async (action: CanvasContextAction) => {
      const editor = editorRef.current;
      const menu = contextMenu;
      if (!editor || !menu) return;
      const resolution = resolveCanvasContextAction(
        editor.getState(),
        menu.target,
        action
      );
      dismissInteractionOverlays();
      for (const command of resolution.commands) editor.dispatch(command);
      for (const intent of resolution.intents)
        await handleIntegrationIntent(intent);
    },
    [contextMenu, dismissInteractionOverlays, handleIntegrationIntent]
  );

  const handleOpenCreateNodeMenuFromContext = useCallback(() => {
    const editor = editorRef.current;
    if (!editor || contextMenu?.target.kind !== 'canvas') return;
    openCreateNodeMenu(
      clientToCanvas(contextMenu.clientPosition, editor.getState().viewport)
    );
  }, [contextMenu, openCreateNodeMenu]);

  const handlePasteFromContext = useCallback(async () => {
    const editor = editorRef.current;
    if (!editor || contextMenu?.target.kind !== 'canvas') return;
    const worldPosition = clientToCanvas(
      contextMenu.clientPosition,
      editor.getState().viewport
    );
    dismissInteractionOverlays();
    await readSystemClipboard(worldPosition);
  }, [contextMenu, dismissInteractionOverlays, readSystemClipboard]);

  const handleSelectCreatedNode = useCallback(
    (kind: CreativeCanvasNodeKind) => {
      const editor = editorRef.current;
      const menu = createNodeMenu;
      if (!editor || !menu || save.revision === null) return;
      const state = editor.getState();
      const directors =
        kind === 'director'
          ? state.document.nodes.filter((node) => node.type === 'director')
          : [];
      if (directors.length > 1) {
        editor.dispatch(
          canvasCommands.setSelection(directors.map((node) => node.id))
        );
        handleBottomViewChange('timeline');
        setNotice('项目存在多个导演节点，请先处理冲突，未创建新的导演节点。');
        dismissInteractionOverlays();
        return;
      }
      const reusedDirector = directors[0] ?? null;
      const node =
        reusedDirector ??
        createCreativeCanvasProductNode(
          kind,
          state,
          measuredSize(canvasHostRef.current),
          { position: centeredNodePosition(kind, menu.worldPosition) }
        );

      if (menu.connection) {
        const sourceNodeId =
          menu.connection.fixedHandle === 'source'
            ? menu.connection.fixedNodeId
            : node.id;
        const targetNodeId =
          menu.connection.fixedHandle === 'source'
            ? node.id
            : menu.connection.fixedNodeId;
        const candidateDocument = {
          ...state.document,
          nodes: reusedDirector
            ? state.document.nodes
            : [...state.document.nodes, node],
        };
        const validation = validateCanvasConnection(candidateDocument, {
          sourceNodeId,
          targetNodeId,
        });
        if (!validation.ok) {
          setNotice(
            `无法创建连接：${connectionErrorMessage(validation.code)}。`
          );
          return;
        }

        const at = Date.now();
        const mergeKey = `create-connected:${node.id}`;
        if (!reusedDirector) {
          editor.dispatch(canvasCommands.addNode(node, { at, mergeKey }));
        }
        editor.dispatch(
          canvasCommands.connect(sourceNodeId, targetNodeId, {
            at,
            mergeKey,
            sourceHandle:
              menu.connection.fixedHandle === 'source'
                ? menu.connection.fixedHandleId
                : 'source',
            targetHandle:
              menu.connection.fixedHandle === 'target'
                ? menu.connection.fixedHandleId
                : 'target',
          })
        );
        editor.dispatch(canvasCommands.setSelection([node.id]));
        setNotice(
          reusedDirector
            ? '已复用项目唯一的导演节点并完成连接。'
            : '已创建节点并完成连接。'
        );
      } else {
        if (reusedDirector) {
          editor.dispatch(canvasCommands.setSelection([node.id]));
          setNotice('项目已有唯一导演节点，已为你选中。');
        } else {
          editor.dispatch(canvasCommands.addNode(node));
          setNotice('已在指定位置创建节点。');
        }
      }
      if (kind === 'director') {
        handleBottomViewChange('timeline');
      }
      dismissInteractionOverlays();
    },
    [
      createNodeMenu,
      dismissInteractionOverlays,
      handleBottomViewChange,
      save.revision,
    ]
  );

  const resolvePendingPanoramaChoice = useCallback(
    (asPanorama: boolean) => {
      const choice = pendingPanoramaChoice;
      if (!choice) return;
      setPendingPanoramaChoice(null);
      try {
        insertAssetAtWorld(choice.asset, choice.worldPosition, asPanorama);
      } catch (error) {
        setNotice(error instanceof Error ? error.message : String(error));
      }
    },
    [insertAssetAtWorld, pendingPanoramaChoice]
  );

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

  const handleInsertAssets = useCallback(
    (selectedAssets: readonly CreativeAsset[]) => {
      const insertion = prepareCenteredInsertion();
      if (!insertion) return;
      const { editor, viewportSize } = insertion;
      let { state } = insertion;
      let inserted = 0;
      const errors: string[] = [];
      for (const asset of selectedAssets) {
        try {
          const node = creativeNodeFromAsset(
            asset,
            state,
            viewportSize,
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
    },
    [prepareCenteredInsertion]
  );

  const handleInsertWorkflowResults = useCallback(
    async (run: WorkflowRunAggregateV1) => {
      if (workflowInsertingRunId || run.record.resultAssetIds.length === 0)
        return;
      setWorkflowInsertingRunId(run.request.id);
      setNotice('正在解析工作流的真实结果素材…');
      try {
        const resolved = await Promise.all(
          run.record.resultAssetIds.map((assetId) =>
            creativeAssetClient.get(assetId)
          )
        );
        const known = new Map(knownAssetsRef.current);
        for (const asset of resolved) known.set(asset.id, asset);
        knownAssetsRef.current = known;
        handleInsertAssets(resolved);
        void assets.reload();
      } catch (error) {
        setNotice(error instanceof Error ? error.message : String(error));
      } finally {
        setWorkflowInsertingRunId(null);
      }
    },
    [assets, handleInsertAssets, workflowInsertingRunId]
  );

  const handleInsertPrompt = useCallback(
    (selection: PromptLibrarySelection) => {
      const insertion = prepareCenteredInsertion();
      if (!insertion) return;
      const { editor, state, viewportSize } = insertion;
      editor.dispatch(
        canvasCommands.addNode(
          creativeTextNodeFromPrompt(
            selection,
            state,
            viewportSize
          )
        )
      );
      setSelectedPromptId(selection.id);
      setNotice(`已将“${selection.title}”插入为文本节点。`);
    },
    [prepareCenteredInsertion]
  );

  const selection = useMemo(
    () => creativeCanvasProductSelectionCapabilities(canvasState),
    [canvasState]
  );
  const productDisabled = save.revision === null;
  const imageMaskRuntimeBlocksNew =
    imageMaskRuntime.submittingCount > 0 ||
    imageMaskRuntime.recoveringCount > 0 ||
    imageMaskRuntime.submissionFailures.length > 0 ||
    imageMaskRuntime.requestError !== null ||
    imageMaskRuntime.entries.some(
      (entry) =>
        entry.task.status === 'queued' || entry.task.status === 'running'
    );
  const projectTitle =
    project.detail?.project.title ??
    (project.isLoading ? '正在载入项目…' : '画布项目');
  const saveMessage = save.error?.message ?? undefined;
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
      kind="generic"
      title="正在载入画布结构"
      description="等待项目文档通过 canonical v1 校验。"
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
      kind="generic"
      title="正在载入属性"
      description="选择真实节点后才能查看 canonical 属性。"
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
      kind="generic"
      title="正在载入撤销状态"
      description="历史面板仅展示当前编辑会话的真实撤销栈。"
    />
  );

  const timeline = renderCanvasState ? (
    <CreativeCanvasTimelinePanel
      state={renderCanvasState}
      disabled={productDisabled}
      onSelectNode={(nodeId) => dispatch(canvasCommands.setSelection([nodeId]))}
      onAddDirector={() => addNode('director')}
      onOpenDirector={(nodeId) => void handleOpenDirector(nodeId)}
    />
  ) : (
    <CreativeCanvasUnavailablePanel
      kind="generic"
      title="正在载入导演时间线"
      description="等待项目文档通过 canonical v1 校验。"
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
        backgroundMenuOpen={backgroundMenuOpen}
        compact={compact}
        disabled={productDisabled}
        onBackToProjects={() => void handleBackToProjects()}
        onToolChange={setTool}
        onAddNode={addNode}
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
                isMiniMapOpen={miniMapOpen}
                onToggleMiniMap={() => setMiniMapOpen((open) => !open)}
                onStateChange={setCanvasState}
                onSaveStateChange={setSave}
                onAgentSessionsChange={handleAgentSessionsChange}
                onPendingTaskCommandBlocked={() =>
                  setNotice(
                    '运行中的生成任务必须保留配置节点；请等待任务结束后再删除或撤销。'
                  )
                }
                onIntegrationIntent={(intent) =>
                  void handleIntegrationIntent(intent)
                }
                renderNode={({
                  node,
                  selected,
                  onActivate,
                  onOpen,
                  onToggleLock,
                  dragHandleProps,
                }) => {
                  const nodeView = (
                    <CreativeNodeView
                      node={node}
                      selected={selected}
                      placement="contained"
                      asset={
                        resolveCreativeNodeAssetPresentation(
                          node,
                          knownAssetsById
                        ) ?? undefined
                      }
                      onActivate={onActivate}
                      onOpen={onOpen}
                      onToggleLock={onToggleLock}
                      onPointerDown={dragHandleProps.onPointerDown}
                    />
                  );
                  if (node.type !== 'image') return nodeView;
                  return (
                    <CreativeCanvasImageToolbar
                      visible={selected && Boolean(node.data.assetId)}
                      disabled={
                        productDisabled ||
                        assetImportBusy ||
                        imageCropBusy ||
                        imageSplitBusy ||
                        imageMaskBusy ||
                        imageMaskRuntimeBlocksNew
                      }
                      onCrop={() => void handleOpenImageCrop(node)}
                      onDownload={() => void handleDownloadImage(node)}
                      onMaskEdit={() => void handleOpenImageMaskEdit(node)}
                      onSplit={() => void handleOpenImageSplit(node)}
                    >
                      {nodeView}
                    </CreativeCanvasImageToolbar>
                  );
                }}
                renderEdge={(context) => (
                  <CreativeCanvasConnectionEdge {...context} />
                )}
                screenOverlay={
                  <CreativeCanvasInteractionOverlays
                    viewportSize={viewportSize}
                    contextMenu={contextMenu}
                    createNodeMenu={
                      createNodeMenu
                        ? { clientPosition: createNodeMenu.clientPosition }
                        : null
                    }
                    disabled={productDisabled || assetImportBusy}
                    onContextAction={(action) =>
                      void handleContextAction(action)
                    }
                    onOpenCreateNodeMenu={handleOpenCreateNodeMenuFromContext}
                    onPasteFromSystemClipboard={() =>
                      void handlePasteFromContext()
                    }
                    onSelectNode={handleSelectCreatedNode}
                    onDismiss={dismissInteractionOverlays}
                  />
                }
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
            <>
              <ImageMaskRuntimeAction
                snapshot={imageMaskRuntime}
                busy={imageMaskRuntimeActionBusy}
                onCancel={(taskId) => void cancelImageMaskRuntimeTask(taskId)}
                onRetry={(taskId) => void retryImageMaskRuntimeTask(taskId)}
              />
              <SaveRecoveryAction
                save={save}
                busy={recoveryBusy}
                notice={notice}
                onReload={() => void handleReloadRemote()}
                onRetry={() => void handleRetrySave()}
              />
            </>
          ),
          toolbarTrailing: (
            <>
              <ProductToolbarButton
                label="将所选节点分组"
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
                label="取消所选分组"
                icon={<Ungroup {...iconProps} />}
                disabled={productDisabled || selection.groupIds.length === 0}
                onClick={() => {
                  for (const groupId of selection.groupIds) {
                    dispatch(canvasCommands.ungroup(groupId));
                  }
                }}
              />
              <ProductToolbarButton
                label="删除所选节点或连接"
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
            workflows: (
              <CreativeCanvasWorkflowPanel
                workflows={workflows}
                runtime={workflowRuntime.snapshot}
                loading={workflowLoading}
                error={workflowError}
                disabled={productDisabled}
                insertingRunId={workflowInsertingRunId}
                onRetry={() => {
                  void loadWorkflows();
                  void workflowRuntime.controller.load().catch(() => undefined);
                }}
                onRun={setWorkflowToRun}
                onInsertResults={(run) => void handleInsertWorkflowResults(run)}
                onOpenCenter={() => void handleOpenWorkflowCenter()}
              />
            ),
          },
          right: {
            assistant: (
              <CreativeCanvasAgentPanel
                ref={agentPanelRef}
                projectId={projectId}
                hydrated={agentDocumentState !== null}
                sessions={agentDocumentState?.sessions ?? []}
                activeSessionId={agentDocumentState?.activeSessionId ?? null}
                disabled={productDisabled}
                onPersist={handlePersistAgentSessions}
                onCollapse={() => handleRightViewChange(null)}
                onOpenModelSettings={() => void handleOpenModelSettings()}
              />
            ),
            properties,
          },
          bottom: {
            history,
            timeline,
          },
        }}
      />
      {imageMaskRuntimeReady && project.detail ? (
        <CanvasImageMaskEditRuntimeBridge
          key={`${projectId}:${imageMaskRuntimeEpoch}`}
          ref={imageMaskRuntimeRef}
          projectId={projectId}
          initialDocument={project.detail.document}
          editorRef={editorRef}
          viewportSize={viewportSize}
          onAsset={(asset) => {
            knownAssetsRef.current = new Map(knownAssetsRef.current).set(
              asset.id,
              asset
            );
            void assets.reload();
          }}
          onSnapshot={setImageMaskRuntime}
          onNotice={setNotice}
        />
      ) : null}
      <WorkflowRunModal
        workflow={workflowToRun}
        runner={workflowRunner}
        onClose={() => setWorkflowToRun(null)}
        onPickAssets={(variable, selectedAssetIds) =>
          workflowAssetPicker.pick({
            acceptedKinds: ['image'],
            initialSelectedIds: selectedAssetIds,
            selectionLimit:
              variable.type === 'image-series' ? variable.maxItems : 1,
            title:
              variable.type === 'image-series'
                ? '选择变量图片'
                : '选择变量参考图',
          })
        }
        onPickReferenceAssets={(selectedAssetIds) =>
          workflowAssetPicker.pick({
            acceptedKinds: ['image'],
            initialSelectedIds: selectedAssetIds,
            selectionLimit: 100,
            title: '选择工作流参考图',
          })
        }
        onUploadReferenceImages={async (files, selectedAssetIds) => {
          const uploaded = await Promise.all(
            files.map((file) =>
              creativeAssetClient.upload(file, {
                title: file.name,
                tags: ['workflow-reference'],
                inLibrary: true,
              })
            )
          );
          return [
            ...new Set([
              ...selectedAssetIds,
              ...uploaded.map((asset) => asset.id),
            ]),
          ];
        }}
      />
      {workflowAssetPicker.dialog}
      <CreativeImageCropDialog
        visible={pendingImageCrop !== null}
        asset={pendingImageCrop?.asset ?? null}
        busy={imageCropBusy}
        progress={imageCropProgress}
        error={imageCropError}
        onClose={closeImageCrop}
        onConfirm={(crop) => void handleConfirmImageCrop(crop)}
      />
      <CreativeImageSplitDialog
        visible={pendingImageSplit !== null}
        asset={pendingImageSplit?.asset ?? null}
        busy={imageSplitBusy}
        progress={imageSplitProgress}
        error={imageSplitError}
        onClose={closeImageSplit}
        onConfirm={(params) => void handleConfirmImageSplit(params)}
      />
      <CreativeImageMaskEditDialog
        visible={pendingImageMaskEdit !== null}
        asset={pendingImageMaskEdit?.asset ?? null}
        catalog={modelCatalog}
        model={imageMaskModel}
        busy={imageMaskBusy}
        retryLocked={Boolean(pendingImageMaskEdit?.submission)}
        progress={imageMaskProgress}
        error={imageMaskError}
        onModelChange={setImageMaskModel}
        onOpenModelSettings={() => void handleOpenModelSettings()}
        onAbandon={() => void abandonImageMaskSubmission()}
        onClose={closeImageMaskEdit}
        onConfirm={(input) => void handleConfirmImageMaskEdit(input)}
      />
      <Modal
        title="选择 2:1 图片的节点类型"
        visible={pendingPanoramaChoice !== null}
        closable={false}
        maskClosable={false}
        escToExit={false}
        footer={
          <div className={styles.panoramaActions}>
            <Button onClick={() => resolvePendingPanoramaChoice(false)}>
              作为普通图片
            </Button>
            <Button
              type="primary"
              onClick={() => resolvePendingPanoramaChoice(true)}
            >
              作为全景图
            </Button>
          </div>
        }
      >
        <p className={styles.panoramaDescription}>
          图片已经真实上传并保存在素材库中。检测到宽高比接近
          2:1，请确认它应作为普通图片还是等距柱状全景图插入当前画布。
        </p>
      </Modal>
    </main>
  );
};

export default CreativeCanvasProductRoute;
