/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import type { CreativeAsset } from '../../assets';
import type {
  CreativeCanvasConnection,
  CreativeCanvasNode,
  CreativeProjectDocument,
  CreativeSize,
} from '../../domain';
import type {
  CreativeModelCatalogSnapshot,
  CreativeModelOption,
  CreativeModelSelectionRef,
} from '../../models';
import {
  isCanvasNodeTaskOwner,
  type CreativeTask,
  type CreativeTaskReference,
} from '../../tasks';
import type {
  ImageWorkbenchInterfaceMode,
  ImageWorkbenchQuality,
  ImageWorkbenchSettings,
  ImageWorkbenchTaskSummary,
} from '../../workbenches/image';
import {
  prepareImageWorkbenchRun,
  workbenchResumeRequestsFromDocument,
  type CreativeWorkbenchResumeRequest,
  type PreparedCreativeWorkbenchRun,
} from '../../workbenches/runtime';
import { validateCanvasConnection, type CanvasState } from '../core';
import { nextCanvasImageTaskPosition } from './imageTaskCanvasLayout';
import {
  createCreativeCanvasProductNode,
  CREATIVE_CANVAS_PRODUCT_NODE_SIZES,
} from './nodeFactory';

export const CREATIVE_IMAGE_COMPOSE_OPERATION = 'image-node-compose';

type ImageNode = Extract<CreativeCanvasNode, { type: 'image' }>;
type ConfigNode = Extract<CreativeCanvasNode, { type: 'config' }>;

export interface PreparedCanvasImageCompose {
  configNode: ConfigNode;
  connection: Omit<CreativeCanvasConnection, 'id'>;
  plan: PreparedCreativeWorkbenchRun;
}

export const DEFAULT_CANVAS_IMAGE_COMPOSE_SETTINGS: ImageWorkbenchSettings = {
  model: null,
  interfaceMode: 'images',
  quality: 'auto',
  width: 1024,
  height: 1024,
  aspectRatio: '1:1',
  count: 1,
};

const interfaceMode = (value: unknown): ImageWorkbenchInterfaceMode =>
  value === 'responses' ? 'responses' : 'images';

const quality = (value: unknown): ImageWorkbenchQuality =>
  value === 'high' || value === 'medium' || value === 'low' ? value : 'auto';

const dimension = (value: unknown): number | null =>
  typeof value === 'number' && Number.isInteger(value) && value > 0 && value <= 8192
    ? value
    : null;

const count = (value: unknown): number =>
  typeof value === 'number' && Number.isInteger(value) && value >= 1 && value <= 10
    ? value
    : 1;

export function canvasImageComposeSettings(
  config: ConfigNode | null
): ImageWorkbenchSettings {
  if (!config) return structuredClone(DEFAULT_CANVAS_IMAGE_COMPOSE_SETTINGS);
  const aspect = config.data.parameters.aspect;
  return {
    model:
      config.data.providerId && config.data.model
        ? { providerId: config.data.providerId, model: config.data.model }
        : null,
    interfaceMode: interfaceMode(config.data.parameters.interface_mode),
    quality: quality(config.data.parameters.quality),
    width: dimension(config.data.parameters.width),
    height: dimension(config.data.parameters.height),
    aspectRatio: typeof aspect === 'string' && aspect.trim() ? aspect : 'auto',
    count: count(config.data.parameters.count),
  };
}

export function canvasImageComposeTaskSummary(
  config: ConfigNode | null
): ImageWorkbenchTaskSummary {
  if (!config) return { state: 'idle', pendingCount: 0 };
  const pending = config.data.status === 'queued' || config.data.status === 'running';
  return {
    state: config.data.status,
    pendingCount: pending ? 1 : 0,
    message: config.data.errorMessage ?? undefined,
  };
}

export function isCanvasImageComposeConfig(
  node: CreativeCanvasNode | undefined
): node is ConfigNode {
  return Boolean(
    node?.type === 'config' &&
    ((node.data.task === 'image_edit' && node.data.capability === 'i2i') ||
      (node.data.task === 'image_generation' && node.data.capability === 't2i')) &&
    node.data.parameters.canvasOperation === CREATIVE_IMAGE_COMPOSE_OPERATION
  );
}

export function canvasImageComposeSourceNodeId(node: ConfigNode): string {
  const value = node.data.parameters.sourceNodeId;
  if (typeof value !== 'string' || !value.trim()) {
    throw new Error('图片创作配置缺少 sourceNodeId。');
  }
  return value;
}

export function latestCanvasImageComposeConfig(
  document: Pick<CreativeProjectDocument, 'nodes'>,
  sourceNodeId: string
): ConfigNode | null {
  const matches = document.nodes.filter(
    (node): node is ConfigNode =>
      isCanvasImageComposeConfig(node) &&
      node.data.parameters.sourceNodeId === sourceNodeId
  );
  return matches.at(-1) ?? null;
}

export function preferredCanvasImageComposeModel(
  options: readonly CreativeModelOption[],
  previous: CreativeModelSelectionRef | null,
  source: CreativeAsset | null
): CreativeModelSelectionRef | null {
  const match = (candidate: CreativeModelSelectionRef | null) =>
    candidate
      ? (options.find(
          (option) =>
            option.providerId === candidate.providerId &&
            option.model === candidate.model
        ) ?? null)
      : null;
  const retained = match(previous);
  if (retained) return { providerId: retained.providerId, model: retained.model };
  const origin =
    source?.origin?.providerId && source.origin.model
      ? match({
          providerId:
            source.origin.providerId as CreativeModelSelectionRef['providerId'],
          model: source.origin.model,
        })
      : null;
  if (origin) return { providerId: origin.providerId, model: origin.model };
  const only = options.length === 1 ? options[0] : null;
  return only ? { providerId: only.providerId, model: only.model } : null;
}

export function prepareCanvasImageCompose(input: {
  projectId: string;
  state: CanvasState;
  viewportSize: CreativeSize;
  sourceNode: ImageNode;
  sourceAsset: CreativeAsset | null;
  catalog: CreativeModelCatalogSnapshot;
  model: CreativeModelSelectionRef;
  prompt: string;
  settings: Omit<ImageWorkbenchSettings, 'model'>;
}): PreparedCanvasImageCompose {
  const sourceAssetId = input.sourceNode.data.assetId;
  if (
    (sourceAssetId === null && input.sourceAsset !== null) ||
    (sourceAssetId !== null &&
      (input.sourceAsset?.kind !== 'image' || input.sourceAsset.id !== sourceAssetId))
  ) {
    throw new Error('图片创作源节点与真实图片素材不一致。');
  }

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
  const plan = prepareImageWorkbenchRun({
    catalog: input.catalog,
    projectId: input.projectId,
    nodeId: base.id,
    model: input.model,
    references: input.sourceAsset
      ? {
          assets: [input.sourceAsset],
          bindings: [
            {
              assetId: input.sourceAsset.id,
              kind: 'image',
              role: 'reference',
            },
          ],
        }
      : { assets: [], bindings: [] },
    operation: input.sourceAsset
      ? { task: 'image_edit', capability: 'i2i' }
      : { task: 'image_generation', capability: 't2i' },
    prompt: input.prompt,
    interfaceMode: input.settings.interfaceMode,
    quality: input.settings.quality,
    width: input.settings.width,
    height: input.settings.height,
    aspectRatio: input.settings.aspectRatio,
    count: input.settings.count,
    extraParameters: {
      canvasOperation: CREATIVE_IMAGE_COMPOSE_OPERATION,
      sourceNodeId: input.sourceNode.id,
      sourceAssetId: input.sourceAsset?.id ?? null,
    },
  });
  const configNode: ConfigNode = {
    ...base,
    data: {
      ...base.data,
      task: plan.input.task,
      capability: plan.input.capability,
      providerId: plan.model.providerId,
      model: plan.model.model,
      prompt: input.prompt.trim(),
      parameters: structuredClone(plan.input.parameters),
      inputAssetIds: input.sourceAsset ? [input.sourceAsset.id] : [],
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
    throw new Error(`无法连接图片创作配置节点：${validation.code}。`);
  }
  return { configNode, connection, plan };
}

export function canvasImageComposeResumeRequests(
  document: CreativeProjectDocument
): CreativeWorkbenchResumeRequest[] {
  const owners = new Set(
    document.nodes.filter(isCanvasImageComposeConfig).map((node) => node.id)
  );
  return workbenchResumeRequestsFromDocument(document).filter(
    (request) =>
      request.reference.owner.kind === 'canvas_node' &&
      owners.has(request.reference.owner.nodeId)
  );
}

export function canvasImageComposeConfigForReference(
  document: Pick<CreativeProjectDocument, 'projectId' | 'nodes'>,
  reference: CreativeTaskReference
): ConfigNode {
  if (
    !isCanvasNodeTaskOwner(reference.owner) ||
    reference.owner.projectId !== document.projectId
  ) {
    throw new Error('图片创作任务不属于当前画布项目。');
  }
  const owner = reference.owner;
  const node = document.nodes.find(
    (candidate) => candidate.id === owner.nodeId
  );
  if (!isCanvasImageComposeConfig(node)) {
    throw new Error('图片创作任务缺少 canonical 配置节点。');
  }
  if (
    node.data.taskId !== reference.taskId ||
    node.data.providerId !== reference.providerId ||
    node.data.model !== reference.model ||
    node.data.task !== reference.task ||
    node.data.capability !== reference.capability
  ) {
    throw new Error('图片创作任务与配置节点身份不一致。');
  }
  return node;
}

export function canvasImageComposeConfigFromTask(
  document: Pick<CreativeProjectDocument, 'projectId' | 'nodes'>,
  task: CreativeTask
): ConfigNode {
  return canvasImageComposeConfigForReference(document, {
    taskId: task.taskId,
    owner: task.owner,
    providerId: task.providerId,
    model: task.model,
    task: task.task,
    capability: task.capability,
  });
}

export function reconcileCanvasImageComposeConfig(
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
          ? (task.error?.message ?? '图片创作失败。')
          : task.status === 'canceled'
            ? '图片创作已取消。'
            : null,
    },
  };
}

export function canvasImageComposeResultPosition(
  nodes: readonly CreativeCanvasNode[],
  config: ConfigNode
): { x: number; y: number } {
  return nextCanvasImageTaskPosition(
    nodes,
    config,
    CREATIVE_CANVAS_PRODUCT_NODE_SIZES.image
  );
}
