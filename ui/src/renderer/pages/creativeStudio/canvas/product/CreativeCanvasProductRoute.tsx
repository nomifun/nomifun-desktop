/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { Delete, Group, Loading, Refresh, Ungroup } from '@icon-park/react';
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
  useCreativeAssets,
} from '../../assets';
import { manualUploadRejectionMessage } from '../../assets/page/model';
import {
  CREATIVE_STUDIO_PROJECTS_PATH,
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
import type { PromptLibrarySelection } from '../../prompts';
import { useCreativeProject } from '../../services';
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
import {
  CanvasMiniMap,
  type CanvasMiniMapNavigationRequest,
} from '../graph';
import {
  resolveCanvasContextAction,
  type CanvasContextAction,
  type CanvasContextTarget,
  type CanvasIntegrationIntent,
} from '../interactions';
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
  CREATIVE_CANVAS_PRODUCT_NODE_SIZES,
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
    width: rect && Number.isFinite(rect.width) && rect.width > 0 ? rect.width : 1,
    height: rect && Number.isFinite(rect.height) && rect.height > 0 ? rect.height : 1,
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
  const agentPanelRef = useRef<CreativeCanvasAgentPanelHandle>(null);
  const canvasHostRef = useRef<HTMLDivElement>(null);
  const panelsRef = useRef<CreativeStudioPanelState>(
    structuredClone(DEFAULT_CREATIVE_STUDIO_PANELS)
  );
  const hydratedPanelsRef = useRef<{ projectId: string; revision: string } | null>(null);
  const hydratedBackgroundRef = useRef<{ projectId: string; revision: string } | null>(null);
  const knownAssetsRef = useRef<ReadonlyMap<string, CreativeAsset>>(new Map());
  const assetImportBusyRef = useRef(false);

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
  const [contextMenu, setContextMenu] = useState<CreativeCanvasContextMenuState | null>(null);
  const [createNodeMenu, setCreateNodeMenu] = useState<ProductCreateNodeMenuState | null>(null);
  const [pendingPanoramaChoice, setPendingPanoramaChoice] = useState<PendingPanoramaChoice | null>(null);
  const [assetImportBusy, setAssetImportBusy] = useState(false);
  const [agentDocumentState, setAgentDocumentState] = useState<AgentDocumentState | null>(null);

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
    setContextMenu(null);
    setCreateNodeMenu(null);
    setPendingPanoramaChoice(null);
    setAssetImportBusy(false);
    setAgentDocumentState(null);
    assetImportBusyRef.current = false;
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
    if (!(await agentPanelRef.current?.prepareToLeave() ?? true)) return false;
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

  const dismissInteractionOverlays = useCallback(() => {
    setContextMenu(null);
    setCreateNodeMenu(null);
  }, []);

  const openCreateNodeMenu = useCallback(
    (worldPosition: CanvasPoint, connection: ConnectionCreateNodeIntent | null = null) => {
      const editor = editorRef.current;
      if (!editor) return;
      setContextMenu(null);
      setCreateNodeMenu({
        worldPosition: { ...worldPosition },
        clientPosition: canvasToClient(worldPosition, editor.getState().viewport),
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
      knownAssetsRef.current = new Map(knownAssetsRef.current).set(asset.id, asset);
      editor.dispatch(canvasCommands.addNode(node));
      setNotice(`已将“${asset.title}”插入为${asPanorama ? '全景图' : '素材'}节点。`);
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
          (progress) => setNotice(`正在上传“${file.name}” ${Math.round(progress)}%`)
        );
        if (panoramaChoice === 'after-upload-if-2-to-1' && isTwoToOneImage(asset)) {
          setPendingPanoramaChoice({ asset, worldPosition: { ...worldPosition } });
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

  const insertClipboardText = useCallback((text: string, worldPosition: CanvasPoint) => {
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
  }, []);

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

  const handleIntegrationIntent = useCallback(
    async (intent: CanvasIntegrationIntent) => {
      switch (intent.type) {
        case 'transient-ui/dismiss':
          dismissInteractionOverlays();
          return;
        case 'context-menu/open': {
          const nodeId = intent.target.kind === 'node' ? intent.target.nodeId : null;
          const node =
            nodeId
              ? editorRef.current?.getState().document.nodes.find(
                  (candidate) => candidate.id === nodeId
                )
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
            if (await flushBeforeLeave()) {
              navigate(creativeStudioDirectorProjectPath(projectId));
            }
            return;
          }
          persistPanels(withCreativeCanvasRightView(panelsRef.current, 'properties'));
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
          await importCanvasFile(intent.file, intent.worldPosition, intent.panoramaChoice);
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
      flushBeforeLeave,
      importCanvasFile,
      navigate,
      openCreateNodeMenu,
      persistPanels,
      projectId,
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
      const resolution = resolveCanvasContextAction(editor.getState(), menu.target, action);
      dismissInteractionOverlays();
      for (const command of resolution.commands) editor.dispatch(command);
      for (const intent of resolution.intents) await handleIntegrationIntent(intent);
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
      const node = createCreativeCanvasProductNode(
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
          nodes: [...state.document.nodes, node],
        };
        const validation = validateCanvasConnection(candidateDocument, {
          sourceNodeId,
          targetNodeId,
        });
        if (!validation.ok) {
          setNotice(`无法创建连接：${connectionErrorMessage(validation.code)}。`);
          return;
        }

        const at = Date.now();
        const mergeKey = `create-connected:${node.id}`;
        editor.dispatch(canvasCommands.addNode(node, { at, mergeKey }));
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
        setNotice('已创建节点并完成连接。');
      } else {
        editor.dispatch(canvasCommands.addNode(node));
        setNotice('已在指定位置创建节点。');
      }
      dismissInteractionOverlays();
    },
    [createNodeMenu, dismissInteractionOverlays, save.revision]
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
                isMiniMapOpen={miniMapOpen}
                onToggleMiniMap={() => setMiniMapOpen((open) => !open)}
                onStateChange={setCanvasState}
                onSaveStateChange={setSave}
                onAgentSessionsChange={handleAgentSessionsChange}
                onIntegrationIntent={(intent) => void handleIntegrationIntent(intent)}
                renderNode={({
                  node,
                  selected,
                  onActivate,
                  onOpen,
                  onToggleLock,
                  dragHandleProps,
                }) => (
                  <CreativeNodeView
                    node={node}
                    selected={selected}
                    placement='contained'
                    asset={resolveCreativeNodeAssetPresentation(node, knownAssetsById) ?? undefined}
                    onActivate={onActivate}
                    onOpen={onOpen}
                    onToggleLock={onToggleLock}
                    onPointerDown={dragHandleProps.onPointerDown}
                  />
                )}
                renderEdge={(context) => <CreativeCanvasConnectionEdge {...context} />}
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
                    onContextAction={(action) => void handleContextAction(action)}
                    onOpenCreateNodeMenu={handleOpenCreateNodeMenuFromContext}
                    onPasteFromSystemClipboard={() => void handlePasteFromContext()}
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
            timeline: <CreativeCanvasTimelineUnwiredPanel />,
          },
        }}
      />
      <Modal
        title='选择 2:1 图片的节点类型'
        visible={pendingPanoramaChoice !== null}
        closable={false}
        maskClosable={false}
        escToExit={false}
        footer={
          <div className={styles.panoramaActions}>
            <Button onClick={() => resolvePendingPanoramaChoice(false)}>
              作为普通图片
            </Button>
            <Button type='primary' onClick={() => resolvePendingPanoramaChoice(true)}>
              作为全景图
            </Button>
          </div>
        }
      >
        <p className={styles.panoramaDescription}>
          图片已经真实上传并保存在素材库中。检测到宽高比接近 2:1，请确认它应作为普通图片还是等距柱状全景图插入当前画布。
        </p>
      </Modal>
    </main>
  );
};

export default CreativeCanvasProductRoute;
