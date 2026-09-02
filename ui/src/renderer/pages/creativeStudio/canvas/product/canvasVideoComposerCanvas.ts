/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import type { CreativeAsset } from '../../assets';
import type {
  CreativeCanvasConnection,
  CreativeCanvasNode,
  CreativeGenerationStatus,
  CreativeProjectDocument,
  CreativeSize,
} from '../../domain';
import type {
  CreativeModelCatalogSnapshot,
  CreativeModelSelectionRef,
} from '../../models';
import {
  isCanvasNodeTaskOwner,
  type CreativeTask,
  type CreativeTaskReference,
} from '../../tasks';
import {
  prepareVideoWorkbenchRun,
  workbenchResumeRequestsFromDocument,
  type CreativeWorkbenchReferences,
  type CreativeWorkbenchResumeRequest,
  type PreparedCreativeWorkbenchRun,
  type VideoWorkbenchOperation,
} from '../../workbenches/runtime';
import { validateCanvasConnection, type CanvasState } from '../core';
import {
  canvasTaskResultPosition,
  nextCanvasImageTaskPosition,
} from './imageTaskCanvasLayout';
import {
  createCreativeCanvasProductNode,
  CREATIVE_CANVAS_PRODUCT_NODE_SIZES,
} from './nodeFactory';
import { creativeStudioProductText } from './i18n';

export const CREATIVE_VIDEO_COMPOSE_OPERATION = 'video-node-compose';

type VideoNode = Extract<CreativeCanvasNode, { type: 'video' }>;
type ConfigNode = Extract<CreativeCanvasNode, { type: 'config' }>;

export interface CanvasVideoComposeSettings {
  model: CreativeModelSelectionRef | null;
  resolution: string;
  aspectRatio: string;
  seconds: number;
}

export interface CanvasVideoComposeDraft {
  prompt: string;
  settings: CanvasVideoComposeSettings;
}

export interface CanvasVideoComposeTaskSummary {
  state: CreativeGenerationStatus;
  pendingCount: number;
  message?: string;
}

export interface PreparedCanvasVideoCompose {
  configNode: ConfigNode;
  connection: Omit<CreativeCanvasConnection, 'id'>;
  plan: PreparedCreativeWorkbenchRun;
}

export type CanvasVideoComposeMode =
  | { kind: 't2v' }
  | { kind: 'i2v'; assetId: string }
  | { kind: 'unsupported'; message: string };

export const DEFAULT_CANVAS_VIDEO_COMPOSE_SETTINGS: CanvasVideoComposeSettings = {
  model: null,
  resolution: '1080p',
  aspectRatio: '16:9',
  seconds: 5,
};

export const DEFAULT_CANVAS_VIDEO_COMPOSE_DRAFT: CanvasVideoComposeDraft = {
  prompt: '',
  settings: structuredClone(DEFAULT_CANVAS_VIDEO_COMPOSE_SETTINGS),
};

const durationSeconds = (value: unknown): number =>
  typeof value === 'number' &&
  Number.isSafeInteger(value) &&
  value >= 1 &&
  value <= 3_600
    ? value
    : DEFAULT_CANVAS_VIDEO_COMPOSE_SETTINGS.seconds;

/** Exact dimensions supported by the current video adapter controls. */
export function canvasVideoComposeDimensions(
  resolution: string,
  aspectRatio: string
): { width: number; height: number } {
  const shortEdge =
    resolution === '720p' ? 720 : resolution === '1080p' ? 1080 : null;
  if (shortEdge === null) {
  throw new Error(
    creativeStudioProductText(
      'creativeStudio.canvas.errors.video.resolutionUnsupported',
      '当前视频创作不支持分辨率 {{resolution}}。',
      { resolution }
    )
  );
  }
  if (aspectRatio === '16:9') {
    return { width: Math.round((shortEdge * 16) / 9), height: shortEdge };
  }
  if (aspectRatio === '9:16') {
    return { width: shortEdge, height: Math.round((shortEdge * 16) / 9) };
  }
  if (aspectRatio === '1:1') {
    return { width: shortEdge, height: shortEdge };
  }
  throw new Error(
    creativeStudioProductText(
      'creativeStudio.canvas.errors.video.aspectRatioUnsupported',
      '当前视频创作不支持画幅 {{aspectRatio}}。',
      { aspectRatio }
    )
  );
}

const modelSelection = (
  providerId: string | null,
  model: string | null
): CreativeModelSelectionRef | null =>
  providerId && model
    ? {
        providerId: providerId as CreativeModelSelectionRef['providerId'],
        model,
      }
    : null;

export function canvasVideoComposeSettings(
  config: ConfigNode | null
): CanvasVideoComposeSettings {
  if (!config) return structuredClone(DEFAULT_CANVAS_VIDEO_COMPOSE_SETTINGS);
  const width = config.data.parameters.width;
  const height = config.data.parameters.height;
  const dimensions = (['720p', '1080p'] as const).flatMap((resolution) =>
    (['16:9', '9:16', '1:1'] as const).map((aspectRatio) => ({
      resolution,
      aspectRatio,
      ...canvasVideoComposeDimensions(resolution, aspectRatio),
    }))
  );
  const selected = dimensions.find(
    (candidate) => candidate.width === width && candidate.height === height
  );
  return {
    model: modelSelection(config.data.providerId, config.data.model),
    resolution:
      selected?.resolution ?? DEFAULT_CANVAS_VIDEO_COMPOSE_SETTINGS.resolution,
    aspectRatio:
      selected?.aspectRatio ?? DEFAULT_CANVAS_VIDEO_COMPOSE_SETTINGS.aspectRatio,
    seconds: durationSeconds(config.data.parameters.seconds),
  };
}

/** Restore the node-owned draft, falling back to the latest submitted config. */
export function canvasVideoComposeDraftFromState(
  state: CanvasState,
  nodeId: string
): CanvasVideoComposeDraft {
  const source = state.document.nodes.find(
    (node): node is VideoNode => node.id === nodeId && node.type === 'video'
  );
  const persisted = source?.data.composer;
  if (persisted) {
    return {
      prompt: persisted.prompt,
      settings: {
        model: persisted.model
          ? {
              providerId:
                persisted.model.providerId as CreativeModelSelectionRef['providerId'],
              model: persisted.model.model,
            }
          : null,
        resolution: persisted.resolution,
        aspectRatio: persisted.aspectRatio,
        seconds: persisted.seconds,
      },
    };
  }
  const config = latestCanvasVideoComposeConfig(state.document, nodeId);
  return {
    prompt: config?.data.prompt ?? DEFAULT_CANVAS_VIDEO_COMPOSE_DRAFT.prompt,
    settings: config
      ? canvasVideoComposeSettings(config)
      : structuredClone(DEFAULT_CANVAS_VIDEO_COMPOSE_SETTINGS),
  };
}

/** Replace the complete durable composer draft without changing node identity. */
export function withCanvasVideoComposeDraft(
  node: VideoNode,
  draft: CanvasVideoComposeDraft
): VideoNode {
  return {
    ...node,
    data: {
      ...node.data,
      composer: {
        prompt: draft.prompt,
        model: draft.settings.model ? { ...draft.settings.model } : null,
        resolution: draft.settings.resolution,
        aspectRatio: draft.settings.aspectRatio,
        seconds: draft.settings.seconds,
      },
    },
  };
}

/** Reference changes can switch t2v/i2v, so never retain a task-specific model. */
export function clearCanvasVideoComposeDraftModel(node: VideoNode): VideoNode {
  if (!node.data.composer?.model) return node;
  return {
    ...node,
    data: {
      ...node.data,
      composer: {
        ...node.data.composer,
        model: null,
      },
    },
  };
}

export function canvasVideoComposeTaskSummary(
  config: ConfigNode | null
): CanvasVideoComposeTaskSummary {
  if (!config) return { state: 'idle', pendingCount: 0 };
  const pending = config.data.status === 'queued' || config.data.status === 'running';
  return {
    state: config.data.status,
    pendingCount: pending ? 1 : 0,
    message: config.data.errorMessage ?? undefined,
  };
}

/** Derive the only honest first-slice mode from the selected empty video and
 * direct incoming real media nodes. Text/config edges do not invent inputs. */
export function canvasVideoComposeMode(
  document: Pick<CreativeProjectDocument, 'nodes' | 'connections'>,
  sourceNodeId: string
): CanvasVideoComposeMode {
  const source = document.nodes.find(
    (node): node is VideoNode => node.id === sourceNodeId && node.type === 'video'
  );
  if (!source) {
    return {
      kind: 'unsupported',
      message: creativeStudioProductText(
        'creativeStudio.canvas.errors.video.nodeMissing',
        '视频节点已不存在。'
      ),
    };
  }
  if (source.data.assetId) {
    return {
      kind: 'unsupported',
      message: creativeStudioProductText(
        'creativeStudio.canvas.errors.video.v2vUnsupported',
        '当前后端尚不支持 V2V。'
      ),
    };
  }
  const incomingIds = new Set(
    document.connections
      .filter((connection) => connection.targetNodeId === sourceNodeId)
      .map((connection) => connection.sourceNodeId)
  );
  const incoming = document.nodes.filter((node) => incomingIds.has(node.id));
  if (
    incoming.some(
      (node) =>
        (node.type === 'video' || node.type === 'audio') && node.data.assetId
    )
  ) {
    return {
      kind: 'unsupported',
      message: creativeStudioProductText(
        'creativeStudio.canvas.errors.video.mediaReferencesUnsupported',
        '当前视频创作不支持视频或音频参考。'
      ),
    };
  }
  const imageAssetIds = [
    ...new Set(
      incoming.flatMap((node) =>
        (node.type === 'image' || node.type === 'panorama') && node.data.assetId
          ? [node.data.assetId]
          : []
      )
    ),
  ];
  if (imageAssetIds.length > 1) {
    return {
      kind: 'unsupported',
      message: creativeStudioProductText(
        'creativeStudio.canvas.errors.video.singleImageReferenceRequired',
        '当前 I2V 只支持一张直接连接的真实图片。'
      ),
    };
  }
  return imageAssetIds[0]
    ? { kind: 'i2v', assetId: imageAssetIds[0] }
    : { kind: 't2v' };
}

export function isCanvasVideoComposeConfig(
  node: CreativeCanvasNode | undefined
): node is ConfigNode {
  return Boolean(
    node?.type === 'config' &&
      node.data.task === 'video_generation' &&
      (node.data.capability === 't2v' ||
        node.data.capability === 'i2v') &&
      node.data.operation?.kind === CREATIVE_VIDEO_COMPOSE_OPERATION &&
      typeof node.data.operation.sourceNodeId === 'string' &&
      Boolean(node.data.operation.sourceNodeId.trim()) &&
      node.data.operation.sourceAssetId === null
  );
}

export function canvasVideoComposeSourceNodeId(node: ConfigNode): string {
  const value = node.data.operation?.sourceNodeId;
  if (typeof value !== 'string' || !value.trim()) {
    throw new Error(
      creativeStudioProductText(
        'creativeStudio.canvas.errors.video.missingSourceNodeId',
        '视频创作配置缺少 sourceNodeId。'
      )
    );
  }
  return value;
}

/** Read the local source identity without leaking it into provider parameters. */
export function canvasVideoComposeSourceAssetId(
  node: ConfigNode
): string | null {
  const value = node.data.operation?.sourceAssetId;
  if (value === null) return null;
  if (typeof value !== 'string' || !value.trim()) {
    throw new Error(
      creativeStudioProductText(
        'creativeStudio.canvas.errors.video.invalidSourceAssetId',
        '视频创作配置包含无效的 sourceAssetId。'
      )
    );
  }
  return value;
}

export function latestCanvasVideoComposeConfig(
  document: Pick<CreativeProjectDocument, 'nodes'>,
  sourceNodeId: string
): ConfigNode | null {
  const matches = document.nodes.filter(
    (node): node is ConfigNode =>
      isCanvasVideoComposeConfig(node) &&
      canvasVideoComposeSourceNodeId(node) === sourceNodeId
  );
  return matches.at(-1) ?? null;
}

export function prepareCanvasVideoCompose(input: {
  projectId: string;
  state: CanvasState;
  viewportSize: CreativeSize;
  sourceNode: VideoNode;
  sourceAsset: CreativeAsset | null;
  catalog: CreativeModelCatalogSnapshot;
  model: CreativeModelSelectionRef;
  operation: VideoWorkbenchOperation;
  references: CreativeWorkbenchReferences;
  prompt: string;
  settings: Omit<CanvasVideoComposeSettings, 'model'>;
}): PreparedCanvasVideoCompose {
  const sourceAssetId = input.sourceNode.data.assetId;
  if (sourceAssetId !== null || input.sourceAsset !== null) {
    throw new Error(
      creativeStudioProductText(
        'creativeStudio.canvas.errors.video.emptyNodeOnly',
        '当前视频创作只能使用空视频源节点。'
      )
    );
  }
  if (input.operation.capability === 'v2v') {
    throw new Error(
      creativeStudioProductText(
        'creativeStudio.canvas.errors.video.v2vUnsupportedLowercase',
        '当前视频创作后端尚不支持 v2v。'
      )
    );
  }
  if (input.operation.capability === 'i2v') {
    const asset = input.references.assets[0];
    const binding = input.references.bindings[0];
    if (
      input.references.assets.length !== 1 ||
      input.references.bindings.length !== 1 ||
      asset?.kind !== 'image' ||
      binding?.kind !== 'image' ||
      binding.role !== 'reference' ||
      binding.assetId !== asset.id
    ) {
      throw new Error(
        creativeStudioProductText(
          'creativeStudio.canvas.errors.video.referenceContract',
          '当前 i2v 只支持一张 role=reference 的真实图片引用。'
        )
      );
    }
  }

  const dimensions = canvasVideoComposeDimensions(
    input.settings.resolution,
    input.settings.aspectRatio
  );

  const configPosition = nextCanvasImageTaskPosition(
    input.state.document.nodes,
    input.sourceNode,
    CREATIVE_CANVAS_PRODUCT_NODE_SIZES.config
  );
  const base = createCreativeCanvasProductNode(
    'config',
    input.state,
    input.viewportSize,
    { position: configPosition, locked: true }
  );
  const plan = prepareVideoWorkbenchRun({
    catalog: input.catalog,
    canvasId: input.projectId,
    nodeId: base.id,
    model: input.model,
    references: input.references,
    operation: input.operation,
    prompt: input.prompt,
    seconds: input.settings.seconds,
    width: dimensions.width,
    height: dimensions.height,
    taskCount: 1,
  });
  const configNode: ConfigNode = {
    ...base,
    data: {
      ...base.data,
      operation: {
        kind: CREATIVE_VIDEO_COMPOSE_OPERATION,
        sourceNodeId: input.sourceNode.id,
        sourceAssetId: null,
      },
      task: plan.input.task,
      capability: plan.input.capability,
      providerId: plan.model.providerId,
      model: plan.model.model,
      prompt: input.prompt.trim(),
      parameters: structuredClone(plan.input.parameters),
      inputAssetIds: plan.input.inputs.map((reference) => reference.assetId),
      taskId: plan.input.idempotencyKey,
      resultAssetIds: [],
      status: 'queued',
      errorMessage: null,
    },
  };
  const connection = {
    sourceNodeId: input.sourceNode.id,
    targetNodeId: configNode.id,
    sourceHandle: 'source',
    targetHandle: 'target',
  };
  const validation = validateCanvasConnection(
    {
      ...input.state.document,
      nodes: [...input.state.document.nodes, configNode],
    },
    connection
  );
  if (!validation.ok) {
    throw new Error(
      creativeStudioProductText(
        'creativeStudio.canvas.errors.video.connectConfigFailed',
        '无法保存视频创作任务关联：{{code}}。',
        { code: validation.code }
      )
    );
  }
  return { configNode, connection, plan };
}

export function canvasVideoComposeResumeRequests(
  document: CreativeProjectDocument
): CreativeWorkbenchResumeRequest[] {
  const owners = new Set(
    document.nodes.filter(isCanvasVideoComposeConfig).map((node) => node.id)
  );
  return workbenchResumeRequestsFromDocument(document).filter(
    (request) =>
      request.reference.owner.kind === 'canvas_node' &&
      owners.has(request.reference.owner.nodeId)
  );
}

export function canvasVideoComposeConfigForReference(
  document: Pick<CreativeProjectDocument, 'projectId' | 'nodes'>,
  reference: CreativeTaskReference
): ConfigNode {
  if (
    !isCanvasNodeTaskOwner(reference.owner) ||
    reference.owner.canvasId !== document.projectId
  ) {
      throw new Error(
        creativeStudioProductText(
          'creativeStudio.canvas.errors.video.wrongCanvas',
          '视频创作任务不属于当前画布。'
        )
      );
  }
  const owner = reference.owner;
  const node = document.nodes.find((candidate) => candidate.id === owner.nodeId);
  if (!isCanvasVideoComposeConfig(node)) {
    throw new Error(
      creativeStudioProductText(
        'creativeStudio.canvas.errors.video.missingConfig',
        '视频创作任务缺少必要任务记录。'
      )
    );
  }
  if (
    node.data.taskId !== reference.taskId ||
    node.data.providerId !== reference.providerId ||
    node.data.model !== reference.model ||
    node.data.task !== reference.task ||
    node.data.capability !== reference.capability
  ) {
    throw new Error(
      creativeStudioProductText(
        'creativeStudio.canvas.errors.video.identityMismatch',
        '视频创作任务与其任务记录身份不一致。'
      )
    );
  }
  return node;
}

export function canvasVideoComposeConfigFromTask(
  document: Pick<CreativeProjectDocument, 'projectId' | 'nodes'>,
  task: CreativeTask
): ConfigNode {
  return canvasVideoComposeConfigForReference(document, {
    taskId: task.taskId,
    owner: task.owner,
    providerId: task.providerId,
    model: task.model,
    task: task.task,
    capability: task.capability,
  });
}

export function reconcileCanvasVideoComposeConfig(
  node: ConfigNode,
  task: CreativeTask
): ConfigNode {
  const terminal =
    task.status === 'succeeded' ||
    task.status === 'failed' ||
    task.status === 'canceled';
  return {
    ...node,
    locked: !terminal,
    data: {
      ...node.data,
      taskId: task.taskId,
      resultAssetIds:
        task.status === 'succeeded' ? [...task.resultAssetIds] : [],
      status: task.status,
      errorMessage:
        task.status === 'failed'
          ? (task.error?.message ??
            creativeStudioProductText(
              'creativeStudio.canvas.errors.video.failed',
              '视频创作失败。'
            ))
          : task.status === 'canceled'
            ? creativeStudioProductText(
                'creativeStudio.canvas.errors.video.cancelled',
                '视频创作已取消。'
              )
            : null,
    },
  };
}

export function canvasVideoComposeResultPosition(
  nodes: readonly CreativeCanvasNode[],
  config: ConfigNode
): { x: number; y: number } {
  return canvasTaskResultPosition(
    nodes,
    config,
    CREATIVE_CANVAS_PRODUCT_NODE_SIZES.video
  );
}
