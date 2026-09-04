/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { uuidv7 } from '@/common/utils/uuidv7';
import { copyText } from '@/renderer/utils/ui/clipboard';
import {
  CloseOne,
  Delete,
  Group,
  Loading,
  Refresh,
  Ungroup,
} from '@icon-park/react';
import { Button, Modal, Tooltip } from '@arco-design/web-react';
import type { TFunction } from 'i18next';
import React, {
  useCallback,
  useEffect,
  useLayoutEffect,
  useMemo,
  useRef,
  useState,
} from 'react';
import { flushSync } from 'react-dom';
import { useTranslation } from 'react-i18next';
import { useNavigate, useParams } from 'react-router-dom';

import {
  creativeAssetClient,
  isCreativeAssetDeleted,
  subscribeCreativeAssetDeletion,
  type CreativeAsset,
  type CreativeAssetKind,
  useCreativeAssetPickerDialog,
  useCreativeAssets,
} from '../../assets';
import {
  creativeAssetDownloadName,
  type CreativeAssetUploadRejection,
} from '../../assets/page/model';
import {
  CREATIVE_STUDIO_PROJECTS_PATH,
  CREATIVE_STUDIO_TEMPLATES_PATH,
  creativeStudioDirectorProjectPath,
} from '../../app/routes';
import {
  DEFAULT_CREATIVE_STUDIO_PANELS,
  isCreativeCanvasUserNode,
  type CreativeCanvasNode,
  type CreativeCanvasUserNodeKind,
  type CreativeChatSessionReference,
  type CreativeImagePromptMention,
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
  effectiveImageReferenceInputLimit,
  imageWorkbenchSizePolicyForModel,
  imageWorkbenchSelectableSizeOptions,
  imageReferenceInputPolicy,
  normalizeImageWorkbenchSettingsSize,
  type ImageWorkbenchAspectRatioOption,
  type ImageWorkbenchModelIdentity,
  type ImageWorkbenchSettings,
} from '../../workbenches/image';
import {
  exactWorkbenchModelOptions,
  imageWorkbenchModelOptions,
  type CreativeWorkbenchRuntimeSnapshot,
  type CreativeWorkbenchReferences,
  type PreparedCreativeWorkbenchRun,
} from '../../workbenches/runtime';
import type {
  CreativeTemplateDefinitionV1,
  CreativeTemplateRunAggregateV1,
} from '../../templates/domain';
import {
  TemplateRunModal,
  type CreativeTemplateRunnerPort,
} from '../../templates/page';
import { useCreativeTemplateRuntime } from '../../templates/runtime';
import { creativeTemplateRepository } from '../../templates/services';
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
  type CanvasCasSaveSnapshot,
  type CreativeCanvasEditorHandle,
} from '../editor';
import { CanvasMiniMap, type CanvasMiniMapNavigationRequest } from '../graph';
import {
  finishCanvasConnectionDrag,
  resolveCanvasContextAction,
  type CanvasContextAction,
  type CanvasIntegrationIntent,
} from '../interactions';
import {
  CreativeCanvasImageToolbar,
  CreativeImagePreviewDialog,
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
import { buildCreativeCanvasAgentContext } from './agent/context';
import type { CreativeCanvasAgentOp } from './agent/artifacts';
import { creativeCanvasAgentOpsPort } from './agent/opsPort';
import CreativeCanvasConnectionEdge from './CreativeCanvasConnectionEdge';
import CreativeCanvasAudioComposer from './CreativeCanvasAudioComposer';
import CreativeCanvasImageComposer, {
  type CreativeCanvasImageComposerReference,
} from './CreativeCanvasImageComposer';
import CreativeCanvasVideoComposer from './CreativeCanvasVideoComposer';
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
import CreativeCanvasTemplatePanel from './CreativeCanvasTemplatePanel';
import {
  CreativeCanvasProductAssetLibrary,
  CreativeCanvasProductPromptLibrary,
  type CreativeCanvasAssetKindFilter,
} from './CreativeCanvasProductLibraries';
import {
  canvasImageComposeDraftFromState,
  canvasImageComposeTaskSummary,
  DEFAULT_CANVAS_IMAGE_COMPOSE_SETTINGS,
  latestCanvasImageComposeConfig,
  prepareCanvasImageCompose,
  withCanvasImageComposeDraft,
  type CanvasImageComposeDraft,
} from './canvasImageComposerCanvas';
import {
  canvasImageReferenceAssetIds,
  compileCanvasImageReferencePrompt,
  evaluateCanvasImageGenerationGate,
  resolveCanvasImageReferences,
  type CanvasImageGenerationBlocker,
  type CanvasImageReference,
  type CanvasImageReferenceResolution,
  type CanvasTextReference,
} from './canvasImageReferences';
import CanvasImageTaskRuntimeBridge, {
  canvasImageTaskReferenceFromPlan,
  type CanvasImageTaskRuntimeBridgeHandle,
} from './CanvasImageTaskRuntimeBridge';
import CanvasVideoTaskRuntimeBridge, {
  canvasVideoTaskReferenceFromPlan,
  type CanvasVideoTaskRuntimeBridgeHandle,
} from './CanvasVideoTaskRuntimeBridge';
import CanvasAudioTaskRuntimeBridge, {
  canvasAudioTaskReferenceFromPlan,
  type CanvasAudioTaskRuntimeBridgeHandle,
} from './CanvasAudioTaskRuntimeBridge';
import {
  canvasAudioComposeDraftFromState,
  canvasAudioComposeEligibility,
  canvasAudioComposeProtocolProfile,
  canvasAudioComposeTaskSummary,
  canvasAudioComposeVoiceAfterModelChange,
  DEFAULT_CANVAS_AUDIO_COMPOSE_DRAFT,
  latestCanvasAudioComposeConfig,
  prepareCanvasAudioCompose,
  withCanvasAudioComposeDraft,
  type CanvasAudioComposeDraft,
  type CanvasAudioComposeSettings,
} from './canvasAudioComposerCanvas';
import { orphanCanvasAudioComposeTask } from './canvasAudioComposerRuntime';
import {
  canvasVideoComposeDraftFromState,
  canvasVideoComposeMode,
  canvasVideoComposeTaskSummary,
  DEFAULT_CANVAS_VIDEO_COMPOSE_DRAFT,
  latestCanvasVideoComposeConfig,
  prepareCanvasVideoCompose,
  withCanvasVideoComposeDraft,
  type CanvasVideoComposeDraft,
  type CanvasVideoComposeMode,
  type CanvasVideoComposeSettings,
} from './canvasVideoComposerCanvas';
import { orphanCanvasVideoComposeTask } from './canvasVideoComposerRuntime';
import {
  createCreativeCanvasProductNode,
  CREATIVE_CANVAS_PRODUCT_NODE_SIZES,
  creativeCanvasProductInsertionViewport,
  creativeNodeFromAsset,
} from './nodeFactory';
import {
  canLeaveCreativeCanvasAfterFlush,
  creativeCanvasBlockedLeaveMessage,
  creativeCanvasProductPanelViews,
  creativeCanvasProductSelectionCapabilities,
  creativeCanvasSaveDisplayMessage,
  resolveCreativeNodeAssetPresentation,
  withCreativeCanvasLeftPanelOpen,
  withCreativeCanvasBottomView,
  withCreativeCanvasLeftView,
  withCreativeCanvasRightPanelWidth,
  withCreativeCanvasRightView,
} from './productController';
import {
  preferredCanvasImageMaskEditModel,
  prepareCanvasImageMaskEdit,
} from './imageMaskEditCanvas';
import { orphanCanvasImageMaskEditTask } from './imageMaskEditRuntime';
import {
  fillEmptyCanvasImageNodeFromAsset,
  uploadCanvasImageNodeAsset,
} from './imageNodeUpload';
import { registerCreativeCanvasProductBeforeLeave } from './beforeLeave';
import styles from './CreativeCanvasProductRoute.module.css';

const INITIAL_SAVE: CanvasCasSaveSnapshot = {
  status: 'idle',
  revision: null,
  hasPendingChanges: false,
  error: null,
};

const INITIAL_CANVAS_TASK_RUNTIME: CreativeWorkbenchRuntimeSnapshot = {
  state: 'idle',
  entries: [],
  submissionFailures: [],
  submittingCount: 0,
  recoveringCount: 0,
  requestError: null,
};

const FALLBACK_VIEWPORT_SIZE: CreativeSize = { width: 1, height: 1 };

export function shouldPublishCanvasStateToProductRoute(
  currentState: CanvasState | null,
  nextState: CanvasState
): boolean {
  return (
    currentState === null ||
    currentState.document !== nextState.document ||
    currentState.selection !== nextState.selection ||
    currentState.clipboard !== nextState.clipboard ||
    currentState.history !== nextState.history
  );
}

export function selectCreativeCanvasAgentContextInputs(
  state: CanvasState | null,
  enabled: boolean
): readonly [
  document: CanvasState['document'] | null,
  selectedNodeIds: readonly string[] | null,
] {
  return enabled && state
    ? [state.document, state.selection.nodeIds]
    : [null, null];
}

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

interface PendingCanvasImageComposeSubmission {
  nodeId: string;
  plan: PreparedCreativeWorkbenchRun;
  failureOrder: number;
}

interface CanvasImageComposeIssue {
  nodeId: string;
  message: string;
}

interface PendingCanvasVideoComposeSubmission {
  nodeId: string;
  plan: PreparedCreativeWorkbenchRun;
  failureOrder: number;
}

interface CanvasVideoComposeIssue {
  nodeId: string;
  message: string;
}

interface PendingCanvasAudioComposeSubmission {
  nodeId: string;
  plan: PreparedCreativeWorkbenchRun;
  failureOrder: number;
}

interface CanvasAudioComposeIssue {
  nodeId: string;
  message: string;
}

interface AgentDocumentState {
  sessions: readonly CreativeChatSessionReference[];
  activeSessionId: string | null;
}

const iconProps = {
  theme: 'outline' as const,
  size: 17,
  fill: 'currentColor',
  strokeWidth: 3,
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

const normalizeCanvasImageReferenceLabel = (
  value: string,
  ordinal: number
): string => {
  const normalized = value
    .replaceAll('@', '')
    .replace(/[\r\n]+/gu, ' ')
    .replace(/\s+/gu, ' ')
    .trim()
    .slice(0, 64);
  return normalized || `图片${ordinal}`;
};

const canvasImageComposerReferences = (
  references: readonly CanvasImageReference[]
): CreativeCanvasImageComposerReference[] => {
  const used = new Set<string>();
  return references.map((reference) => {
    const preferred = normalizeCanvasImageReferenceLabel(
      reference.displayName,
      reference.ordinal
    );
    let label = preferred;
    let suffix = 2;
    while (used.has(label)) {
      label = `${preferred} ${suffix}`;
      suffix += 1;
    }
    used.add(label);
    return {
      nodeId: reference.sourceNodeId,
      assetId: reference.assetId,
      connectionId: reference.connection?.id ?? null,
      base: reference.connection === null,
      label,
      thumbnailUrl: reference.asset.thumbnailUrl ?? reference.asset.originalUrl,
      originalUrl: reference.asset.originalUrl,
      ordinal: reference.ordinal,
    };
  });
};

const invalidCanvasImageComposerReferences = (
  state: CanvasState,
  targetNodeId: string,
  resolution: CanvasImageReferenceResolution,
  assetsById: ReadonlyMap<string, CreativeAsset>,
  t: TFunction
): CreativeCanvasImageComposerReference[] => {
  const validConnectionIds = new Set(
    resolution.references.flatMap((reference) =>
      reference.connection ? [reference.connection.id] : []
    )
  );
  const nodesById = new Map(state.document.nodes.map((node) => [node.id, node]));
  const issueByConnectionId = new Map(
    resolution.issues.flatMap((issue) =>
      'connectionId' in issue ? [[issue.connectionId, issue] as const] : []
    )
  );
  const items: CreativeCanvasImageComposerReference[] = [];
  for (const [index, connection] of state.document.connections
    .filter((edge) => edge.targetNodeId === targetNodeId)
    .entries()) {
    if (validConnectionIds.has(connection.id)) continue;
    const source = nodesById.get(connection.sourceNodeId);
    const issue = issueByConnectionId.get(connection.id);
    if (!issue || source?.type === 'text') continue;
    const assetId =
      source && (source.type === 'image' || source.type === 'panorama')
        ? source.data.assetId
        : 'assetId' in issue
          ? issue.assetId
          : null;
    const asset = assetId ? assetsById.get(assetId) ?? null : null;
    const label = normalizeCanvasImageReferenceLabel(
      asset?.title ??
        (source?.type === 'image' ? source.data.caption : '') ??
        t('creativeStudio.canvas.image.unavailableReference', {
          defaultValue: '不可用参考',
        }),
      index + 1
    );
    items.push({
      nodeId: source?.id ?? connection.sourceNodeId,
      assetId,
      connectionId: connection.id,
      base: false,
      label,
      thumbnailUrl: asset?.thumbnailUrl ?? asset?.originalUrl ?? null,
      originalUrl: asset?.originalUrl ?? null,
      ordinal: 1_000 + index,
      disabledReason:
        canvasImageGenerationBlockerMessage(
          { code: 'reference_resolution_failed', issue },
          t
        ) ??
        t('creativeStudio.canvas.image.unavailableReference', {
          defaultValue: '不可用参考',
        }),
    });
  }
  const targetIssue = resolution.issues.find(
    (issue) =>
      issue.code === 'target_asset_unresolved' ||
      issue.code === 'target_asset_kind_unsupported'
  );
  if (targetIssue && 'assetId' in targetIssue) {
    const target = nodesById.get(targetNodeId);
    const asset = assetsById.get(targetIssue.assetId) ?? null;
    items.unshift({
      nodeId: targetNodeId,
      assetId: targetIssue.assetId,
      connectionId: null,
      base: true,
      label: normalizeCanvasImageReferenceLabel(
        asset?.title ?? (target?.type === 'image' ? target.data.caption : ''),
        1
      ),
      thumbnailUrl: asset?.thumbnailUrl ?? asset?.originalUrl ?? null,
      originalUrl: asset?.originalUrl ?? null,
      ordinal: 0,
      disabledReason:
        canvasImageGenerationBlockerMessage(
          { code: 'reference_resolution_failed', issue: targetIssue },
          t
        ) ?? undefined,
    });
  }
  return items;
};

const canvasTextComposerReferences = (
  references: readonly CanvasTextReference[],
  t: TFunction
): CreativeCanvasImageComposerReference[] => references.map((reference) => {
  const mentionLabel = t('creativeStudio.canvas.image.textReferenceLabel', { index: reference.ordinal });
  return {
    nodeId: reference.sourceNodeId,
    kind: 'text',
    assetId: null,
    connectionId: reference.connection.id,
    base: false,
    label: reference.text.replace(/\s+/gu, ' ').slice(0, 64) || mentionLabel,
    textContent: reference.text,
    mentionLabel,
    ordinal: reference.ordinal,
    disabledReason: reference.text ? undefined : t('creativeStudio.canvas.image.textReferenceEmpty'),
  };
});

const canvasImageWorkbenchReferences = (
  resolution: CanvasImageReferenceResolution
): CreativeWorkbenchReferences => ({
  assets: resolution.references.map((reference) => reference.asset),
  bindings: resolution.references.map((reference) => ({
    assetId: reference.assetId,
    kind: 'image' as const,
    role: 'reference' as const,
  })),
});

const canvasImageGenerationBlockerMessage = (
  blocker: CanvasImageGenerationBlocker | undefined,
  t: TFunction
): string | null => {
  if (!blocker) return null;
  if (blocker.code === 'reference_limit_exceeded') {
    return t('creativeStudio.canvas.image.referenceLimitExceeded', {
      count: blocker.referenceCount,
      max: blocker.maxInputImages,
      defaultValue: `当前模型最多支持 ${blocker.maxInputImages} 张参考图，已连接 ${blocker.referenceCount} 张。`,
    });
  }
  if (blocker.code === 'reference_limit_unknown') {
    return t('creativeStudio.canvas.image.referenceLimitUnknown', {
      count: blocker.referenceCount,
      defaultValue: `当前模型未声明多图上限，无法安全发送 ${blocker.referenceCount} 张参考图。`,
    });
  }
  if (blocker.code === 'reference_bytes_exceeded') {
    return t('creativeStudio.canvas.image.referenceBytesExceeded', {
      total: Math.ceil(blocker.totalBytes / (1024 * 1024)),
      max: Math.floor(blocker.maxInputBytes / (1024 * 1024)),
      defaultValue: `参考图合计约 ${Math.ceil(blocker.totalBytes / (1024 * 1024))} MB，超过 ${Math.floor(blocker.maxInputBytes / (1024 * 1024))} MB 安全上限。`,
    });
  }
  if (blocker.code === 'prompt_compilation_failed') {
    return blocker.issue.code === 'mention_reference_disconnected'
      ? t('creativeStudio.canvas.image.referenceDisconnected', {
          defaultValue: 'Prompt 中存在已断开的素材引用，请重新连接或删除该引用。',
        })
      : t('creativeStudio.canvas.image.referenceTextChanged', {
          defaultValue: 'Prompt 中的素材引用已被部分修改，请删除后重新使用 @ 选择。',
        });
  }
  switch (blocker.issue.code) {
    case 'source_text_empty':
      return t('creativeStudio.canvas.image.textReferenceEmpty');
    case 'duplicate_asset':
      return t('creativeStudio.canvas.image.duplicateReferenceAsset', {
        defaultValue: '同一图片通过多个节点重复接入，请断开重复连线。',
      });
    case 'source_asset_id_missing':
      return t('creativeStudio.canvas.image.referenceAssetMissing', {
        defaultValue: '已连接的图片节点还没有可用素材。',
      });
    case 'source_asset_unresolved':
    case 'target_asset_unresolved':
      return t('creativeStudio.canvas.image.referenceAssetLoading', {
        defaultValue: '正在载入参考图片，请稍候。',
      });
    case 'source_asset_deleted':
    case 'target_asset_deleted':
      return t('creativeStudio.assets.deleted', { defaultValue: '素材已删除' });
    case 'source_asset_kind_unsupported':
    case 'target_asset_kind_unsupported':
      return t('creativeStudio.canvas.image.referenceKindUnsupported', {
        defaultValue: '已连接素材不是可用图片。',
      });
    case 'source_node_missing':
    case 'target_node_missing':
      return t('creativeStudio.canvas.image.referenceNodeMissing', {
        defaultValue: '参考节点已经不存在。',
      });
    case 'target_node_kind_unsupported':
    case 'source_node_kind_unsupported':
      return t('creativeStudio.canvas.image.referenceKindUnsupported', {
        defaultValue: '该连接不能作为图片参考。',
      });
  }
};

const centeredNodePosition = (
  kind: CreativeCanvasUserNodeKind,
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
  >['code'],
  t: TFunction
): string => {
  switch (code) {
    case 'missing_source':
    case 'missing_target':
      return t('creativeStudio.canvas.connection.errors.missingEndpoint', {
        defaultValue: '连接端点已经不存在',
      });
    case 'self_connection':
      return t('creativeStudio.canvas.connection.errors.selfConnection', {
        defaultValue: '节点不能连接到自身',
      });
    case 'duplicate_connection':
      return t('creativeStudio.canvas.connection.errors.duplicate', {
        defaultValue: '这两个节点已经连接',
      });
    case 'group_connection':
      return t('creativeStudio.canvas.connection.errors.groupUnsupported', {
        defaultValue: '节点组不能参与生成连接',
      });
    case 'config_to_config':
      return t('creativeStudio.canvas.connection.errors.configToConfig', {
        defaultValue: '两个生成任务记录不能直接连接',
      });
    case 'director_output_not_supported':
      return t('creativeStudio.canvas.connection.errors.directorInputOnly', {
        defaultValue: '导演节点只能接收输入',
      });
    case 'director_requires_image_input':
      return t('creativeStudio.canvas.connection.errors.directorImageOnly', {
        defaultValue: '导演节点只接受图片或全景图输入',
      });
    case 'no_valid_drop_target':
      return t('creativeStudio.canvas.connection.errors.invalidDropTarget', {
        defaultValue: '请将连接拖到目标节点卡片上',
      });
  }
};

const manualUploadRejectionMessage = (
  rejection: CreativeAssetUploadRejection,
  t: TFunction
): string => {
  switch (rejection) {
    case 'audio_unsupported':
      return t('creativeStudio.canvas.upload.audioUnsupported', {
        defaultValue:
          '暂不支持手动上传音频；通过音频工作台生成的音频仍会进入素材库。',
      });
    case 'file_too_large':
      return t('creativeStudio.canvas.upload.assetTooLarge', {
        defaultValue: '单个素材不能超过 64 MB。',
      });
    case 'unsupported_media_type':
      return t('creativeStudio.canvas.upload.mediaTypeUnsupported', {
        defaultValue: '手动上传仅支持图片和视频文件。',
      });
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
  requiresAuthoritativeReload: boolean;
  onReload(): void;
  onRetry(): void;
}> = ({ save, busy, notice, requiresAuthoritativeReload, onReload, onRetry }) => {
  const { t } = useTranslation();
  return (
    <>
    {notice ? (
      <span className={styles.notice} role="status" title={notice}>
        {notice}
      </span>
    ) : null}
    {save.status === 'conflict' || requiresAuthoritativeReload ? (
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
        {t('creativeStudio.canvas.save.reloadRemote', {
          defaultValue: '重新载入远端',
        })}
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
        {t('creativeStudio.canvas.save.retry', {
          defaultValue: '重试保存',
        })}
      </button>
    ) : null}
    </>
  );
};

const CanvasTaskRuntimeAction: React.FC<{
  label: string;
  snapshot: CreativeWorkbenchRuntimeSnapshot;
  busy: boolean;
  onCancel(taskId: string): void;
  onRetry(taskId: string): void;
}> = ({ label, snapshot, busy, onCancel, onRetry }) => {
  const { t } = useTranslation();
  const taskLabel = (
    _task: CreativeWorkbenchRuntimeSnapshot['entries'][number]['task']
  ) => label;
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
          {t('creativeStudio.canvas.tasks.syncInterrupted', {
            task: taskLabel(requestError.task),
            defaultValue: '{{task}}同步中断',
          })}
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
          {t('creativeStudio.canvas.tasks.retrySync', {
            defaultValue: '重试任务同步',
          })}
        </button>
      </>
    );
  }
  if (active) {
    return (
      <>
        <span className={styles.notice} role="status">
          {active.task.status === 'queued'
            ? t('creativeStudio.canvas.tasks.queued', {
                task: taskLabel(active.task),
                defaultValue: '{{task}}等待执行',
              })
            : t('creativeStudio.canvas.tasks.running', {
                task: taskLabel(active.task),
                defaultValue: '{{task}}生成中',
              })}
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
          {t('creativeStudio.canvas.tasks.cancel', {
            defaultValue: '取消任务',
          })}
        </button>
      </>
    );
  }
  if (snapshot.recoveringCount > 0) {
    return (
      <span className={styles.notice} role="status">
        {t('creativeStudio.canvas.tasks.recovering', {
          task: label,
          defaultValue: '正在恢复{{task}}…',
        })}
      </span>
    );
  }
  return null;
};

/**
 * Route-level product composition. CreativeCanvasEditor remains the only
 * reducer and CAS owner; this component mirrors only product-consumed state
 * slices to drive chrome and panels, leaving viewport-only updates editor-local.
 */
const CreativeCanvasProductRoute: React.FC = () => {
  const { canvasId: routeCanvasId } = useParams<{ canvasId: string }>();
  const canvasId = routeCanvasId?.trim() ?? '';
  // Legacy local adapter: the migrated product internals still use projectId.
  const projectId = canvasId;
  const navigate = useNavigate();
  const { t, i18n } = useTranslation();
  const locale = i18n.resolvedLanguage ?? i18n.language ?? 'zh-CN';
  const project = useCreativeProject(projectId || null);
  const modelCatalog = useNomiCreativeModelCatalog();
  const templateRuntime = useCreativeTemplateRuntime();
  const templateAssetPicker = useCreativeAssetPickerDialog();

  const editorRef = useRef<CreativeCanvasEditorHandle>(null);
  const imageTaskRuntimeRef = useRef<CanvasImageTaskRuntimeBridgeHandle>(null);
  const videoTaskRuntimeRef = useRef<CanvasVideoTaskRuntimeBridgeHandle>(null);
  const audioTaskRuntimeRef = useRef<CanvasAudioTaskRuntimeBridgeHandle>(null);
  const agentPanelRef = useRef<CreativeCanvasAgentPanelHandle>(null);
  const agentOpsApplyRef = useRef<Promise<void> | null>(null);
  const agentOpsReloadRequiredRef = useRef(false);
  const canvasHostRef = useRef<HTMLDivElement>(null);
  const imageNodeUploadInputRef = useRef<HTMLInputElement>(null);
  const imageNodeUploadTargetRef = useRef<string | null>(null);
  const panelsRef = useRef<CreativeStudioPanelState>(
    structuredClone(DEFAULT_CREATIVE_STUDIO_PANELS)
  );
  const hydratedPanelsRef = useRef<{
    projectId: string;
    revision: string;
  } | null>(null);
  const knownAssetsRef = useRef<ReadonlyMap<string, CreativeAsset>>(new Map());
  const assetImportBusyRef = useRef(false);
  const imageToolBusyRef = useRef(false);
  const imageToolAbortRef = useRef<AbortController | null>(null);
  const activeProjectIdRef = useRef(projectId);
  const templateRequestRef = useRef(0);

  const canvasStateRef = useRef<CanvasState | null>(null);
  const [canvasState, setCanvasState] = useState<CanvasState | null>(null);
  const [editingTextNodeId, setEditingTextNodeId] = useState<string | null>(
    null
  );
  const handleCanvasStateChange = useCallback((nextState: CanvasState) => {
    const currentState = canvasStateRef.current;
    canvasStateRef.current = nextState;
    setEditingTextNodeId((current) => {
      if (!current) return null;
      const node = nextState.document.nodes.find(
        (candidate) => candidate.id === current
      );
      return node?.type === 'text' &&
        !node.locked &&
        nextState.selection.nodeIds.length === 1 &&
        nextState.selection.nodeIds[0] === current
        ? current
        : null;
    });
    if (!shouldPublishCanvasStateToProductRoute(currentState, nextState)) return;
    setCanvasState(nextState);
  }, []);
  const [save, setSave] = useState<CanvasCasSaveSnapshot>(INITIAL_SAVE);
  const [tool, setTool] = useState<CanvasInteractionTool>('select');
  const [viewportSize, setViewportSize] = useState<CreativeSize>(
    FALLBACK_VIEWPORT_SIZE
  );
  const [miniMapOpen, setMiniMapOpen] = useState(false);
  const [miniMapDragging, setMiniMapDragging] = useState(false);
  const [panels, setPanels] = useState<CreativeStudioPanelState>(() =>
    structuredClone(DEFAULT_CREATIVE_STUDIO_PANELS)
  );
  const [recoveryBusy, setRecoveryBusy] = useState(false);
  const [notice, setNotice] = useState<string | null>(null);
  const [assetSearch, setAssetSearch] = useState('');
  const [canvasReferenceAssets, setCanvasReferenceAssets] = useState<
    ReadonlyMap<string, CreativeAsset>
  >(new Map());
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
  const [previewImageNode, setPreviewImageNode] =
    useState<Extract<CreativeCanvasNode, { type: 'image' }> | null>(null);
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
  const [imageTaskRuntime, setImageTaskRuntime] =
    useState<CreativeWorkbenchRuntimeSnapshot>(INITIAL_CANVAS_TASK_RUNTIME);
  const [imageTaskRuntimeReady, setImageTaskRuntimeReady] = useState(false);
  const [imageTaskRuntimeEpoch, setImageTaskRuntimeEpoch] = useState(0);
  const [imageTaskRuntimeActionBusy, setImageTaskRuntimeActionBusy] =
    useState(false);
  const [imageComposeBusy, setImageComposeBusy] = useState(false);
  const [imageComposeIssue, setImageComposeIssue] =
    useState<CanvasImageComposeIssue | null>(null);
  const [imageComposeSubmission, setImageComposeSubmission] =
    useState<PendingCanvasImageComposeSubmission | null>(null);
  const [videoTaskRuntime, setVideoTaskRuntime] =
    useState<CreativeWorkbenchRuntimeSnapshot>(INITIAL_CANVAS_TASK_RUNTIME);
  const [videoTaskRuntimeReady, setVideoTaskRuntimeReady] = useState(false);
  const [videoTaskRuntimeEpoch, setVideoTaskRuntimeEpoch] = useState(0);
  const [videoTaskRuntimeActionBusy, setVideoTaskRuntimeActionBusy] =
    useState(false);
  const [videoComposeBusy, setVideoComposeBusy] = useState(false);
  const [videoComposeIssue, setVideoComposeIssue] =
    useState<CanvasVideoComposeIssue | null>(null);
  const [videoComposeSubmission, setVideoComposeSubmission] =
    useState<PendingCanvasVideoComposeSubmission | null>(null);
  const [audioTaskRuntime, setAudioTaskRuntime] =
    useState<CreativeWorkbenchRuntimeSnapshot>(INITIAL_CANVAS_TASK_RUNTIME);
  const [audioTaskRuntimeReady, setAudioTaskRuntimeReady] = useState(false);
  const [audioTaskRuntimeEpoch, setAudioTaskRuntimeEpoch] = useState(0);
  const [audioTaskRuntimeActionBusy, setAudioTaskRuntimeActionBusy] =
    useState(false);
  const [audioComposeBusy, setAudioComposeBusy] = useState(false);
  const [audioComposeIssue, setAudioComposeIssue] =
    useState<CanvasAudioComposeIssue | null>(null);
  const [audioComposeSubmission, setAudioComposeSubmission] =
    useState<PendingCanvasAudioComposeSubmission | null>(null);
  const [agentDocumentState, setAgentDocumentState] =
    useState<AgentDocumentState | null>(null);
  const [agentOpsApplyBusy, setAgentOpsApplyBusy] = useState(false);
  const [agentOpsReloadRequired, setAgentOpsReloadRequired] = useState(false);
  const setAgentOpsReloadFence = useCallback((required: boolean) => {
    agentOpsReloadRequiredRef.current = required;
    setAgentOpsReloadRequired(required);
  }, []);
  const [templates, setTemplates] = useState<CreativeTemplateDefinitionV1[]>([]);
  const [templateLoading, setTemplateLoading] = useState(false);
  const [templateError, setTemplateError] = useState<string | null>(null);
  const [templateToRun, setTemplateToRun] =
    useState<CreativeTemplateDefinitionV1 | null>(null);
  const [templateInsertingRunId, setTemplateInsertingRunId] = useState<
    string | null
  >(null);
  const agentOpsBlockedByCanvasMutation =
    recoveryBusy ||
    assetImportBusy ||
    imageCropBusy ||
    imageSplitBusy ||
    imageMaskBusy ||
    imageTaskRuntimeActionBusy ||
    imageComposeBusy ||
    videoTaskRuntimeActionBusy ||
    videoComposeBusy ||
    audioTaskRuntimeActionBusy ||
    audioComposeBusy ||
    templateInsertingRunId !== null ||
    [imageTaskRuntime, videoTaskRuntime, audioTaskRuntime].some(
      (runtime) =>
        runtime.submittingCount > 0 ||
        runtime.recoveringCount > 0 ||
        runtime.entries.some(
          (entry) => entry.task.status === 'queued' || entry.task.status === 'running'
        )
    );

  const loadTemplates = useCallback(async () => {
    const request = ++templateRequestRef.current;
    setTemplateLoading(true);
    setTemplateError(null);
    try {
      const loaded = await creativeTemplateRepository.list();
      if (request !== templateRequestRef.current) return;
      setTemplates(
        [...loaded].sort(
          (left, right) => right.metadata.updatedAt - left.metadata.updatedAt
        )
      );
    } catch (error) {
      if (request !== templateRequestRef.current) return;
      setTemplateError(error instanceof Error ? error.message : String(error));
    } finally {
      if (request === templateRequestRef.current) setTemplateLoading(false);
    }
  }, []);

  useEffect(() => {
    void loadTemplates();
    return () => {
      templateRequestRef.current += 1;
    };
  }, [loadTemplates]);

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
  const imageComposeModelOptions = useMemo(
    () => imageWorkbenchModelOptions(modelCatalog, 'image_edit'),
    [modelCatalog]
  );
  const imageGenerationModelOptions = useMemo(
    () => imageWorkbenchModelOptions(modelCatalog, 'image_generation'),
    [modelCatalog]
  );
  const imageGenerationExactOptions = useMemo(
    () => exactWorkbenchModelOptions(modelCatalog, 'image_generation'),
    [modelCatalog]
  );
  const videoModelOptions = useMemo(
    () => exactWorkbenchModelOptions(modelCatalog, 'video_generation'),
    [modelCatalog]
  );
  const audioModelOptions = useMemo(
    () => exactWorkbenchModelOptions(modelCatalog, 'speech_synthesis'),
    [modelCatalog]
  );

  const knownAssetsById = useMemo(() => {
    const merged = new Map(knownAssetsRef.current);
    for (const asset of assets.assets) merged.set(asset.id, asset);
    for (const asset of canvasReferenceAssets.values()) merged.set(asset.id, asset);
    knownAssetsRef.current = merged;
    return merged;
  }, [assets.assets, canvasReferenceAssets]);

  const selectedCanvasImageReferenceAssetIds = useMemo(() => {
    if (!canvasState || canvasState.selection.nodeIds.length !== 1) return [];
    const nodeId = canvasState.selection.nodeIds[0];
    const node = canvasState.document.nodes.find((candidate) => candidate.id === nodeId);
    return node?.type === 'image'
      ? canvasImageReferenceAssetIds(canvasState, node.id)
      : [];
  }, [canvasState]);
  const selectedCanvasImageReferenceAssetKey =
    selectedCanvasImageReferenceAssetIds.join('\u0000');
  const canvasMediaAssetIds = useMemo(() => [...new Set([
    ...selectedCanvasImageReferenceAssetIds,
    ...(canvasState?.document.nodes.flatMap((node) => {
      if (node.type === 'video') {
        return [node.data.assetId, node.data.posterAssetId].filter(
          (assetId): assetId is string => Boolean(assetId)
        );
      }
      return (node.type === 'image' || node.type === 'panorama' || node.type === 'audio')
        && node.data.assetId ? [node.data.assetId] : [];
    }) ?? []),
  ])], [canvasState?.document.nodes, selectedCanvasImageReferenceAssetKey]);
  const canvasMediaAssetKey = canvasMediaAssetIds.join('\u0000');

  useEffect(() => {
    let active = true;
    const resolve = (ids: readonly string[]) => {
      void Promise.allSettled(ids.map((id) => creativeAssetClient.get(id))).then((results) => {
        if (!active) return;
        setCanvasReferenceAssets((current) => {
          const next = new Map(current);
          for (const result of results) {
            if (result.status === 'fulfilled') next.set(result.value.id, result.value);
          }
          return next;
        });
      });
    };
    const unsubscribe = subscribeCreativeAssetDeletion(creativeAssetClient, (assetId) => {
      const known = knownAssetsRef.current.get(assetId);
      if (known) {
        const deleted = { ...known, deletedAt: Date.now(), textContent: null, originalUrl: '', thumbnailUrl: null, inLibrary: false };
        knownAssetsRef.current = new Map(knownAssetsRef.current).set(assetId, deleted);
        setCanvasReferenceAssets((current) => new Map(current).set(assetId, deleted));
      }
      resolve([assetId]);
    });
    const refresh = () => resolve(canvasMediaAssetIds);
    window.addEventListener('focus', refresh);
    return () => { active = false; unsubscribe(); window.removeEventListener('focus', refresh); };
  }, [projectId, canvasMediaAssetKey]);

  useEffect(() => {
    if (!projectId || canvasMediaAssetIds.length === 0) return;
    const missing = canvasMediaAssetIds.filter(
      (assetId) => !knownAssetsById.has(assetId)
    );
    if (missing.length === 0) return;
    let active = true;
    void Promise.allSettled(missing.map((assetId) => creativeAssetClient.get(assetId))).then(
      (results) => {
        if (!active || activeProjectIdRef.current !== projectId) return;
        const resolved = results.flatMap((result) =>
          result.status === 'fulfilled'
            ? [result.value]
            : []
        );
        if (resolved.length === 0) return;
        setCanvasReferenceAssets((current) => {
          const next = new Map(current);
          for (const asset of resolved) next.set(asset.id, asset);
          return next;
        });
      }
    );
    return () => {
      active = false;
    };
  }, [knownAssetsById, projectId, canvasMediaAssetKey]);

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
    imageNodeUploadTargetRef.current = null;
    if (imageNodeUploadInputRef.current) imageNodeUploadInputRef.current.value = '';
    const defaultPanels = structuredClone(DEFAULT_CREATIVE_STUDIO_PANELS);
    panelsRef.current = defaultPanels;
    setPanels(defaultPanels);
    hydratedPanelsRef.current = null;
    canvasStateRef.current = null;
    setCanvasState(null);
    setEditingTextNodeId(null);
    setSave(INITIAL_SAVE);
    agentOpsReloadRequiredRef.current = false;
    setAgentOpsReloadRequired(false);
    setAgentOpsApplyBusy(false);
    setNotice(null);
    setCanvasReferenceAssets(new Map());
    setContextMenu(null);
    setCreateNodeMenu(null);
    setPendingPanoramaChoice(null);
    setAssetImportBusy(false);
    setPreviewImageNode(null);
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
    setImageTaskRuntime(INITIAL_CANVAS_TASK_RUNTIME);
    setImageTaskRuntimeReady(false);
    setImageTaskRuntimeEpoch(0);
    setImageTaskRuntimeActionBusy(false);
    setImageComposeBusy(false);
    setImageComposeIssue(null);
    setImageComposeSubmission(null);
    setVideoTaskRuntime(INITIAL_CANVAS_TASK_RUNTIME);
    setVideoTaskRuntimeReady(false);
    setVideoTaskRuntimeEpoch(0);
    setVideoTaskRuntimeActionBusy(false);
    setVideoComposeBusy(false);
    setVideoComposeIssue(null);
    setVideoComposeSubmission(null);
    setAudioTaskRuntime(INITIAL_CANVAS_TASK_RUNTIME);
    setAudioTaskRuntimeReady(false);
    setAudioTaskRuntimeEpoch(0);
    setAudioTaskRuntimeActionBusy(false);
    setAudioComposeBusy(false);
    setAudioComposeIssue(null);
    setAudioComposeSubmission(null);
    setAgentDocumentState(null);
    assetImportBusyRef.current = false;
    imageToolBusyRef.current = false;
    knownAssetsRef.current = new Map();
    return () => {
      imageToolAbortRef.current?.abort();
    };
  }, [projectId]);

  useEffect(() => {
    if (
      imageTaskRuntimeReady &&
      videoTaskRuntimeReady &&
      audioTaskRuntimeReady
    ) {
      return;
    }
    const detail = project.detail;
    if (!detail || detail.project.projectId !== projectId || !canvasState)
      return;
    const currentNodeIds = new Set(
      canvasState.document.nodes.map((node) => node.id)
    );
    if (detail.document.nodes.every((node) => currentNodeIds.has(node.id))) {
      if (!imageTaskRuntimeReady) setImageTaskRuntimeReady(true);
      if (!videoTaskRuntimeReady) setVideoTaskRuntimeReady(true);
      if (!audioTaskRuntimeReady) setAudioTaskRuntimeReady(true);
    }
  }, [
    audioTaskRuntimeReady,
    canvasState,
    imageTaskRuntimeReady,
    project.detail,
    projectId,
    videoTaskRuntimeReady,
  ]);

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

  const handleLeftPanelOpenChange = useCallback(
    (open: boolean) => {
      persistPanels(withCreativeCanvasLeftPanelOpen(panelsRef.current, open));
    },
    [persistPanels]
  );

  const handleRightViewChange = useCallback(
    (view: CreativeStudioPanelState['right']['activeView'] | null) => {
      persistPanels(withCreativeCanvasRightView(panelsRef.current, view));
    },
    [persistPanels]
  );

  const handleRightPanelWidthChange = useCallback(
    (width: number) => {
      persistPanels(
        withCreativeCanvasRightPanelWidth(panelsRef.current, width)
      );
    },
    [persistPanels]
  );

  const handleBottomViewChange = useCallback(
    (view: CreativeStudioPanelState['bottom']['activeView'] | null) => {
      persistPanels(withCreativeCanvasBottomView(panelsRef.current, view));
    },
    [persistPanels]
  );

  const updateImageComposeDraft = useCallback(
    (
      nodeId: string,
      update: (current: CanvasImageComposeDraft) => CanvasImageComposeDraft
    ) => {
      const editor = editorRef.current;
      if (!editor) return;
      const state = editor.getState();
      const node = state.document.nodes.find(
        (candidate): candidate is Extract<CreativeCanvasNode, { type: 'image' }> =>
          candidate.id === nodeId && candidate.type === 'image'
      );
      if (!node) return;
      const current = canvasImageComposeDraftFromState(state, nodeId);
      const nextState = editor.dispatch(
        canvasCommands.updateNode(withCanvasImageComposeDraft(node, update(current)), {
          mergeKey: `image-composer:${nodeId}`,
        })
      );
      handleCanvasStateChange(nextState);
    },
    [handleCanvasStateChange]
  );

  const updateVideoComposeDraft = useCallback(
    (
      nodeId: string,
      update: (current: CanvasVideoComposeDraft) => CanvasVideoComposeDraft
    ) => {
      const editor = editorRef.current;
      if (!editor) return;
      const state = editor.getState();
      const node = state.document.nodes.find(
        (candidate): candidate is Extract<CreativeCanvasNode, { type: 'video' }> =>
          candidate.id === nodeId && candidate.type === 'video'
      );
      if (!node) return;
      const current = canvasVideoComposeDraftFromState(state, nodeId);
      const nextState = editor.dispatch(
        canvasCommands.updateNode(withCanvasVideoComposeDraft(node, update(current)), {
          mergeKey: `video-composer:${nodeId}`,
        })
      );
      handleCanvasStateChange(nextState);
    },
    [handleCanvasStateChange]
  );

  const updateAudioComposeDraft = useCallback(
    (
      nodeId: string,
      update: (current: CanvasAudioComposeDraft) => CanvasAudioComposeDraft
    ) => {
      const editor = editorRef.current;
      if (!editor) return;
      const state = editor.getState();
      const node = state.document.nodes.find(
        (candidate): candidate is Extract<CreativeCanvasNode, { type: 'audio' }> =>
          candidate.id === nodeId && candidate.type === 'audio'
      );
      if (!node) return;
      const current = canvasAudioComposeDraftFromState(state, nodeId);
      editor.dispatch(
        canvasCommands.updateNode(
          withCanvasAudioComposeDraft(node, update(current)),
          { mergeKey: `audio-composer:${nodeId}` }
        )
      );
      handleCanvasStateChange(editor.getState());
    },
    [handleCanvasStateChange]
  );

  const handleInlineTextChange = useCallback(
    (nodeId: string, text: string) => {
      const editor = editorRef.current;
      if (!editor) return;
      const node = editor
        .getState()
        .document.nodes.find(
          (candidate): candidate is Extract<
            CreativeCanvasNode,
            { type: 'text' }
          > => candidate.id === nodeId && candidate.type === 'text'
        );
      if (!node || node.locked || text.length > 1_000_000) {
        setEditingTextNodeId(null);
        return;
      }
      const nextState = editor.dispatch(
        canvasCommands.updateNode(
          { ...node, data: { ...node.data, text } },
          { mergeKey: `inline-text:${nodeId}` }
        )
      );
      handleCanvasStateChange(nextState);
    },
    [handleCanvasStateChange]
  );

  const finishInlineTextEditing = useCallback((nodeId: string) => {
    setEditingTextNodeId((current) => (current === nodeId ? null : current));
  }, []);

  const openPromptLibrary = useCallback(() => {
    handleLeftViewChange('prompts');
  }, [handleLeftViewChange]);

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
    (kind: CreativeCanvasUserNodeKind) => {
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
              ? t('creativeStudio.canvas.notices.directorSelected', {
                  defaultValue: '画布已有唯一导演节点，已为你选中。',
                })
              : t('creativeStudio.canvas.notices.directorConflict', {
                  defaultValue: '画布存在多个导演节点，请在时间线面板中处理冲突。',
                })
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
        setNotice(
          t('creativeStudio.canvas.notices.directorCreated', {
            defaultValue: '已创建当前画布唯一的导演节点。',
          })
        );
      } else {
        setNotice(null);
      }
    },
    [handleBottomViewChange, prepareCenteredInsertion, save.revision]
  );

  const handleMiniMapNavigate = useCallback(
    (request: CanvasMiniMapNavigationRequest) => {
      setMiniMapDragging(request.phase !== 'end');
      editorRef.current?.dispatch(canvasCommands.setViewport(request.viewport));
    },
    []
  );

  const flushBeforeLeave = useCallback(async (): Promise<boolean> => {
    if (imageToolBusyRef.current) {
      setNotice(
        t('creativeStudio.canvas.notices.imageToolBusyLeave', {
          defaultValue: '图片工具仍在处理，请等待完成后再离开。',
        })
      );
      return false;
    }
    if (agentOpsReloadRequiredRef.current) {
      setNotice(
        t('creativeStudio.canvas.notices.agentReloadRequired', {
          defaultValue: '必须先重新载入 Agent 提案提交后的远端画布。',
        })
      );
      return false;
    }
    const agentOpsApply = agentOpsApplyRef.current;
    if (agentOpsApply) {
      try {
        await agentOpsApply;
      } catch {
        setNotice(
          t('creativeStudio.canvas.notices.agentApplyUnconfirmedLeave', {
            defaultValue: 'Agent 提案的应用结果尚未确认，请复核远端画布后再离开。',
          })
        );
        return false;
      }
    }
    if (agentOpsReloadRequiredRef.current) {
      setNotice(
        t('creativeStudio.canvas.notices.agentReloadRequired', {
          defaultValue: '必须先重新载入 Agent 提案提交后的远端画布。',
        })
      );
      return false;
    }
    if (!((await agentPanelRef.current?.prepareToLeave()) ?? true))
      return false;
    if (agentOpsReloadRequiredRef.current) {
      setNotice(
        t('creativeStudio.canvas.notices.agentReloadRequired', {
          defaultValue: '必须先重新载入 Agent 提案提交后的远端画布。',
        })
      );
      return false;
    }
    const editor = editorRef.current;
    if (!editor) return true;
    const result = await editor.flush();
    const canLeave = canLeaveCreativeCanvasAfterFlush(result);
    if (!canLeave) {
      setNotice(
        creativeCanvasBlockedLeaveMessage(result) ??
          t('creativeStudio.canvas.save.notSafe', {
            defaultValue: '画布尚未安全保存。',
          })
      );
    }
    if (agentOpsReloadRequiredRef.current) {
      setNotice(
        t('creativeStudio.canvas.notices.agentReloadRequired', {
          defaultValue: '必须先重新载入 Agent 提案提交后的远端画布。',
        })
      );
      return false;
    }
    return canLeave;
  }, []);

  const handlePersistAgentSessions = useCallback(
    async (
      sessions: readonly CreativeChatSessionReference[],
      activeSessionId: string | null
    ) => {
      const editor = editorRef.current;
      if (!editor) {
        throw new Error(
          t('creativeStudio.canvas.errors.agentSessionSaveUnavailable', {
            defaultValue: '画布尚未载入，无法保存 Agent 会话。',
          })
        );
      }
      await editor.persistAgentSessions(sessions, activeSessionId);
    },
    []
  );

  const handleApplyCanvasAgentOps = useCallback(
    async (
      assistantMessageId: string,
      ops: readonly CreativeCanvasAgentOp[]
    ) => {
      if (agentOpsApplyRef.current) {
        throw new Error(
          t('creativeStudio.canvas.errors.agentApplyBusy', {
            defaultValue: '已有 Agent 提案正在应用。',
          })
        );
      }
      if (
        agentOpsBlockedByCanvasMutation ||
        assetImportBusyRef.current ||
        imageToolBusyRef.current
      ) {
        throw new Error(
          t('creativeStudio.canvas.errors.agentApplyBlocked', {
            defaultValue:
              '画布仍有创作或恢复任务，请等待完成后再应用 Agent 提案。',
          })
        );
      }
      const editor = editorRef.current;
      if (!editor) {
        throw new Error(
          t('creativeStudio.canvas.errors.agentApplyUnavailable', {
            defaultValue: '画布尚未载入，无法应用 Agent 提案。',
          })
        );
      }
      flushSync(() => setAgentOpsApplyBusy(true));
      setNotice(
        t('creativeStudio.canvas.notices.applyingAgentOps', {
          count: ops.length,
          defaultValue: '正在应用 Agent 提案的 {{count}} 项画布操作…',
        })
      );
      const operation = (async () => {
        const reloadRemoteSafely = async (): Promise<boolean> => {
          try {
            return await editor.reloadRemote();
          } catch {
            return false;
          }
        };
        const flush = await editor.flush();
        if (!canLeaveCreativeCanvasAfterFlush(flush)) {
          setNotice(
            creativeCanvasBlockedLeaveMessage(flush) ??
              t('creativeStudio.canvas.save.notSafe', {
                defaultValue: '画布尚未安全保存。',
              })
          );
          throw new Error(
            creativeCanvasBlockedLeaveMessage(flush) ??
              t('creativeStudio.canvas.save.notSafe', {
                defaultValue: '画布尚未安全保存。',
              })
          );
        }
        const expectedRevision = editor.getSaveState().revision;
        if (expectedRevision === null) {
          throw new Error(
            t('creativeStudio.canvas.errors.missingRemoteRevision', {
              defaultValue: '画布缺少可验证的远端 revision。',
            })
          );
        }
        let replayed = false;
        try {
          const applied = await creativeCanvasAgentOpsPort.apply({
            canvasId: projectId,
            assistantMessageId,
            expectedRevision,
            ops,
          });
          replayed = applied.replayed;
        } catch (error) {
          const reloaded = await reloadRemoteSafely();
          setAgentOpsReloadFence(!reloaded);
          if (reloaded) agentPanelRef.current?.refreshAuthority();
          setNotice(
            reloaded
              ? t('creativeStudio.canvas.notices.agentApplyReloaded', {
                  defaultValue:
                    'Agent 提案的应用结果未确认；已重新载入远端画布，请复核。',
                })
              : t('creativeStudio.canvas.notices.agentApplyReloadFailed', {
                  defaultValue: 'Agent 提案应用失败，远端画布也暂时无法重新载入。',
                })
          );
          throw error;
        }
        if (!(await reloadRemoteSafely())) {
          setAgentOpsReloadFence(true);
          setNotice(
            t('creativeStudio.canvas.notices.agentSubmittedReloadFailed', {
              defaultValue: 'Agent 提案已提交，但远端画布暂时无法重新载入。',
            })
          );
          return;
        }
        setAgentOpsReloadFence(false);
        setNotice(
          replayed
            ? t('creativeStudio.canvas.notices.agentOpsReplayed', {
                count: ops.length,
                defaultValue: '已确认该 Agent 提案此前应用的 {{count}} 项画布操作。',
              })
            : t('creativeStudio.canvas.notices.agentOpsApplied', {
                count: ops.length,
                defaultValue: '已应用 Agent 提案的 {{count}} 项画布操作。',
              })
        );
      })();
      agentOpsApplyRef.current = operation;
      try {
        await operation;
      } finally {
        if (agentOpsApplyRef.current === operation) {
          agentOpsApplyRef.current = null;
        }
        setAgentOpsApplyBusy(false);
      }
    },
    [agentOpsBlockedByCanvasMutation, projectId, setAgentOpsReloadFence]
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

  const handleOpenTemplateCenter = useCallback(async () => {
    if (await flushBeforeLeave()) navigate(CREATIVE_STUDIO_TEMPLATES_PATH);
  }, [flushBeforeLeave, navigate]);

  const templateRunner = useMemo<CreativeTemplateRunnerPort>(
    () => ({
      async start(input) {
        await templateRuntime.controller.start(input);
      },
    }),
    [templateRuntime.controller]
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
      if (!editor) {
        throw new Error(
          t('creativeStudio.canvas.errors.assetInsertUnavailable', {
            defaultValue: '画布尚未载入，无法插入素材。',
          })
        );
      }
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
        t('creativeStudio.canvas.notices.assetInserted', {
          title: asset.title,
          kind: asPanorama
            ? t('creativeStudio.canvas.nodeKinds.panorama', {
                defaultValue: '全景图',
              })
            : t('creativeStudio.canvas.notices.assetKind', {
                defaultValue: '素材',
              }),
          defaultValue: '已将“{{title}}”插入为{{kind}}节点。',
        })
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
        setNotice(
          t('creativeStudio.canvas.notices.uploadBusy', {
            defaultValue: '已有素材正在上传，请等待完成。',
          })
        );
        return;
      }
      assetImportBusyRef.current = true;
      setAssetImportBusy(true);
      setNotice(
        t('creativeStudio.canvas.notices.uploading', {
          name: file.name,
          defaultValue: '正在上传“{{name}}”…',
        })
      );
      try {
        const asset = await creativeAssetClient.upload(
          file,
          { title: file.name, inLibrary: true, tags: ['canvas-import'] },
          undefined,
          (progress) =>
            setNotice(
              t('creativeStudio.canvas.notices.uploadProgress', {
                name: file.name,
                progress: Math.round(progress),
                defaultValue: '正在上传“{{name}}” {{progress}}%',
              })
            )
        );
        if (
          panoramaChoice === 'after-upload-if-2-to-1' &&
          isTwoToOneImage(asset)
        ) {
          setPendingPanoramaChoice({
            asset,
            worldPosition: { ...worldPosition },
          });
          setNotice(
            t('creativeStudio.canvas.notices.panoramaDetected', {
              defaultValue: '检测到真实 2:1 图片，请选择普通图片或全景图节点。',
            })
          );
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

  const openImageNodeUpload = useCallback((nodeId: string) => {
    if (assetImportBusyRef.current) {
      setNotice(
        t('creativeStudio.canvas.notices.uploadBusy', {
          defaultValue: '已有素材正在上传，请等待完成。',
        })
      );
      return;
    }
    imageNodeUploadTargetRef.current = nodeId;
    const input = imageNodeUploadInputRef.current;
    if (!input) {
      setNotice(
        t('creativeStudio.canvas.errors.imagePickerUnavailable', {
          defaultValue: '图片文件选择器暂时不可用。',
        })
      );
      return;
    }
    input.value = '';
    input.click();
  }, []);

  const handleImageNodeUploadChange = useCallback(
    async (event: React.ChangeEvent<HTMLInputElement>) => {
      const input = event.currentTarget;
      const file = input.files?.[0] ?? null;
      const nodeId = imageNodeUploadTargetRef.current;
      imageNodeUploadTargetRef.current = null;
      input.value = '';
      if (!file || !nodeId) return;
      if (!file.type.startsWith('image/')) {
        setNotice(
          t('creativeStudio.canvas.upload.imageOnly', {
            defaultValue: '该节点只接受真实图片文件。',
          })
        );
        return;
      }
      if (assetImportBusyRef.current) {
        setNotice(
          t('creativeStudio.canvas.notices.uploadBusy', {
            defaultValue: '已有素材正在上传，请等待完成。',
          })
        );
        return;
      }

      assetImportBusyRef.current = true;
      setAssetImportBusy(true);
      setNotice(
        t('creativeStudio.canvas.notices.uploading', {
          name: file.name,
          defaultValue: '正在上传“{{name}}”…',
        })
      );
      let uploadedAsset: CreativeAsset | null = null;
      try {
        const uploaded = await uploadCanvasImageNodeAsset({
          port: creativeAssetClient,
          file,
          operationId: uuidv7(),
          onProgress: (progress) =>
            setNotice(
              t('creativeStudio.canvas.notices.uploadProgress', {
                name: file.name,
                progress: Math.round(progress),
                defaultValue: '正在上传“{{name}}” {{progress}}%',
              })
            ),
        });
        const asset = uploaded.asset;
        uploadedAsset = asset;
        if (activeProjectIdRef.current !== projectId) {
          throw new DOMException('Canvas changed', 'AbortError');
        }
        const editor = editorRef.current;
        if (!editor) {
          throw new Error(
            t('creativeStudio.canvas.errors.closedAfterUpload', {
              defaultValue: '画布已经关闭，图片保留在素材库中。',
            })
          );
        }
        const state = editor.getState();
        const source = state.document.nodes.find(
          (node): node is Extract<CreativeCanvasNode, { type: 'image' }> =>
            node.id === nodeId && node.type === 'image'
        );
        if (!source) {
          throw new Error(
            t('creativeStudio.canvas.errors.imageNodeRemovedAfterUpload', {
              defaultValue: '图片节点已被删除，上传结果保留在素材库中。',
            })
          );
        }
        const updated = fillEmptyCanvasImageNodeFromAsset(source, asset);
        knownAssetsRef.current = new Map(knownAssetsRef.current).set(asset.id, asset);
        const updatedState = editor.dispatch(
          canvasCommands.updateNode(updated, {
            at: Date.now(),
            mergeKey: `image-upload:${nodeId}`,
          })
        );
        const linked = updatedState.document.nodes.find(
          (node) => node.id === nodeId && node.type === 'image'
        );
        if (linked?.type !== 'image' || linked.data.assetId !== asset.id) {
          throw new Error(
            t('creativeStudio.canvas.errors.imageNodeTaskProtected', {
              defaultValue:
                '图片节点当前受运行任务保护；上传素材已保留在素材库中。',
            })
          );
        }
        const flush = await editor.flush();
        if (!canLeaveCreativeCanvasAfterFlush(flush)) {
          throw new Error(
            creativeCanvasBlockedLeaveMessage(flush) ??
              t('creativeStudio.canvas.errors.imageNodeNotSaved', {
                defaultValue: '图片节点尚未安全保存。',
              })
          );
        }
        setNotice(
          uploaded.recoveredAfterResponseLoss
            ? t('creativeStudio.canvas.notices.uploadRecoveredIntoNode', {
                title: asset.title,
                defaultValue: '已找回上传结果并将“{{title}}”填入原图片节点。',
              })
            : t('creativeStudio.canvas.notices.uploadFilledNode', {
                title: asset.title,
                defaultValue: '已将“{{title}}”填入原图片节点。',
              })
        );
      } catch (error) {
        if (!(error instanceof DOMException && error.name === 'AbortError')) {
          setNotice(error instanceof Error ? error.message : String(error));
        }
      } finally {
        if (uploadedAsset) void assets.reload();
        assetImportBusyRef.current = false;
        setAssetImportBusy(false);
      }
    },
    [assets, projectId]
  );

  const resolveCanvasImageAsset = useCallback(
    async (node: Extract<CreativeCanvasNode, { type: 'image' }>, allowDeleted = false) => {
      const assetId = node.data.assetId?.trim();
      if (!assetId) {
        throw new Error(
          t('creativeStudio.canvas.errors.imageAssetMissing', {
            defaultValue: '该图片节点尚未关联真实素材。',
          })
        );
      }
      const asset = await creativeAssetClient.get(assetId);
      if (!allowDeleted && isCreativeAssetDeleted(asset)) {
        throw new Error(t('creativeStudio.assets.deleted', { defaultValue: '素材已删除' }));
      }
      if (asset.kind !== 'image') {
        throw new Error(
          t('creativeStudio.canvas.errors.imageAssetKindMismatch', {
            defaultValue: '该节点关联的素材不是图片，已停止图片操作。',
          })
        );
      }
      knownAssetsRef.current = new Map(knownAssetsRef.current).set(
        asset.id,
        asset
      );
      return asset;
    },
    [t]
  );

  const resolveCanvasImagePreviewAsset = useCallback(
    (node: Extract<CreativeCanvasNode, { type: 'image' }>) => resolveCanvasImageAsset(node, true),
    [resolveCanvasImageAsset]
  );

  const handleOpenImageCrop = useCallback(
    async (node: Extract<CreativeCanvasNode, { type: 'image' }>) => {
      if (imageToolBusyRef.current || assetImportBusyRef.current) {
        setNotice(
          t('creativeStudio.canvas.notices.imageOperationBusy', {
            defaultValue: '已有图片或素材操作正在进行，请等待完成。',
          })
        );
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
        setNotice(
          t('creativeStudio.canvas.notices.imageOperationBusy', {
            defaultValue: '已有图片或素材操作正在进行，请等待完成。',
          })
        );
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
        setImageCropError(
          t('creativeStudio.canvas.errors.concurrentUpload', {
            defaultValue: '另一个素材上传仍在进行，请等待完成后重试。',
          })
        );
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
          throw new DOMException('Canvas changed', 'AbortError');
        }

        const current = editor.getState();
        const source = current.document.nodes.find(
          (node): node is Extract<CreativeCanvasNode, { type: 'image' }> =>
            node.id === request.nodeId && node.type === 'image'
        );
        if (!source || source.data.assetId !== request.asset.id) {
          throw new Error(
            t('creativeStudio.canvas.errors.cropSourceChanged', {
              defaultValue:
                '原图片节点已被删除或替换；裁剪素材已保存在素材库中。',
            })
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
          throw new Error(
            t('creativeStudio.canvas.errors.cropNodeConstructionFailed', {
              defaultValue: '裁剪结果未能构造成图片节点。',
            })
          );
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
            t('creativeStudio.canvas.errors.connectCropResult', {
              reason: connectionErrorMessage(validation.code, t),
              defaultValue: '无法连接裁剪结果：{{reason}}。',
            })
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
              ? t('creativeStudio.canvas.notices.cropRecovered', {
                  defaultValue:
                    '上传响应中断后已找回真实裁剪素材，并将派生节点保存到画布。',
                })
              : t('creativeStudio.canvas.notices.cropSucceeded', {
                  defaultValue: '已裁剪真实原图，创建派生图片节点并保存连线。',
                })
          );
        } else {
          setNotice(
            t('creativeStudio.canvas.errors.cropSaveFailed', {
              message: flush.error.message,
              defaultValue: '裁剪素材已上传，但画布保存失败：{{message}}',
            })
          );
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
        setImageSplitError(
          t('creativeStudio.canvas.errors.concurrentUpload', {
            defaultValue: '另一个素材上传仍在进行，请等待完成后重试。',
          })
        );
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
          throw new DOMException('Canvas changed', 'AbortError');
        }

        const current = editor.getState();
        const source = current.document.nodes.find(
          (node): node is Extract<CreativeCanvasNode, { type: 'image' }> =>
            node.id === request.nodeId && node.type === 'image'
        );
        if (!source || source.data.assetId !== request.asset.id) {
          throw new Error(
            t('creativeStudio.canvas.errors.splitSourceChanged', {
              defaultValue: '原图片节点已被删除或替换，未向画布写入切图结果。',
            })
          );
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
            throw new Error(
              t('creativeStudio.canvas.errors.splitNodeConstructionFailed', {
                defaultValue: '切图结果未能构造成图片节点。',
              })
            );
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
              t('creativeStudio.canvas.errors.connectSplitResult', {
                reason: connectionErrorMessage(validation.code, t),
                defaultValue: '无法连接切图结果：{{reason}}。',
              })
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
              ? t('creativeStudio.canvas.notices.splitRecovered', {
                  count: derivedNodes.length,
                  defaultValue:
                    '上传响应中断后已找回切图素材，创建并保存 {{count}} 个图片子节点。',
                })
              : t('creativeStudio.canvas.notices.splitSucceeded', {
                  count: derivedNodes.length,
                  defaultValue:
                    '已切分真实原图，创建并保存 {{count}} 个图片子节点及连线。',
                })
          );
        } else {
          setNotice(
            t('creativeStudio.canvas.errors.splitSaveFailed', {
              message: flush.error.message,
              defaultValue: '切图素材已上传，但画布保存失败：{{message}}',
            })
          );
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
      const runtime = imageTaskRuntimeRef.current?.snapshot();
      const runtimeBlocked =
        !imageTaskRuntimeReady ||
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
            ? t('creativeStudio.canvas.notices.maskTaskBusy', {
                defaultValue:
                  '已有局部编辑任务正在运行、恢复或等待确认，请先处理该任务。',
              })
            : t('creativeStudio.canvas.notices.imageOperationBusy', {
                defaultValue: '已有图片或素材操作正在进行，请等待完成。',
              })
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
      imageTaskRuntimeReady,
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
        ReturnType<CanvasImageTaskRuntimeBridgeHandle['submit']>
      >,
      plan: PreparedCreativeWorkbenchRun
    ) => {
      if (result.kind === 'admitted') {
        setPendingImageMaskEdit(null);
        setImageMaskProgress(null);
        setImageMaskError(null);
        setNotice(
          t('creativeStudio.canvas.notices.maskTaskSubmitted', {
            defaultValue:
              '局部编辑任务已安全提交；对应输入节点会持续显示真实后端状态。',
          })
        );
        return;
      }
      setPendingImageMaskEdit((current) =>
        current
          ? {
              ...current,
              submission: {
                plan,
                reference: canvasImageTaskReferenceFromPlan(plan),
                failureOrder: result.order,
              },
            }
          : current
      );
      setImageMaskError(
        t('creativeStudio.canvas.errors.maskSubmissionUnconfirmed', {
          message: result.error.message,
          defaultValue:
            '任务提交结果尚未确认：{{message}}。请安全重试同一任务，或确认服务器不存在后放弃。',
        })
      );
    },
    []
  );

  const handleConfirmImageMaskEdit = useCallback(
    async (input: CreativeImageMaskEditSubmit) => {
      const request = pendingImageMaskEdit;
      const editor = editorRef.current;
      const runtime = imageTaskRuntimeRef.current;
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
        setImageMaskError(
          t('creativeStudio.canvas.errors.concurrentUpload', {
            defaultValue: '另一个素材上传仍在进行，请等待完成后重试。',
          })
        );
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
          throw new DOMException('Canvas changed', 'AbortError');
        }

        const current = editor.getState();
        const source = current.document.nodes.find(
          (node): node is Extract<CreativeCanvasNode, { type: 'image' }> =>
            node.id === request.nodeId && node.type === 'image'
        );
        if (!source || source.data.assetId !== request.asset.id) {
          throw new Error(
            t('creativeStudio.canvas.errors.maskSourceChanged', {
              defaultValue: '原图片节点已被删除或替换，未创建局部编辑任务。',
            })
          );
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
        canvasOwned = true;

        const result = await runtime.submit(prepared.plan);
        applyImageMaskAdmission(result, prepared.plan);
        if (uploaded.recoveredAfterResponseLoss && result.kind === 'admitted') {
          setNotice(
            t('creativeStudio.canvas.notices.maskReferenceRecovered', {
              defaultValue:
                '上传响应中断后已找回标记参考图，并安全提交局部编辑任务。',
            })
          );
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
              .recoverTask(canvasImageTaskReferenceFromPlan(prepared.plan))
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
            setNotice(
              t('creativeStudio.canvas.errors.taskAdmissionUnconfirmed', {
                message,
                defaultValue:
                  '任务接收状态未确认，已保留同一任务恢复标记：{{message}}',
              })
            );
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
    const runtime = imageTaskRuntimeRef.current;
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
          setNotice(
            t('creativeStudio.canvas.notices.taskRecovered', {
              defaultValue: '服务器已存在该任务，已安全恢复而未重复创建。',
            })
          );
        }
        return;
      }
      await orphanCanvasImageMaskEditTask({
        editor,
        projectId,
        reference: request.submission.reference,
      });
      setImageTaskRuntimeEpoch((value) => value + 1);
      setImageTaskRuntime(INITIAL_CANVAS_TASK_RUNTIME);
      setPendingImageMaskEdit(null);
      setImageMaskProgress(null);
      setNotice(
        t('creativeStudio.canvas.notices.taskConfirmedMissing', {
          defaultValue:
            '已确认服务器不存在该任务；任务状态已记录为失败并清理恢复标记。',
        })
      );
    } catch (error) {
      setImageMaskError(error instanceof Error ? error.message : String(error));
    } finally {
      imageToolBusyRef.current = false;
      setImageMaskBusy(false);
    }
  }, [applyImageMaskAdmission, pendingImageMaskEdit, projectId]);

  const retryImageRuntimeTask = useCallback(
    async (taskId: string) => {
      const runtime = imageTaskRuntimeRef.current;
      if (!runtime || imageTaskRuntimeActionBusy) return;
      setImageTaskRuntimeActionBusy(true);
      try {
        await runtime.retryTask(taskId);
      } catch (error) {
        setNotice(error instanceof Error ? error.message : String(error));
      } finally {
        setImageTaskRuntimeActionBusy(false);
      }
    },
    [imageTaskRuntimeActionBusy]
  );

  const cancelImageRuntimeTask = useCallback(
    async (taskId: string) => {
      const runtime = imageTaskRuntimeRef.current;
      if (!runtime || imageTaskRuntimeActionBusy) return;
      setImageTaskRuntimeActionBusy(true);
      try {
        await runtime.cancelTask(taskId);
      } catch (error) {
        setNotice(error instanceof Error ? error.message : String(error));
      } finally {
        setImageTaskRuntimeActionBusy(false);
      }
    },
    [imageTaskRuntimeActionBusy]
  );

  const applyImageComposeAdmission = useCallback(
    (
      nodeId: string,
      plan: PreparedCreativeWorkbenchRun,
      result: Awaited<ReturnType<CanvasImageTaskRuntimeBridgeHandle['submit']>>
    ) => {
      if (result.kind === 'admitted') {
        setImageComposeSubmission(null);
        setImageComposeIssue(null);
        setNotice(
          t('creativeStudio.canvas.notices.imageTaskSubmitted', {
            defaultValue:
              '图片创作任务已安全提交；对应输入节点会持续显示真实后端状态。',
          })
        );
        return;
      }
      setImageComposeSubmission({
        nodeId,
        plan,
        failureOrder: result.order,
      });
      setImageComposeIssue({
        nodeId,
        message: t('creativeStudio.canvas.errors.submissionUnconfirmed', {
          message: result.error.message,
          defaultValue: '任务提交结果尚未确认：{{message}}。请重试同一任务。',
        }),
      });
    },
    []
  );

  const generateFromCanvasImage = useCallback(
    async (
      nodeId: string,
      prompt: string,
      mentions: readonly CreativeImagePromptMention[],
      settings: ImageWorkbenchSettings
    ) => {
      const editor = editorRef.current;
      const runtime = imageTaskRuntimeRef.current;
      if (!editor || !runtime || imageToolBusyRef.current || imageComposeSubmission) return;
      if (!settings.model || modelCatalog.status !== 'ready') {
        setImageComposeIssue({
          nodeId,
          message: t('creativeStudio.canvas.errors.noImageModel', {
            defaultValue: '没有可用且明确选择的真实图片模型，未发起生成。',
          }),
        });
        return;
      }
      const snapshot = runtime.snapshot();
      if (
        snapshot.submittingCount > 0 ||
        snapshot.recoveringCount > 0 ||
        snapshot.submissionFailures.length > 0 ||
        snapshot.requestError !== null ||
        snapshot.entries.some(
          (entry) => entry.task.status === 'queued' || entry.task.status === 'running'
        )
      ) {
        setImageComposeIssue({
          nodeId,
          message: t('creativeStudio.canvas.errors.imageTaskBusy', {
            defaultValue: '已有图片任务正在处理，请等待完成。',
          }),
        });
        return;
      }

      imageToolBusyRef.current = true;
      setImageComposeBusy(true);
      setImageComposeIssue(null);
      let prepared: ReturnType<typeof prepareCanvasImageCompose> | null = null;
      let canvasOwned = false;
      try {
        const state = editor.getState();
        const sourceNode = state.document.nodes.find(
          (node): node is Extract<CreativeCanvasNode, { type: 'image' }> =>
            node.id === nodeId && node.type === 'image'
        );
        if (!sourceNode) {
          throw new Error(
            t('creativeStudio.canvas.errors.imageNodeRemovedBeforeTask', {
              defaultValue: '图片节点已被删除，未创建图片创作任务。',
            })
          );
        }
        const requiredAssetIds = canvasImageReferenceAssetIds(state, nodeId);
        const resolvedAssets = await Promise.all(
          requiredAssetIds.map(async (assetId) => {
            const cached = knownAssetsRef.current.get(assetId);
            const asset = cached ?? (await creativeAssetClient.get(assetId));
            if (asset.kind !== 'image') {
              throw new Error(
                t('creativeStudio.canvas.image.referenceKindUnsupported', {
                  defaultValue: '已连接素材不是可用图片。',
                })
              );
            }
            return asset;
          })
        );
        if (activeProjectIdRef.current !== projectId) {
          throw new DOMException('Canvas changed', 'AbortError');
        }
        const currentState = editor.getState();
        const currentSource = currentState.document.nodes.find(
          (node): node is Extract<CreativeCanvasNode, { type: 'image' }> =>
            node.id === nodeId && node.type === 'image'
        );
        if (!currentSource) {
          throw new Error(
            t('creativeStudio.canvas.errors.imageNodeRemovedBeforeTask', {
              defaultValue: '图片节点已被删除，未创建图片创作任务。',
            })
          );
        }
        const latestRequiredAssetIds = canvasImageReferenceAssetIds(
          currentState,
          nodeId
        );
        if (
          latestRequiredAssetIds.length !== requiredAssetIds.length ||
          latestRequiredAssetIds.some((assetId, index) => assetId !== requiredAssetIds[index])
        ) {
          throw new Error(
            t('creativeStudio.canvas.errors.imageSourceChangedBeforeTask', {
              defaultValue: '图片节点或其直接参考已变化，未创建图片创作任务。',
            })
          );
        }
        const nextKnownAssets = new Map(knownAssetsRef.current);
        for (const asset of resolvedAssets) nextKnownAssets.set(asset.id, asset);
        knownAssetsRef.current = nextKnownAssets;
        setCanvasReferenceAssets((current) => {
          const next = new Map(current);
          for (const asset of resolvedAssets) next.set(asset.id, asset);
          return next;
        });

        const referenceResolution = resolveCanvasImageReferences(
          currentState,
          nodeId,
          resolvedAssets
        );
        const hasReferences = referenceResolution.references.length > 0;
        const selectedModelOptions = hasReferences
          ? imageMaskModelOptions
          : imageGenerationExactOptions;
        const selectedModel = selectedModelOptions.find(
          (option) =>
            option.providerId === settings.model?.providerId &&
            option.model === settings.model.model
        );
        if (!selectedModel) {
          throw new Error(
            hasReferences
              ? t('creativeStudio.canvas.errors.imageEditModelUnavailable', {
                  defaultValue: '所选图片编辑模型已不可用，未发起生成。',
                })
              : t('creativeStudio.canvas.errors.imageGenerateModelUnavailable', {
                  defaultValue: '所选图片生成模型已不可用，未发起生成。',
                })
          );
        }
        const compilation = compileCanvasImageReferencePrompt(
          prompt,
          mentions.map((mention) => ({
            sourceNodeId: mention.sourceNodeId,
            start: mention.start,
            end: mention.end,
            tokenText: `@${mention.fallbackLabel}`,
          })),
          referenceResolution.references,
          referenceResolution.textReferences
        );
        const inputPolicy = imageReferenceInputPolicy(
          selectedModel.protocol,
          hasReferences ? 'image_edit' : 'image_generation'
        );
        const effectiveInputLimit = effectiveImageReferenceInputLimit(inputPolicy);
        const generationGate = evaluateCanvasImageGenerationGate({
          resolution: referenceResolution,
          compilation,
          maxInputImages: effectiveInputLimit,
        });
        if (!generationGate.allowed || !compilation.ok) {
          throw new Error(
            canvasImageGenerationBlockerMessage(generationGate.blockers[0], t) ??
              t('creativeStudio.canvas.errors.imageReferenceUnavailable', {
                defaultValue: '当前参考图无法安全提交。',
              })
          );
        }
        const sourceAsset = currentSource.data.assetId
          ? (referenceResolution.references.find(
              (reference) =>
                reference.sourceNodeId === currentSource.id &&
                reference.assetId === currentSource.data.assetId
            )?.asset ?? null)
          : null;
        if (currentSource.data.assetId && !sourceAsset) {
          throw new Error(
            t('creativeStudio.canvas.errors.imageSourceChangedBeforeTask', {
              defaultValue: '原图片节点已被删除或替换，未创建图片创作任务。',
            })
          );
        }
        const source = withCanvasImageComposeDraft(currentSource, {
          prompt,
          mentions: structuredClone([...mentions]),
          settings: {
            ...settings,
            model: {
              providerId: selectedModel.providerId,
              model: selectedModel.model,
            },
          },
        });
        editor.dispatch(
          canvasCommands.updateNode(source, {
            mergeKey: `image-composer:${nodeId}`,
          })
        );
        prepared = prepareCanvasImageCompose({
          projectId,
          state: editor.getState(),
          viewportSize: measuredSize(canvasHostRef.current),
          sourceNode: source,
          sourceAsset,
          references: canvasImageWorkbenchReferences(referenceResolution),
          catalog: modelCatalog,
          model: selectedModel,
          prompt,
          providerPrompt: compilation.providerPrompt,
          settings: {
            interfaceMode: settings.interfaceMode,
            quality: settings.quality,
            width: settings.width,
            height: settings.height,
            aspectRatio: settings.aspectRatio,
            count: settings.count,
          },
        });
        const at = Date.now();
        const mergeKey = `image-compose:${source.id}:${prepared.plan.input.idempotencyKey}`;
        editor.dispatch(canvasCommands.addNode(prepared.configNode, { at, mergeKey }));
        editor.dispatch(
          canvasCommands.connect(source.id, prepared.configNode.id, {
            sourceHandle: prepared.connection.sourceHandle,
            targetHandle: prepared.connection.targetHandle,
            at,
            mergeKey,
          })
        );
        canvasOwned = true;
        const result = await runtime.submit(prepared.plan);
        applyImageComposeAdmission(nodeId, prepared.plan, result);
      } catch (error) {
        let message = error instanceof Error ? error.message : String(error);
        if (canvasOwned && prepared) {
          try {
            await editor.addPendingTask(prepared.plan.input.idempotencyKey);
            void runtime
              .recoverTask(canvasImageTaskReferenceFromPlan(prepared.plan))
              .catch((recoveryError) =>
                setNotice(
                  recoveryError instanceof Error
                    ? recoveryError.message
                    : String(recoveryError)
                )
              );
            message = t('creativeStudio.canvas.errors.taskAdmissionUnconfirmed', {
              message,
              defaultValue:
                '任务接收状态未确认，已保留同一任务恢复标记：{{message}}',
            });
          } catch (saveError) {
            message = `${message}；${
              saveError instanceof Error ? saveError.message : String(saveError)
            }`;
          }
        }
        if (activeProjectIdRef.current === projectId) {
          setImageComposeIssue({ nodeId, message });
        }
      } finally {
        imageToolBusyRef.current = false;
        setImageComposeBusy(false);
      }
    },
    [
      applyImageComposeAdmission,
      imageComposeSubmission,
      imageGenerationExactOptions,
      imageMaskModelOptions,
      modelCatalog,
      projectId,
    ]
  );

  const retryCanvasImageComposeSubmission = useCallback(
    async (nodeId: string) => {
      const request = imageComposeSubmission;
      const runtime = imageTaskRuntimeRef.current;
      if (
        !request ||
        request.nodeId !== nodeId ||
        !runtime ||
        imageToolBusyRef.current
      ) {
        return;
      }
      imageToolBusyRef.current = true;
      setImageComposeBusy(true);
      setImageComposeIssue(null);
      try {
        const result = await runtime.retrySubmission(
          request.failureOrder,
          request.plan.input.idempotencyKey
        );
        applyImageComposeAdmission(nodeId, request.plan, result);
      } catch (error) {
        setImageComposeIssue({
          nodeId,
          message: error instanceof Error ? error.message : String(error),
        });
      } finally {
        imageToolBusyRef.current = false;
        setImageComposeBusy(false);
      }
    },
    [applyImageComposeAdmission, imageComposeSubmission]
  );

  const applyVideoComposeAdmission = useCallback(
    (
      nodeId: string,
      plan: PreparedCreativeWorkbenchRun,
      result: Awaited<ReturnType<CanvasVideoTaskRuntimeBridgeHandle['submit']>>
    ) => {
      if (result.kind === 'admitted') {
        setVideoComposeSubmission(null);
        setVideoComposeIssue(null);
        setNotice(
          t('creativeStudio.canvas.notices.videoTaskSubmitted', {
            defaultValue:
              '视频创作任务已安全提交；对应输入节点会持续显示真实后端状态。',
          })
        );
        return;
      }
      setVideoComposeSubmission({ nodeId, plan, failureOrder: result.order });
      setVideoComposeIssue({
        nodeId,
        message: t('creativeStudio.canvas.errors.submissionUnconfirmed', {
          message: result.error.message,
          defaultValue: '任务提交结果尚未确认：{{message}}。请重试同一任务。',
        }),
      });
    },
    []
  );

  const generateFromCanvasVideo = useCallback(
    async (
      nodeId: string,
      prompt: string,
      settings: CanvasVideoComposeSettings
    ) => {
      const editor = editorRef.current;
      const runtime = videoTaskRuntimeRef.current;
      if (
        !editor ||
        !runtime ||
        videoComposeBusy ||
        videoComposeSubmission
      ) {
        return;
      }
      if (!settings.model || modelCatalog.status !== 'ready') {
        setVideoComposeIssue({
          nodeId,
          message: t('creativeStudio.canvas.errors.noVideoModel', {
            defaultValue: '没有可用且明确选择的真实视频模型，未发起生成。',
          }),
        });
        return;
      }
      const snapshot = runtime.snapshot();
      if (
        snapshot.submittingCount > 0 ||
        snapshot.recoveringCount > 0 ||
        snapshot.submissionFailures.length > 0 ||
        snapshot.requestError !== null ||
        snapshot.entries.some(
          (entry) => entry.task.status === 'queued' || entry.task.status === 'running'
        )
      ) {
        setVideoComposeIssue({
          nodeId,
          message: t('creativeStudio.canvas.errors.videoTaskBusy', {
            defaultValue: '已有视频任务正在处理，请等待完成。',
          }),
        });
        return;
      }

      setVideoComposeBusy(true);
      setVideoComposeIssue(null);
      let prepared: ReturnType<typeof prepareCanvasVideoCompose> | null = null;
      let canvasOwned = false;
      try {
        const state = editor.getState();
        const source = state.document.nodes.find(
          (node): node is Extract<CreativeCanvasNode, { type: 'video' }> =>
            node.id === nodeId && node.type === 'video'
        );
        if (!source) {
          throw new Error(
            t('creativeStudio.canvas.errors.videoNodeRemovedBeforeTask', {
              defaultValue: '视频节点已被删除，未创建视频创作任务。',
            })
          );
        }
        const mode = canvasVideoComposeMode(state.document, nodeId);
        if (mode.kind === 'unsupported') throw new Error(mode.message);
        const selectedModel = videoModelOptions.find(
          (option) =>
            option.providerId === settings.model?.providerId &&
            option.model === settings.model.model
        );
        if (!selectedModel) {
          throw new Error(
            t('creativeStudio.canvas.errors.videoModelUnavailable', {
              defaultValue: '所选视频模型已不可用，未发起生成。',
            })
          );
        }
        const reference =
          mode.kind === 'i2v'
            ? (knownAssetsRef.current.get(mode.assetId) ??
              (await creativeAssetClient.get(mode.assetId)))
            : null;
        if (reference && reference.kind !== 'image') {
          throw new Error(
            t('creativeStudio.canvas.errors.videoReferenceResolutionFailed', {
              defaultValue: 'I2V 引用没有解析为真实图片素材。',
            })
          );
        }
        if (reference) {
          knownAssetsRef.current = new Map(knownAssetsRef.current).set(
            reference.id,
            reference
          );
        }
        if (activeProjectIdRef.current !== projectId) {
          throw new DOMException('Canvas changed', 'AbortError');
        }
        const currentState = editor.getState();
        const currentSource = currentState.document.nodes.find(
          (node): node is Extract<CreativeCanvasNode, { type: 'video' }> =>
            node.id === nodeId && node.type === 'video'
        );
        const currentMode = canvasVideoComposeMode(currentState.document, nodeId);
        if (
          !currentSource ||
          currentSource.data.assetId !== null ||
          currentMode.kind !== mode.kind ||
          (mode.kind === 'i2v' &&
            (currentMode.kind !== 'i2v' || currentMode.assetId !== mode.assetId))
        ) {
          throw new Error(
            t('creativeStudio.canvas.errors.videoSourceChangedBeforeTask', {
              defaultValue: '视频节点或其直接引用已变化，未创建视频创作任务。',
            })
          );
        }
        const durableSource = withCanvasVideoComposeDraft(currentSource, {
          prompt,
          settings: {
            ...settings,
            model: {
              providerId: selectedModel.providerId,
              model: selectedModel.model,
            },
          },
        });
        editor.dispatch(
          canvasCommands.updateNode(durableSource, {
            mergeKey: `video-composer:${nodeId}`,
          })
        );
        prepared = prepareCanvasVideoCompose({
          projectId,
          state: editor.getState(),
          viewportSize: measuredSize(canvasHostRef.current),
          sourceNode: durableSource,
          sourceAsset: null,
          catalog: modelCatalog,
          model: selectedModel,
          operation: {
            task: 'video_generation',
            capability: mode.kind === 'i2v' ? 'i2v' : 't2v',
          },
          references: reference
            ? {
                assets: [reference],
                bindings: [
                  {
                    assetId: reference.id,
                    kind: 'image',
                    role: 'reference',
                  },
                ],
              }
            : { assets: [], bindings: [] },
          prompt,
          settings: {
            resolution: settings.resolution,
            aspectRatio: settings.aspectRatio,
            seconds: settings.seconds,
          },
        });
        const at = Date.now();
        const mergeKey = `video-compose:${nodeId}:${prepared.plan.input.idempotencyKey}`;
        editor.dispatch(canvasCommands.addNode(prepared.configNode, { at, mergeKey }));
        editor.dispatch(
          canvasCommands.connect(nodeId, prepared.configNode.id, {
            sourceHandle: prepared.connection.sourceHandle,
            targetHandle: prepared.connection.targetHandle,
            at,
            mergeKey,
          })
        );
        canvasOwned = true;
        const result = await runtime.submit(prepared.plan);
        applyVideoComposeAdmission(nodeId, prepared.plan, result);
      } catch (error) {
        let message = error instanceof Error ? error.message : String(error);
        if (canvasOwned && prepared) {
          try {
            await editor.addPendingTask(prepared.plan.input.idempotencyKey);
            void runtime
              .recoverTask(canvasVideoTaskReferenceFromPlan(prepared.plan))
              .catch((recoveryError) =>
                setNotice(
                  recoveryError instanceof Error
                    ? recoveryError.message
                    : String(recoveryError)
                )
              );
            message = t('creativeStudio.canvas.errors.taskAdmissionUnconfirmed', {
              message,
              defaultValue:
                '任务接收状态未确认，已保留同一任务恢复标记：{{message}}',
            });
          } catch (saveError) {
            message = `${message}；${
              saveError instanceof Error ? saveError.message : String(saveError)
            }`;
          }
        }
        if (activeProjectIdRef.current === projectId) {
          setVideoComposeIssue({ nodeId, message });
        }
      } finally {
        setVideoComposeBusy(false);
      }
    },
    [
      applyVideoComposeAdmission,
      modelCatalog,
      projectId,
      videoComposeBusy,
      videoComposeSubmission,
      videoModelOptions,
    ]
  );

  const retryCanvasVideoComposeSubmission = useCallback(
    async (nodeId: string) => {
      const request = videoComposeSubmission;
      const runtime = videoTaskRuntimeRef.current;
      if (!request || request.nodeId !== nodeId || !runtime || videoComposeBusy) {
        return;
      }
      setVideoComposeBusy(true);
      setVideoComposeIssue(null);
      try {
        const result = await runtime.retrySubmission(
          request.failureOrder,
          request.plan.input.idempotencyKey
        );
        applyVideoComposeAdmission(nodeId, request.plan, result);
      } catch (error) {
        setVideoComposeIssue({
          nodeId,
          message: error instanceof Error ? error.message : String(error),
        });
      } finally {
        setVideoComposeBusy(false);
      }
    },
    [applyVideoComposeAdmission, videoComposeBusy, videoComposeSubmission]
  );

  const confirmCanvasVideoComposeSubmission = useCallback(
    async (nodeId: string) => {
      const request = videoComposeSubmission;
      const runtime = videoTaskRuntimeRef.current;
      const editor = editorRef.current;
      if (
        !request ||
        request.nodeId !== nodeId ||
        !runtime ||
        !editor ||
        videoComposeBusy
      ) {
        return;
      }
      setVideoComposeBusy(true);
      setVideoComposeIssue(null);
      const reference = canvasVideoTaskReferenceFromPlan(request.plan);
      try {
        const exists = await runtime.taskExists(reference);
        if (activeProjectIdRef.current !== projectId) return;
        if (exists) {
          const result = await runtime.retrySubmission(
            request.failureOrder,
            request.plan.input.idempotencyKey
          );
          applyVideoComposeAdmission(nodeId, request.plan, result);
          if (result.kind === 'admitted') {
            setNotice(
              t('creativeStudio.canvas.notices.videoTaskRecovered', {
                defaultValue: '服务器已存在该视频任务，已安全恢复而未重复创建。',
              })
            );
          }
          return;
        }
        await orphanCanvasVideoComposeTask({
          editor,
          projectId,
          reference,
        });
        if (activeProjectIdRef.current !== projectId) return;
        setVideoComposeSubmission(null);
        setVideoTaskRuntimeEpoch((value) => value + 1);
        setVideoComposeIssue({
          nodeId,
          message: t('creativeStudio.canvas.notices.videoTaskMissing', {
            defaultValue:
              '服务器确认未创建该视频任务，已清理恢复标记，可以重新生成。',
          }),
        });
      } catch (error) {
        if (activeProjectIdRef.current === projectId) {
          setVideoComposeIssue({
            nodeId,
            message: error instanceof Error ? error.message : String(error),
          });
        }
      } finally {
        if (activeProjectIdRef.current === projectId) {
          setVideoComposeBusy(false);
        }
      }
    },
    [
      applyVideoComposeAdmission,
      projectId,
      videoComposeBusy,
      videoComposeSubmission,
    ]
  );

  const retryVideoRuntimeTask = useCallback(
    async (taskId: string) => {
      const runtime = videoTaskRuntimeRef.current;
      if (!runtime || videoTaskRuntimeActionBusy) return;
      setVideoTaskRuntimeActionBusy(true);
      try {
        await runtime.retryTask(taskId);
      } catch (error) {
        setNotice(error instanceof Error ? error.message : String(error));
      } finally {
        setVideoTaskRuntimeActionBusy(false);
      }
    },
    [videoTaskRuntimeActionBusy]
  );

  const cancelVideoRuntimeTask = useCallback(
    async (taskId: string) => {
      const runtime = videoTaskRuntimeRef.current;
      if (!runtime || videoTaskRuntimeActionBusy) return;
      setVideoTaskRuntimeActionBusy(true);
      try {
        await runtime.cancelTask(taskId);
      } catch (error) {
        setNotice(error instanceof Error ? error.message : String(error));
      } finally {
        setVideoTaskRuntimeActionBusy(false);
      }
    },
    [videoTaskRuntimeActionBusy]
  );

  const applyAudioComposeAdmission = useCallback(
    (
      nodeId: string,
      plan: PreparedCreativeWorkbenchRun,
      result: Awaited<ReturnType<CanvasAudioTaskRuntimeBridgeHandle['submit']>>
    ) => {
      if (result.kind === 'admitted') {
        setAudioComposeSubmission(null);
        setAudioComposeIssue(null);
        setNotice(
          t('creativeStudio.canvas.notices.audioTaskSubmitted', {
            defaultValue:
              '音频创作任务已安全提交；对应输入节点会持续显示真实后端状态。',
          })
        );
        return;
      }
      setAudioComposeSubmission({ nodeId, plan, failureOrder: result.order });
      setAudioComposeIssue({
        nodeId,
        message: t('creativeStudio.canvas.errors.submissionUnconfirmed', {
          message: result.error.message,
          defaultValue: '任务提交结果尚未确认：{{message}}。请重试同一任务。',
        }),
      });
    },
    []
  );

  const generateFromCanvasAudio = useCallback(
    async (
      nodeId: string,
      prompt: string,
      settings: CanvasAudioComposeSettings
    ) => {
      const editor = editorRef.current;
      const runtime = audioTaskRuntimeRef.current;
      if (!editor || !runtime || audioComposeBusy || audioComposeSubmission) {
        return;
      }
      if (!settings.model || modelCatalog.status !== 'ready') {
        setAudioComposeIssue({
          nodeId,
          message: t('creativeStudio.canvas.errors.noAudioModel', {
            defaultValue: '没有可用且明确选择的真实语音合成模型，未发起生成。',
          }),
        });
        return;
      }
      const snapshot = runtime.snapshot();
      if (
        snapshot.submittingCount > 0 ||
        snapshot.recoveringCount > 0 ||
        snapshot.submissionFailures.length > 0 ||
        snapshot.requestError !== null ||
        snapshot.entries.some(
          (entry) =>
            entry.task.status === 'queued' || entry.task.status === 'running'
        )
      ) {
        setAudioComposeIssue({
          nodeId,
          message: t('creativeStudio.canvas.errors.audioTaskBusy', {
            defaultValue: '已有音频任务正在处理，请等待完成。',
          }),
        });
        return;
      }

      setAudioComposeBusy(true);
      setAudioComposeIssue(null);
      let prepared: ReturnType<typeof prepareCanvasAudioCompose> | null = null;
      let canvasOwned = false;
      try {
        const state = editor.getState();
        const source = state.document.nodes.find(
          (node): node is Extract<CreativeCanvasNode, { type: 'audio' }> =>
            node.id === nodeId && node.type === 'audio'
        );
        if (!source) {
          throw new Error(
            t('creativeStudio.canvas.errors.audioNodeRemovedBeforeTask', {
              defaultValue: '音频节点已被删除，未创建音频创作任务。',
            })
          );
        }
        const eligibility = canvasAudioComposeEligibility(
          state.document,
          nodeId
        );
        if (eligibility.kind === 'unsupported') {
          throw new Error(eligibility.message);
        }
        const selectedModel = audioModelOptions.find(
          (option) =>
            option.providerId === settings.model?.providerId &&
            option.model === settings.model.model
        );
        if (!selectedModel) {
          throw new Error(
            t('creativeStudio.canvas.errors.audioModelUnavailable', {
              defaultValue: '所选语音合成模型已不可用，未发起生成。',
            })
          );
        }
        if (activeProjectIdRef.current !== projectId) {
          throw new DOMException('Canvas changed', 'AbortError');
        }
        const currentState = editor.getState();
        const currentSource = currentState.document.nodes.find(
          (node): node is Extract<CreativeCanvasNode, { type: 'audio' }> =>
            node.id === nodeId && node.type === 'audio'
        );
        const currentEligibility = canvasAudioComposeEligibility(
          currentState.document,
          nodeId
        );
        if (!currentSource || currentEligibility.kind !== 'tts') {
          throw new Error(
            t('creativeStudio.canvas.errors.audioSourceChangedBeforeTask', {
              defaultValue: '音频节点或其直接引用已变化，未创建音频创作任务。',
            })
          );
        }
        const durableSource = withCanvasAudioComposeDraft(currentSource, {
          prompt,
          settings: {
            ...settings,
            model: {
              providerId: selectedModel.providerId,
              model: selectedModel.model,
            },
          },
        });
        editor.dispatch(
          canvasCommands.updateNode(durableSource, {
            mergeKey: `audio-composer:${nodeId}`,
          })
        );
        prepared = prepareCanvasAudioCompose({
          projectId,
          state: editor.getState(),
          viewportSize: measuredSize(canvasHostRef.current),
          sourceNode: durableSource,
          sourceAsset: null,
          catalog: modelCatalog,
          model: selectedModel,
          references: { assets: [], bindings: [] },
          prompt,
          settings: {
            voice: settings.voice,
            format: settings.format,
          },
        });
        const at = Date.now();
        const mergeKey = `audio-compose:${nodeId}:${prepared.plan.input.idempotencyKey}`;
        editor.dispatch(
          canvasCommands.addNode(prepared.configNode, { at, mergeKey })
        );
        editor.dispatch(
          canvasCommands.connect(nodeId, prepared.configNode.id, {
            sourceHandle: prepared.connection.sourceHandle,
            targetHandle: prepared.connection.targetHandle,
            at,
            mergeKey,
          })
        );
        canvasOwned = true;
        const result = await runtime.submit(prepared.plan);
        applyAudioComposeAdmission(nodeId, prepared.plan, result);
      } catch (error) {
        let message = error instanceof Error ? error.message : String(error);
        if (canvasOwned && prepared) {
          try {
            await editor.addPendingTask(prepared.plan.input.idempotencyKey);
            void runtime
              .recoverTask(canvasAudioTaskReferenceFromPlan(prepared.plan))
              .catch((recoveryError) =>
                setNotice(
                  recoveryError instanceof Error
                    ? recoveryError.message
                    : String(recoveryError)
                )
              );
            message = t('creativeStudio.canvas.errors.taskAdmissionUnconfirmed', {
              message,
              defaultValue:
                '任务接收状态未确认，已保留同一任务恢复标记：{{message}}',
            });
          } catch (saveError) {
            message = `${message}；${
              saveError instanceof Error ? saveError.message : String(saveError)
            }`;
          }
        }
        if (activeProjectIdRef.current === projectId) {
          setAudioComposeIssue({ nodeId, message });
        }
      } finally {
        if (activeProjectIdRef.current === projectId) {
          setAudioComposeBusy(false);
        }
      }
    },
    [
      applyAudioComposeAdmission,
      audioComposeBusy,
      audioComposeSubmission,
      audioModelOptions,
      modelCatalog,
      projectId,
    ]
  );

  const retryCanvasAudioComposeSubmission = useCallback(
    async (nodeId: string) => {
      const request = audioComposeSubmission;
      const runtime = audioTaskRuntimeRef.current;
      if (!request || request.nodeId !== nodeId || !runtime || audioComposeBusy) {
        return;
      }
      setAudioComposeBusy(true);
      setAudioComposeIssue(null);
      try {
        const result = await runtime.retrySubmission(
          request.failureOrder,
          request.plan.input.idempotencyKey
        );
        applyAudioComposeAdmission(nodeId, request.plan, result);
      } catch (error) {
        setAudioComposeIssue({
          nodeId,
          message: error instanceof Error ? error.message : String(error),
        });
      } finally {
        setAudioComposeBusy(false);
      }
    },
    [applyAudioComposeAdmission, audioComposeBusy, audioComposeSubmission]
  );

  const confirmCanvasAudioComposeSubmission = useCallback(
    async (nodeId: string) => {
      const request = audioComposeSubmission;
      const runtime = audioTaskRuntimeRef.current;
      const editor = editorRef.current;
      if (
        !request ||
        request.nodeId !== nodeId ||
        !runtime ||
        !editor ||
        audioComposeBusy
      ) {
        return;
      }
      setAudioComposeBusy(true);
      setAudioComposeIssue(null);
      const reference = canvasAudioTaskReferenceFromPlan(request.plan);
      try {
        const exists = await runtime.taskExists(reference);
        if (activeProjectIdRef.current !== projectId) return;
        if (exists) {
          const result = await runtime.retrySubmission(
            request.failureOrder,
            request.plan.input.idempotencyKey
          );
          applyAudioComposeAdmission(nodeId, request.plan, result);
          if (result.kind === 'admitted') {
            setNotice(
              t('creativeStudio.canvas.notices.audioTaskRecovered', {
                defaultValue: '服务器已存在该音频任务，已安全恢复而未重复创建。',
              })
            );
          }
          return;
        }
        await orphanCanvasAudioComposeTask({ editor, projectId, reference });
        if (activeProjectIdRef.current !== projectId) return;
        setAudioComposeSubmission(null);
        setAudioTaskRuntimeEpoch((value) => value + 1);
        setAudioComposeIssue({
          nodeId,
          message: t('creativeStudio.canvas.notices.audioTaskMissing', {
            defaultValue:
              '服务器确认未创建该音频任务，已清理恢复标记，可以重新生成。',
          }),
        });
      } catch (error) {
        if (activeProjectIdRef.current === projectId) {
          setAudioComposeIssue({
            nodeId,
            message: error instanceof Error ? error.message : String(error),
          });
        }
      } finally {
        if (activeProjectIdRef.current === projectId) {
          setAudioComposeBusy(false);
        }
      }
    },
    [
      applyAudioComposeAdmission,
      audioComposeBusy,
      audioComposeSubmission,
      projectId,
    ]
  );

  const retryAudioRuntimeTask = useCallback(
    async (taskId: string) => {
      const runtime = audioTaskRuntimeRef.current;
      if (!runtime || audioTaskRuntimeActionBusy) return;
      setAudioTaskRuntimeActionBusy(true);
      try {
        await runtime.retryTask(taskId);
      } catch (error) {
        setNotice(error instanceof Error ? error.message : String(error));
      } finally {
        setAudioTaskRuntimeActionBusy(false);
      }
    },
    [audioTaskRuntimeActionBusy]
  );

  const cancelAudioRuntimeTask = useCallback(
    async (taskId: string) => {
      const runtime = audioTaskRuntimeRef.current;
      if (!runtime || audioTaskRuntimeActionBusy) return;
      setAudioTaskRuntimeActionBusy(true);
      try {
        await runtime.cancelTask(taskId);
      } catch (error) {
        setNotice(error instanceof Error ? error.message : String(error));
      } finally {
        setAudioTaskRuntimeActionBusy(false);
      }
    },
    [audioTaskRuntimeActionBusy]
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
      setNotice(
        t('creativeStudio.canvas.notices.clipboardTextInserted', {
          defaultValue: '已从真实系统剪贴板插入文本。',
        })
      );
      return true;
    },
    []
  );

  const readSystemClipboard = useCallback(
    async (worldPosition: CanvasPoint) => {
      try {
        if (typeof navigator === 'undefined' || !navigator.clipboard) {
          throw new Error(
            t('creativeStudio.canvas.errors.clipboardUnavailable', {
              defaultValue: '当前运行环境不提供系统剪贴板读取能力。',
            })
          );
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
        setNotice(
          t('creativeStudio.canvas.notices.clipboardEmpty', {
            defaultValue: '系统剪贴板中没有可插入的真实文本、图片或视频。',
          })
        );
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
        setNotice(
          t('creativeStudio.canvas.notices.addDirectorFirst', {
            defaultValue: '请先添加导演节点，再进入 3D 导演台。',
          })
        );
        return;
      }
      if (directors.length > 1) {
        editor.dispatch(
          canvasCommands.setSelection(directors.map((node) => node.id))
        );
        setNotice(
          t('creativeStudio.canvas.notices.resolveDirectorConflict', {
            defaultValue:
              '画布存在多个导演节点。请只保留一个，再进入 3D 导演台。',
          })
        );
        return;
      }
      const director = directors[0];
      if (requestedNodeId && requestedNodeId !== director.id) {
        setNotice(
          t('creativeStudio.canvas.notices.directorMissing', {
            defaultValue: '请求的导演节点已不存在，请从时间线面板重新打开。',
          })
        );
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
            selectedEdgeCount: editorRef.current?.getState().selection.edgeIds.length ?? 0,
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
          setNotice(
            t('creativeStudio.canvas.connection.createFailed', {
              reason: connectionErrorMessage(intent.code, t),
              defaultValue: '无法创建连接：{{reason}}。',
            })
          );
          return;
        case 'connection/batch-created':
          setNotice(t('creativeStudio.canvas.notices.connectionsCreated', {
            count: intent.count,
            skipped: intent.skippedCount,
          }));
          return;
        case 'connection/created':
          setNotice(
            t('creativeStudio.canvas.notices.connectionCreated', {
              defaultValue: '已创建连接。',
            })
          );
          return;
        case 'node/open': {
          const node = editorRef.current
            ?.getState()
            .document.nodes.find(
              (candidate) => candidate.id === intent.nodeId
            );
          dispatch(canvasCommands.setSelection([intent.nodeId]));
          dismissInteractionOverlays();
          if (intent.mode === 'edit-text') {
            if (node?.type !== 'text') {
              setEditingTextNodeId(null);
              return;
            }
            if (node.locked) {
              setEditingTextNodeId(null);
              setNotice(
                t('creativeStudio.canvas.nodes.locked', {
                  defaultValue: '节点已锁定',
                })
              );
              return;
            }
            setEditingTextNodeId(node.id);
            return;
          }
          setEditingTextNodeId(null);
          if (intent.mode === 'open-director') {
            await handleOpenDirector(intent.nodeId);
            return;
          }
          persistPanels(
            withCreativeCanvasRightView(panelsRef.current, 'properties')
          );
          setNotice(
            t('creativeStudio.canvas.notices.propertiesOpened', {
              defaultValue: '已在属性面板打开所选节点。',
            })
          );
          return;
        }
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
            ? t('creativeStudio.canvas.upload.rejectedFiles', {
                count: intent.rejected.length,
                fileName: first.fileName,
                reason: manualUploadRejectionMessage(first.reason, t),
                defaultValue:
                  '{{count}} 个文件未导入（{{fileName}}: {{reason}}）',
              })
            : '';
          const ignored = intent.ignoredAcceptedFileNames.length
            ? t('creativeStudio.canvas.upload.ignoredFiles', {
                count: intent.ignoredAcceptedFileNames.length,
                defaultValue: '{{count}} 个额外文件按源产品规则未处理',
              })
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
    (kind: CreativeCanvasUserNodeKind) => {
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
        setNotice(
          t('creativeStudio.canvas.notices.directorCreateConflict', {
            defaultValue:
              '画布存在多个导演节点，请先处理冲突，未创建新的导演节点。',
          })
        );
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
        const candidateDocument = {
          ...state.document,
          nodes: reusedDirector
            ? state.document.nodes
            : [...state.document.nodes, node],
        };
        const at = Date.now();
        const mergeKey = `create-connected:${node.id}`;
        const resolution = finishCanvasConnectionDrag(candidateDocument, {
          ...menu.connection,
          kind: 'connection',
          pointerId: 0,
          clientPosition: { x: 0, y: 0 },
        }, 0, { nodeId: node.id }, { at, mergeKey });
        const rejection = resolution.intents.find((intent) => intent.type === 'connection/rejected');
        if (rejection) {
          setNotice(
            t('creativeStudio.canvas.connection.createFailed', {
              reason: connectionErrorMessage(rejection.code, t),
              defaultValue: '无法创建连接：{{reason}}。',
            })
          );
          return;
        }

        if (!reusedDirector) {
          editor.dispatch(canvasCommands.addNode(node, { at, mergeKey }));
        }
        for (const command of resolution.commands) editor.dispatch(command);
        editor.dispatch(canvasCommands.setSelection([node.id]));
        const batch = resolution.intents.find((intent) => intent.type === 'connection/batch-created');
        setNotice(
          batch ? t('creativeStudio.canvas.notices.connectionsCreated', { count: batch.count, skipped: batch.skippedCount }) : reusedDirector
            ? t('creativeStudio.canvas.notices.directorReusedAndConnected', {
                defaultValue: '已复用画布唯一的导演节点并完成连接。',
              })
            : t('creativeStudio.canvas.notices.nodeCreatedAndConnected', {
                defaultValue: '已创建节点并完成连接。',
              })
        );
      } else {
        if (reusedDirector) {
          editor.dispatch(canvasCommands.setSelection([node.id]));
          setNotice(
            t('creativeStudio.canvas.notices.directorSelected', {
              defaultValue: '画布已有唯一导演节点，已为你选中。',
            })
          );
        } else {
          editor.dispatch(canvasCommands.addNode(node));
          setNotice(
            t('creativeStudio.canvas.notices.nodeCreatedAtPosition', {
              defaultValue: '已在指定位置创建节点。',
            })
          );
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

  const handleBackToCanvases = useCallback(async () => {
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
      if (reloaded) {
        setAgentOpsReloadFence(false);
        agentPanelRef.current?.refreshAuthority();
      }
      setNotice(
        reloaded
          ? t('creativeStudio.canvas.notices.remoteReloaded', {
              defaultValue: '已重新载入远端版本。',
            })
          : t('creativeStudio.canvas.notices.remoteUnavailable', {
              defaultValue: '远端版本暂时不可用。',
            })
      );
    } catch {
      setNotice(
        t('creativeStudio.canvas.notices.remoteUnavailable', {
          defaultValue: '远端版本暂时不可用。',
        })
      );
    } finally {
      setRecoveryBusy(false);
    }
  }, [recoveryBusy, setAgentOpsReloadFence]);

  const handleRetrySave = useCallback(async () => {
    if (!editorRef.current || recoveryBusy) return;
    setRecoveryBusy(true);
    try {
      const result = await editorRef.current.flush();
      setNotice(
        result.status === 'saved' || result.status === 'noop'
          ? t('creativeStudio.canvas.notices.saveCompleted', {
              defaultValue: '保存已完成。',
            })
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
            {
              cascadeIndex: state.document.nodes.filter(
                isCreativeCanvasUserNode
              ).length,
            }
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
          ? t('creativeStudio.canvas.notices.assetsPartiallyInserted', {
              inserted,
              failed: errors.length,
              error: errors[0],
              defaultValue:
                '{{inserted}} 项已插入；{{failed}} 项未插入：{{error}}',
            })
          : t('creativeStudio.canvas.notices.assetsInserted', {
              count: inserted,
              defaultValue: '{{count}} 项素材已插入画布。',
            })
      );
    },
    [prepareCenteredInsertion]
  );

  const handleInsertTemplateResults = useCallback(
    async (run: CreativeTemplateRunAggregateV1) => {
      if (templateInsertingRunId || run.record.resultAssetIds.length === 0)
        return;
      setTemplateInsertingRunId(run.request.id);
        setNotice(
          t('creativeStudio.canvas.notices.resolvingTemplateResults', {
            defaultValue: '正在解析模板的真实结果素材…',
          })
        );
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
        setTemplateInsertingRunId(null);
      }
    },
    [assets, handleInsertAssets, templateInsertingRunId]
  );

  const handleCopyPrompt = useCallback((selection: PromptLibrarySelection) => {
    void copyText(selection.prompt)
      .then(() => {
        setSelectedPromptId(selection.id);
        setNotice(
          t('creativeStudio.canvas.notices.promptCopied', {
            title: selection.title,
            defaultValue: '已复制“{{title}}”的提示词到剪贴板。',
          })
        );
      })
      .catch(() => {
        setNotice(
          t('creativeStudio.canvas.errors.promptCopyFailed', {
            defaultValue: '提示词复制失败，请检查剪贴板权限。',
          })
        );
      });
  }, []);

  const selection = useMemo(
    () => creativeCanvasProductSelectionCapabilities(canvasState),
    [canvasState]
  );
  const panelViews = creativeCanvasProductPanelViews(panels);
  const [agentContextDocument, agentContextSelectedNodeIds] =
    selectCreativeCanvasAgentContextInputs(
      canvasState,
      panelViews.right === 'assistant'
    );
  const agentPlanningContext = useMemo(() => {
    if (
      !agentContextDocument ||
      !agentContextSelectedNodeIds ||
      save.revision === null
    ) {
      return null;
    }
    return buildCreativeCanvasAgentContext({
      document: {
        projectId,
        nodes: agentContextDocument.nodes,
        connections: agentContextDocument.connections,
      },
      canvasRevision: save.revision,
      selectedNodeIds: agentContextSelectedNodeIds,
    });
  }, [
    agentContextDocument,
    agentContextSelectedNodeIds,
    projectId,
    save.revision,
  ]);
  const productDisabled =
    save.revision === null ||
    recoveryBusy ||
    agentOpsApplyBusy ||
    agentOpsReloadRequired;
  useEffect(() => {
    if (productDisabled) setEditingTextNodeId(null);
  }, [productDisabled]);
  const imageTaskRuntimeBlocksNew =
    imageTaskRuntime.submittingCount > 0 ||
    imageTaskRuntime.recoveringCount > 0 ||
    imageTaskRuntime.submissionFailures.length > 0 ||
    imageTaskRuntime.requestError !== null ||
    imageTaskRuntime.entries.some(
      (entry) =>
        entry.task.status === 'queued' || entry.task.status === 'running'
    );
  const videoTaskRuntimeBlocksNew =
    videoTaskRuntime.submittingCount > 0 ||
    videoTaskRuntime.recoveringCount > 0 ||
    videoTaskRuntime.submissionFailures.length > 0 ||
    videoTaskRuntime.requestError !== null ||
    videoTaskRuntime.entries.some(
      (entry) =>
        entry.task.status === 'queued' || entry.task.status === 'running'
    );
  const audioTaskRuntimeBlocksNew =
    audioTaskRuntime.submittingCount > 0 ||
    audioTaskRuntime.recoveringCount > 0 ||
    audioTaskRuntime.submissionFailures.length > 0 ||
    audioTaskRuntime.requestError !== null ||
    audioTaskRuntime.entries.some(
      (entry) =>
        entry.task.status === 'queued' || entry.task.status === 'running'
    );
  const canvasTitle =
    project.detail?.project.title ??
    (project.isLoading
      ? t('creativeStudio.canvas.loadingTitle', {
          defaultValue: '正在载入画布…',
        })
      : t('creativeStudio.canvas.untitled', {
          defaultValue: '无限画布',
        }));
  const saveMessage = creativeCanvasSaveDisplayMessage(save);
  const compact = viewportSize.width < 760;
  const canvasLayoutStyle = {
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
      title={t('creativeStudio.canvas.loading.outlineTitle', {
        defaultValue: '正在载入画布结构',
      })}
      description={t('creativeStudio.canvas.loading.documentValidation', {
        defaultValue: '等待画布文档通过 canonical v1 校验。',
      })}
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
      title={t('creativeStudio.canvas.loading.propertiesTitle', {
        defaultValue: '正在载入属性',
      })}
      description={t('creativeStudio.canvas.loading.propertiesDescription', {
        defaultValue: '选择真实节点后才能查看 canonical 属性。',
      })}
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
      title={t('creativeStudio.canvas.loading.historyTitle', {
        defaultValue: '正在载入撤销状态',
      })}
      description={t('creativeStudio.canvas.loading.historyDescription', {
        defaultValue: '历史面板仅展示当前编辑会话的真实撤销栈。',
      })}
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
      title={t('creativeStudio.canvas.loading.timelineTitle', {
        defaultValue: '正在载入导演时间线',
      })}
      description={t('creativeStudio.canvas.loading.documentValidation', {
        defaultValue: '等待画布文档通过 canonical v1 校验。',
      })}
    />
  );

  return (
    <main
      className={styles.root}
      style={canvasLayoutStyle}
      data-creative-canvas-product-route
      data-canvas-id={canvasId}
    >
      <CreativeCanvasChrome
        canvasTitle={canvasTitle}
        saveStatus={save.status}
        saveMessage={saveMessage}
        tool={tool}
        canUndo={Boolean(canvasState && canUndoCanvas(canvasState))}
        canRedo={Boolean(canvasState && canRedoCanvas(canvasState))}
        leftOpen={panels.left.open}
        leftView={panelViews.left}
        rightView={panelViews.right}
        rightPanelWidth={panels.right.width}
        bottomView={panelViews.bottom}
        compact={compact}
        disabled={productDisabled}
        onBackToCanvases={() => void handleBackToCanvases()}
        onToolChange={setTool}
        onAddNode={addNode}
        onUndo={() => dispatch(canvasCommands.undo())}
        onRedo={() => dispatch(canvasCommands.redo())}
        onLeftPanelOpenChange={handleLeftPanelOpenChange}
        onLeftViewChange={handleLeftViewChange}
        onRightViewChange={handleRightViewChange}
        onRightPanelWidthChange={handleRightPanelWidthChange}
        onBottomViewChange={handleBottomViewChange}
        slots={{
          canvas: (
            <div ref={canvasHostRef} className={styles.canvasHost}>
              <CreativeCanvasEditor
                ref={editorRef}
                projectId={projectId}
                tool={tool}
                disabled={productDisabled}
                isNodeVisible={isCreativeCanvasUserNode}
                showSaveState={false}
                isMiniMapOpen={miniMapOpen}
                onToggleMiniMap={() => setMiniMapOpen((open) => !open)}
                onStateChange={handleCanvasStateChange}
                onSaveStateChange={setSave}
                onAgentSessionsChange={handleAgentSessionsChange}
                onPendingTaskCommandBlocked={() =>
                  setNotice(
                    t('creativeStudio.canvas.errors.pendingTaskProtected', {
                      defaultValue:
                        '运行中的生成任务受保护；请等待任务结束后再删除或撤销。',
                    })
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
                      textEditing={
                        node.type === 'text' && editingTextNodeId === node.id
                      }
                      onTextChange={
                        node.type === 'text'
                          ? (text) => handleInlineTextChange(node.id, text)
                          : undefined
                      }
                      onTextEditingComplete={
                        node.type === 'text'
                          ? () => finishInlineTextEditing(node.id)
                          : undefined
                      }
                    />
                  );
                  if (node.type === 'video') {
                    const composeConfig = canvasState
                      ? latestCanvasVideoComposeConfig(canvasState.document, node.id)
                      : null;
                    const composeDraft = canvasState
                      ? canvasVideoComposeDraftFromState(canvasState, node.id)
                      : structuredClone(DEFAULT_CANVAS_VIDEO_COMPOSE_DRAFT);
                    const mode: CanvasVideoComposeMode = canvasState
                      ? canvasVideoComposeMode(canvasState.document, node.id)
                      : {
                          kind: 'unsupported',
                          message: t('creativeStudio.canvas.errors.notLoaded', {
                            defaultValue: '画布尚未完成载入。',
                          }),
                        };
                    const selectedModel = composeDraft.settings.model;
                    const exactModel = selectedModel
                      ? videoModelOptions.find(
                          (option) =>
                            option.providerId === selectedModel.providerId &&
                            option.model === selectedModel.model
                        )
                      : null;
                    const onlyModel =
                      videoModelOptions.length === 1 ? videoModelOptions[0] : null;
                    const composeSettings: CanvasVideoComposeSettings = {
                      ...composeDraft.settings,
                      model: exactModel
                        ? {
                            providerId: exactModel.providerId,
                            model: exactModel.model,
                          }
                        : onlyModel
                          ? {
                              providerId: onlyModel.providerId,
                              model: onlyModel.model,
                            }
                          : null,
                    };
                    const referenceAsset =
                      mode.kind === 'i2v'
                        ? knownAssetsById.get(mode.assetId) ?? null
                        : null;
                    const singleSelected =
                      selected && canvasState?.selection.nodeIds.length === 1;
                    const retrySubmission =
                      videoComposeSubmission?.nodeId === node.id;
                    return (
                      <div className={styles.nodeComposerHost} data-video-composer-host>
                        {nodeView}
                        {singleSelected ? (
                          <CreativeCanvasVideoComposer
                            nodeId={node.id}
                            mode={mode.kind}
                            reference={
                              mode.kind === 'i2v'
                                ? {
                                    name:
                                      referenceAsset?.title ??
                                      t('creativeStudio.canvas.video.connectedImage', {
                                        defaultValue: '已连接图片',
                                      }),
                                    previewUrl:
                                      referenceAsset?.thumbnailUrl ??
                                      referenceAsset?.originalUrl ??
                                      creativeAssetClient.url(mode.assetId),
                                    originalUrl:
                                      referenceAsset?.originalUrl ??
                                      creativeAssetClient.url(mode.assetId),
                                  }
                                : null
                            }
                            initialPrompt={composeDraft.prompt}
                            settings={composeSettings}
                            modelOptions={videoModelOptions}
                            task={canvasVideoComposeTaskSummary(composeConfig)}
                            disabled={
                              productDisabled ||
                              assetImportBusy ||
                              videoComposeBusy ||
                              !videoTaskRuntimeReady ||
                              (!retrySubmission &&
                                videoTaskRuntimeBlocksNew &&
                                composeConfig?.data.status !== 'queued' &&
                                composeConfig?.data.status !== 'running')
                            }
                            error={
                              videoComposeIssue?.nodeId === node.id
                                ? videoComposeIssue.message
                                : mode.kind === 'unsupported'
                                  ? mode.message
                                  : null
                            }
                            retrySubmission={retrySubmission}
                            onPromptChange={(prompt) =>
                              updateVideoComposeDraft(node.id, (current) => ({
                                ...current,
                                prompt,
                              }))
                            }
                            onOpenPromptLibrary={() =>
                              openPromptLibrary()
                            }
                            onModelChange={(model) =>
                              updateVideoComposeDraft(node.id, (current) => ({
                                ...current,
                                settings: { ...current.settings, model },
                              }))
                            }
                            onResolutionChange={(resolution) =>
                              updateVideoComposeDraft(node.id, (current) => ({
                                ...current,
                                settings: { ...current.settings, resolution },
                              }))
                            }
                            onAspectRatioChange={(aspectRatio) =>
                              updateVideoComposeDraft(node.id, (current) => ({
                                ...current,
                                settings: { ...current.settings, aspectRatio },
                              }))
                            }
                            onSecondsChange={(seconds) =>
                              updateVideoComposeDraft(node.id, (current) => ({
                                ...current,
                                settings: { ...current.settings, seconds },
                              }))
                            }
                            onGenerate={(prompt) =>
                              void generateFromCanvasVideo(
                                node.id,
                                prompt,
                                composeSettings
                              )
                            }
                            onRetrySubmission={() =>
                              void retryCanvasVideoComposeSubmission(node.id)
                            }
                            onConfirmSubmission={() =>
                              void confirmCanvasVideoComposeSubmission(node.id)
                            }
                          />
                        ) : null}
                      </div>
                    );
                  }
                  if (node.type === 'audio') {
                    const composeConfig = canvasState
                      ? latestCanvasAudioComposeConfig(canvasState.document, node.id)
                      : null;
                    const composeDraft = canvasState
                      ? canvasAudioComposeDraftFromState(canvasState, node.id)
                      : structuredClone(DEFAULT_CANVAS_AUDIO_COMPOSE_DRAFT);
                    const eligibility = canvasState
                      ? canvasAudioComposeEligibility(canvasState.document, node.id)
                      : {
                          kind: 'unsupported' as const,
                          message: t('creativeStudio.canvas.errors.notLoaded', {
                            defaultValue: '画布尚未完成载入。',
                          }),
                        };
                    const selectedModel = composeDraft.settings.model;
                    const exactModel = selectedModel
                      ? audioModelOptions.find(
                          (option) =>
                            option.providerId === selectedModel.providerId &&
                            option.model === selectedModel.model
                        )
                      : null;
                    const onlyModel =
                      audioModelOptions.length === 1 ? audioModelOptions[0] : null;
                    const resolvedModel = exactModel ?? onlyModel;
                    const composeSettings: CanvasAudioComposeSettings = {
                      ...composeDraft.settings,
                      voice: canvasAudioComposeVoiceAfterModelChange(
                        exactModel ?? null,
                        resolvedModel,
                        composeDraft.settings.voice
                      ),
                      model: resolvedModel
                        ? {
                            providerId: resolvedModel.providerId,
                            model: resolvedModel.model,
                          }
                        : null,
                    };
                    const protocolProfile = canvasAudioComposeProtocolProfile(
                      resolvedModel?.protocol ?? ''
                    );
                    const singleSelected =
                      selected && canvasState?.selection.nodeIds.length === 1;
                    const retrySubmission =
                      audioComposeSubmission?.nodeId === node.id;
                    return (
                      <div
                        className={styles.nodeComposerHost}
                        data-audio-composer-host
                      >
                        {nodeView}
                        {singleSelected ? (
                          <CreativeCanvasAudioComposer
                            nodeId={node.id}
                            initialPrompt={composeDraft.prompt}
                            settings={composeSettings}
                            modelOptions={audioModelOptions}
                            task={canvasAudioComposeTaskSummary(composeConfig)}
                            voiceSupported={protocolProfile.fieldSupport.voice}
                            voiceRequired={protocolProfile.voiceRequired}
                            formatSupported={protocolProfile.fieldSupport.format}
                            maxTextLength={protocolProfile.maxTextLength}
                            disabled={
                              productDisabled ||
                              assetImportBusy ||
                              audioComposeBusy ||
                              !audioTaskRuntimeReady ||
                              eligibility.kind === 'unsupported' ||
                              (!retrySubmission &&
                                audioTaskRuntimeBlocksNew &&
                                composeConfig?.data.status !== 'queued' &&
                                composeConfig?.data.status !== 'running')
                            }
                            error={
                              audioComposeIssue?.nodeId === node.id
                                ? audioComposeIssue.message
                                : eligibility.kind === 'unsupported'
                                  ? eligibility.message
                                  : null
                            }
                            retrySubmission={retrySubmission}
                            onPromptChange={(prompt) =>
                              updateAudioComposeDraft(node.id, (current) => ({
                                ...current,
                                prompt,
                              }))
                            }
                            onOpenPromptLibrary={() =>
                              openPromptLibrary()
                            }
                            onModelChange={(model) => {
                              const nextModel = model
                                ? (audioModelOptions.find(
                                    (option) =>
                                      option.providerId === model.providerId &&
                                      option.model === model.model
                                  ) ?? null)
                                : null;
                              updateAudioComposeDraft(node.id, (current) => ({
                                ...current,
                                settings: {
                                  ...current.settings,
                                  model,
                                  voice: canvasAudioComposeVoiceAfterModelChange(
                                    exactModel ?? null,
                                    nextModel,
                                    current.settings.voice
                                  ),
                                },
                              }));
                            }}
                            onVoiceChange={(voice) =>
                              updateAudioComposeDraft(node.id, (current) => ({
                                ...current,
                                settings: {
                                  ...current.settings,
                                  voice: voice.trim(),
                                },
                              }))
                            }
                            onFormatChange={(format) =>
                              updateAudioComposeDraft(node.id, (current) => ({
                                ...current,
                                settings: { ...current.settings, format },
                              }))
                            }
                            onGenerate={(prompt) =>
                              void generateFromCanvasAudio(
                                node.id,
                                prompt,
                                composeSettings
                              )
                            }
                            onRetrySubmission={() =>
                              void retryCanvasAudioComposeSubmission(node.id)
                            }
                            onConfirmSubmission={() =>
                              void confirmCanvasAudioComposeSubmission(node.id)
                            }
                          />
                        ) : null}
                      </div>
                    );
                  }
                  if (node.type !== 'image') return nodeView;
                  const composeConfig = canvasState
                    ? latestCanvasImageComposeConfig(canvasState.document, node.id)
                    : null;
                  const composeFallback = canvasState
                    ? canvasImageComposeDraftFromState(canvasState, node.id)
                    : {
                        prompt: '',
                        mentions: [],
                        settings: structuredClone(DEFAULT_CANVAS_IMAGE_COMPOSE_SETTINGS),
                      };
                  const composeDraft = composeFallback;
                  const composeMentions = composeDraft.mentions ?? [];
                  const referenceAssetIds = canvasState
                    ? canvasImageReferenceAssetIds(canvasState, node.id)
                    : [];
                  const hasReferenceIntent = referenceAssetIds.length > 0;
                  const referenceResolution = canvasState
                    ? resolveCanvasImageReferences(
                        canvasState,
                        node.id,
                        [...knownAssetsById.values()]
                      )
                    : {
                        targetNodeId: node.id,
                        inboundConnectionCount: 0,
                        references: [],
                        textReferences: [],
                        issues: [],
                      };
                  const baseComposeModelOptions = hasReferenceIntent
                    ? imageComposeModelOptions
                    : imageGenerationModelOptions;
                  const composeModelOptions = baseComposeModelOptions.map((option) => {
                    const policy = imageReferenceInputPolicy(
                      option.protocol,
                      hasReferenceIntent ? 'image_edit' : 'image_generation'
                    );
                    const effectiveLimit = effectiveImageReferenceInputLimit(policy);
                    const referenceCount = referenceAssetIds.length;
                    const incompatible =
                      (effectiveLimit !== null && referenceCount > effectiveLimit) ||
                      (policy.kind === 'unknown' && referenceCount > 1);
                    return incompatible || option.disabled
                      ? { ...option, disabled: true }
                      : option;
                  });
                  const selectedModel = composeDraft.settings.model;
                  const exactModel = selectedModel
                    ? composeModelOptions.find(
                        (option) =>
                          option.providerId === selectedModel.providerId &&
                          option.model === selectedModel.model
                      )
                    : null;
                  const enabledComposeModelOptions = composeModelOptions.filter(
                    (option) => !option.disabled
                  );
                  const onlyModel = enabledComposeModelOptions.length === 1
                    ? enabledComposeModelOptions[0]
                    : null;
                  const resolvedModel = exactModel ?? onlyModel;
                  const referencePolicy = imageReferenceInputPolicy(
                    resolvedModel?.protocol,
                    hasReferenceIntent ? 'image_edit' : 'image_generation'
                  );
                  const effectiveReferenceLimit =
                    effectiveImageReferenceInputLimit(referencePolicy);
                  const promptCompilation = compileCanvasImageReferencePrompt(
                    composeDraft.prompt,
                    composeMentions.map((mention) => ({
                      sourceNodeId: mention.sourceNodeId,
                      start: mention.start,
                      end: mention.end,
                      tokenText: `@${mention.fallbackLabel}`,
                    })),
                    referenceResolution.references,
                    referenceResolution.textReferences
                  );
                  const generationGate = evaluateCanvasImageGenerationGate({
                    resolution: referenceResolution,
                    compilation: promptCompilation,
                    maxInputImages: effectiveReferenceLimit,
                  });
                  const generationBlockerMessage =
                    canvasImageGenerationBlockerMessage(
                      generationGate.blockers[0],
                      t
                  );
                  const composerReferences = [
                    ...canvasImageComposerReferences(referenceResolution.references),
                    ...canvasTextComposerReferences(referenceResolution.textReferences, t),
                    ...(canvasState
                      ? invalidCanvasImageComposerReferences(
                          canvasState,
                          node.id,
                          referenceResolution,
                          knownAssetsById,
                          t
                        )
                      : []),
                  ].sort((left, right) => left.ordinal - right.ordinal);
                  const composeSizePolicy = imageWorkbenchSizePolicyForModel(resolvedModel);
                  const composeSizeOptions = imageWorkbenchSelectableSizeOptions(
                    composeSizePolicy.options
                  );
                  const composeSettings: ImageWorkbenchSettings =
                    normalizeImageWorkbenchSettingsSize(
                      {
                        ...composeDraft.settings,
                        model: resolvedModel
                          ? {
                              providerId: resolvedModel.providerId,
                              model: resolvedModel.model,
                            }
                          : null,
                      },
                      composeSizePolicy
                    );
                  const singleSelected =
                    selected && canvasState?.selection.nodeIds.length === 1;
                  const retrySubmission = imageComposeSubmission?.nodeId === node.id;
                  return (
                    <CreativeCanvasImageToolbar
                      nodeId={node.id}
                      visible={Boolean(singleSelected)}
                      hasImageContent={Boolean(node.data.assetId)}
                      disabled={
                        productDisabled ||
                        assetImportBusy ||
                        imageCropBusy ||
                        imageSplitBusy ||
                        imageMaskBusy ||
                        imageComposeBusy ||
                        imageTaskRuntimeBlocksNew
                      }
                      onInfo={() => {
                        handleRightViewChange('properties');
                        setNotice(
                          t('creativeStudio.canvas.notices.propertiesOpened', {
                            defaultValue: '已在属性面板打开所选节点。',
                          })
                        );
                      }}
                      onDelete={() =>
                        dispatch(
                          canvasCommands.deleteSelection({ nodeIds: [node.id] })
                        )
                      }
                      onUpload={() => openImageNodeUpload(node.id)}
                      onPreview={() => setPreviewImageNode(node)}
                      onCrop={() => void handleOpenImageCrop(node)}
                      onDownload={() => void handleDownloadImage(node)}
                      onMaskEdit={() => void handleOpenImageMaskEdit(node)}
                      onSplit={() => void handleOpenImageSplit(node)}
                    >
                      {nodeView}
                      {singleSelected ? (
                        <CreativeCanvasImageComposer
                          nodeId={node.id}
                          hasImageContent={hasReferenceIntent}
                          initialPrompt={composeDraft.prompt}
                          initialMentions={composeMentions}
                          references={composerReferences}
                          settings={composeSettings}
                          aspectRatioOptions={composeSizeOptions}
                          maxCount={composeSizePolicy.maxCount}
                          modelOptions={composeModelOptions}
                          task={canvasImageComposeTaskSummary(composeConfig)}
                          disabled={
                            productDisabled ||
                            assetImportBusy ||
                            imageCropBusy ||
                            imageSplitBusy ||
                            imageMaskBusy ||
                            imageComposeBusy ||
                            (!retrySubmission &&
                              imageTaskRuntimeBlocksNew &&
                              composeConfig?.data.status !== 'queued' &&
                              composeConfig?.data.status !== 'running')
                          }
                          generateBlocked={!generationGate.allowed}
                          error={
                            imageComposeIssue?.nodeId === node.id
                              ? imageComposeIssue.message
                              : generationBlockerMessage
                          }
                          retrySubmission={retrySubmission}
                          onPromptChange={(change) =>
                            updateImageComposeDraft(
                              node.id,
                              (current) => ({
                                ...current,
                                prompt: change.value,
                                mentions: structuredClone(change.mentions),
                              })
                            )
                          }
                          onReferenceActivate={(sourceNodeId) =>
                            dispatch(canvasCommands.setSelection([sourceNodeId]))
                          }
                          onReferenceDisconnect={(connectionId) =>
                            dispatch(canvasCommands.deleteEdges([connectionId]))
                          }
                          onReferencesDisconnect={(connectionIds) =>
                            dispatch(canvasCommands.deleteEdges(connectionIds))
                          }
                          onOpenPromptLibrary={() =>
                            openPromptLibrary()
                          }
                          onModelChange={(model: ImageWorkbenchModelIdentity | null) =>
                            updateImageComposeDraft(
                              node.id,
                              (current) => {
                                const modelOption = model
                                  ? composeModelOptions.find(
                                      (option) =>
                                        option.providerId === model.providerId &&
                                        option.model === model.model
                                    )
                                  : null;
                                return {
                                  ...current,
                                  settings: normalizeImageWorkbenchSettingsSize(
                                    { ...current.settings, model },
                                    imageWorkbenchSizePolicyForModel(modelOption)
                                  ),
                                };
                              }
                            )
                          }
                          onInterfaceModeChange={(interfaceMode) =>
                            updateImageComposeDraft(
                              node.id,
                              (current) => ({
                                ...current,
                                settings: { ...current.settings, interfaceMode },
                              })
                            )
                          }
                          onQualityChange={(quality) =>
                            updateImageComposeDraft(
                              node.id,
                              (current) => ({
                                ...current,
                                settings: { ...current.settings, quality },
                              })
                            )
                          }
                          onAspectRatioChange={(option: ImageWorkbenchAspectRatioOption) =>
                            updateImageComposeDraft(
                              node.id,
                              (current) => ({
                                ...current,
                                settings: {
                                  ...current.settings,
                                  aspectRatio: option.value,
                                  width: option.width,
                                  height: option.height,
                                },
                              })
                            )
                          }
                          onCountChange={(count) =>
                            updateImageComposeDraft(
                              node.id,
                              (current) => ({
                                ...current,
                                settings: { ...current.settings, count },
                              })
                            )
                          }
                          onGenerate={(prompt, mentions) =>
                            void generateFromCanvasImage(
                              node.id,
                              prompt,
                              mentions,
                              composeSettings
                            )
                          }
                          onRetrySubmission={() =>
                            void retryCanvasImageComposeSubmission(node.id)
                          }
                        />
                      ) : null}
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
                    nodes={state.document.nodes.filter(isCreativeCanvasUserNode)}
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
              <CanvasTaskRuntimeAction
                label={t('creativeStudio.canvas.tasks.image', {
                  defaultValue: '图片任务',
                })}
                snapshot={imageTaskRuntime}
                busy={imageTaskRuntimeActionBusy}
                onCancel={(taskId) => void cancelImageRuntimeTask(taskId)}
                onRetry={(taskId) => void retryImageRuntimeTask(taskId)}
              />
              <CanvasTaskRuntimeAction
                label={t('creativeStudio.canvas.tasks.video', {
                  defaultValue: '视频任务',
                })}
                snapshot={videoTaskRuntime}
                busy={videoTaskRuntimeActionBusy}
                onCancel={(taskId) => void cancelVideoRuntimeTask(taskId)}
                onRetry={(taskId) => void retryVideoRuntimeTask(taskId)}
              />
              <CanvasTaskRuntimeAction
                label={t('creativeStudio.canvas.tasks.audio', {
                  defaultValue: '音频任务',
                })}
                snapshot={audioTaskRuntime}
                busy={audioTaskRuntimeActionBusy}
                onCancel={(taskId) => void cancelAudioRuntimeTask(taskId)}
                onRetry={(taskId) => void retryAudioRuntimeTask(taskId)}
              />
              <SaveRecoveryAction
                save={save}
                busy={recoveryBusy}
                notice={notice}
                requiresAuthoritativeReload={agentOpsReloadRequired}
                onReload={() => void handleReloadRemote()}
                onRetry={() => void handleRetrySave()}
              />
            </>
          ),
          toolbarTrailing: (
            <>
              <ProductToolbarButton
                label={t('creativeStudio.canvas.toolbar.group', {
                  defaultValue: '将所选节点分组',
                })}
                icon={<Group {...iconProps} />}
                disabled={productDisabled || !selection.canGroup}
                onClick={() =>
                  dispatch(
                    canvasCommands.groupNodes({
                      nodeIds: canvasState?.selection.nodeIds,
                      title: t('creativeStudio.canvas.nodes.defaultGroupTitle', {
                        defaultValue: '节点组',
                      }),
                    })
                  )
                }
              />
              <ProductToolbarButton
                label={t('creativeStudio.canvas.toolbar.ungroup', {
                  defaultValue: '取消所选分组',
                })}
                icon={<Ungroup {...iconProps} />}
                disabled={productDisabled || selection.groupIds.length === 0}
                onClick={() => {
                  for (const groupId of selection.groupIds) {
                    dispatch(canvasCommands.ungroup(groupId));
                  }
                }}
              />
              <ProductToolbarButton
                label={canvasState?.selection.nodeIds.length === 0 && canvasState.selection.edgeIds.length > 0
                  ? t('creativeStudio.canvas.connection.deleteSelected', { count: canvasState.selection.edgeIds.length })
                  : t('creativeStudio.canvas.toolbar.deleteSelection', {
                      defaultValue: '删除所选节点或连接',
                    })}
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
                onCopy={handleCopyPrompt}
              />
            ),
            templates: (
              <CreativeCanvasTemplatePanel
                templates={templates}
                runtime={templateRuntime.snapshot}
                loading={templateLoading}
                error={templateError}
                disabled={productDisabled}
                insertingRunId={templateInsertingRunId}
                onRetry={() => {
                  void loadTemplates();
                  void templateRuntime.controller.load().catch(() => undefined);
                }}
                onRun={setTemplateToRun}
                onInsertResults={(run) => void handleInsertTemplateResults(run)}
                onOpenCenter={() => void handleOpenTemplateCenter()}
              />
            ),
          },
          right: {
            assistant: (
              <CreativeCanvasAgentPanel
                ref={agentPanelRef}
                canvasId={projectId}
                hydrated={agentDocumentState !== null}
                sessions={agentDocumentState?.sessions ?? []}
                activeSessionId={agentDocumentState?.activeSessionId ?? null}
                planningContext={agentPlanningContext}
                disabled={productDisabled}
                onPersist={handlePersistAgentSessions}
                onApplyCanvasOps={handleApplyCanvasAgentOps}
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
      {imageTaskRuntimeReady && project.detail ? (
        <CanvasImageTaskRuntimeBridge
          key={`${projectId}:${imageTaskRuntimeEpoch}`}
          ref={imageTaskRuntimeRef}
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
          onSnapshot={setImageTaskRuntime}
          onNotice={setNotice}
        />
      ) : null}
      {videoTaskRuntimeReady && project.detail ? (
        <CanvasVideoTaskRuntimeBridge
          key={`${projectId}:video:${videoTaskRuntimeEpoch}`}
          ref={videoTaskRuntimeRef}
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
          onSnapshot={setVideoTaskRuntime}
          onNotice={setNotice}
        />
      ) : null}
      {audioTaskRuntimeReady && project.detail ? (
        <CanvasAudioTaskRuntimeBridge
          key={`${projectId}:audio:${audioTaskRuntimeEpoch}`}
          ref={audioTaskRuntimeRef}
          projectId={projectId}
          initialDocument={project.detail.document}
          editorRef={editorRef}
          onAsset={(asset) => {
            knownAssetsRef.current = new Map(knownAssetsRef.current).set(
              asset.id,
              asset
            );
            void assets.reload();
          }}
          onSnapshot={setAudioTaskRuntime}
          onNotice={setNotice}
        />
      ) : null}
      <TemplateRunModal
        template={templateToRun}
        runner={templateRunner}
        onClose={() => setTemplateToRun(null)}
        onPickAssets={(variable, selectedAssetIds) =>
          templateAssetPicker.pick({
            acceptedKinds: ['image'],
            initialSelectedIds: selectedAssetIds,
            selectionLimit:
              variable.type === 'image-series' ? variable.maxItems : 1,
            title:
              variable.type === 'image-series'
                ? t('creativeStudio.canvas.templates.pickVariableImages', {
                    defaultValue: '选择变量图片',
                  })
                : t('creativeStudio.canvas.templates.pickVariableReference', {
                    defaultValue: '选择变量参考图',
                  }),
          })
        }
        onPickReferenceAssets={(selectedAssetIds) =>
          templateAssetPicker.pick({
            acceptedKinds: ['image'],
            initialSelectedIds: selectedAssetIds,
            selectionLimit: 100,
            title: t('creativeStudio.canvas.templates.pickReferences', {
              defaultValue: '选择模板参考图',
            }),
          })
        }
        onUploadReferenceImages={async (files, selectedAssetIds) => {
          const uploaded = await Promise.all(
            files.map((file) =>
              creativeAssetClient.upload(file, {
                title: file.name,
                tags: ['template-reference'],
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
      {templateAssetPicker.dialog}
      {previewImageNode ? (
        <CreativeImagePreviewDialog
          key={`${projectId}:${previewImageNode.id}:${previewImageNode.data.assetId}`}
          node={previewImageNode}
          resolveAsset={resolveCanvasImagePreviewAsset}
          onClose={() => setPreviewImageNode(null)}
        />
      ) : null}
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
      <input
        ref={imageNodeUploadInputRef}
        hidden
        type="file"
        accept="image/*"
        aria-label={t('creativeStudio.canvas.upload.inputLabel', {
          defaultValue: '上传图片到所选节点',
        })}
        onChange={(event) => void handleImageNodeUploadChange(event)}
      />
      <Modal
        title={t('creativeStudio.canvas.panorama.dialogTitle', {
          defaultValue: '选择 2:1 图片的节点类型',
        })}
        visible={pendingPanoramaChoice !== null}
        closable={false}
        maskClosable={false}
        escToExit={false}
        footer={
          <div className={styles.panoramaActions}>
            <Button onClick={() => resolvePendingPanoramaChoice(false)}>
              {t('creativeStudio.canvas.panorama.asImage', {
                defaultValue: '作为普通图片',
              })}
            </Button>
            <Button
              type="primary"
              onClick={() => resolvePendingPanoramaChoice(true)}
            >
              {t('creativeStudio.canvas.panorama.asPanorama', {
                defaultValue: '作为全景图',
              })}
            </Button>
          </div>
        }
      >
        <p className={styles.panoramaDescription}>
          {t('creativeStudio.canvas.panorama.dialogDescription', {
            defaultValue:
              '图片已经真实上传并保存在素材库中。检测到宽高比接近 2:1，请确认它应作为普通图片还是等距柱状全景图插入当前画布。',
          })}
        </p>
      </Modal>
    </main>
  );
};

export default CreativeCanvasProductRoute;
