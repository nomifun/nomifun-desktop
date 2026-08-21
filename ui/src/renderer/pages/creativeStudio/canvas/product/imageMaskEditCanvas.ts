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
import {
  prepareImageWorkbenchRun,
  workbenchResumeRequestsFromDocument,
  type CreativeWorkbenchResumeRequest,
  type PreparedCreativeWorkbenchRun,
} from '../../workbenches/runtime';
import { validateCanvasConnection, type CanvasState } from '../core';
import { creativeImageMaskEditPrompt } from '../imageTools';
import {
  createCreativeCanvasProductNode,
  CREATIVE_CANVAS_PRODUCT_NODE_SIZES,
} from './nodeFactory';
import { nextCanvasImageTaskPosition } from './imageTaskCanvasLayout';

export const CREATIVE_IMAGE_MASK_EDIT_OPERATION = 'image-mask-edit';

type ImageNode = Extract<CreativeCanvasNode, { type: 'image' }>;
type ConfigNode = Extract<CreativeCanvasNode, { type: 'config' }>;

export interface PreparedCanvasImageMaskEdit {
  configNode: ConfigNode;
  connection: Omit<CreativeCanvasConnection, 'id'>;
  plan: PreparedCreativeWorkbenchRun;
}

export function isCanvasImageMaskEditConfig(
  node: CreativeCanvasNode | undefined
): node is ConfigNode {
  return Boolean(
    node?.type === 'config' &&
    node.data.task === 'image_edit' &&
    node.data.capability === 'i2i' &&
    node.data.parameters.canvasOperation === CREATIVE_IMAGE_MASK_EDIT_OPERATION
  );
}

export function preferredCanvasImageMaskEditModel(
  options: readonly CreativeModelOption[],
  previous: CreativeModelSelectionRef | null,
  source: CreativeAsset
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
  if (retained)
    return { providerId: retained.providerId, model: retained.model };
  const origin =
    source.origin?.providerId && source.origin.model
      ? match({
          providerId: source.origin
            .providerId as CreativeModelSelectionRef['providerId'],
          model: source.origin.model,
        })
      : null;
  if (origin) return { providerId: origin.providerId, model: origin.model };
  const only = options.length === 1 ? options[0] : null;
  return only ? { providerId: only.providerId, model: only.model } : null;
}

export function prepareCanvasImageMaskEdit(input: {
  projectId: string;
  state: CanvasState;
  viewportSize: CreativeSize;
  sourceNode: ImageNode;
  sourceAsset: CreativeAsset;
  markedReference: CreativeAsset;
  referenceDimensions: CreativeSize;
  catalog: CreativeModelCatalogSnapshot;
  model: CreativeModelSelectionRef;
  userPrompt: string;
}): PreparedCanvasImageMaskEdit {
  if (
    input.sourceAsset.kind !== 'image' ||
    input.sourceNode.data.assetId !== input.sourceAsset.id
  ) {
    throw new Error('局部编辑源节点与真实图片素材不一致。');
  }
  if (
    input.markedReference.kind !== 'image' ||
    input.markedReference.inLibrary ||
    !input.markedReference.tags.some((tag) =>
      tag.startsWith('canvas-mask-edit-operation:')
    )
  ) {
    throw new Error('局部编辑缺少已验证的隐藏标记参考图。');
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
  const prompt = creativeImageMaskEditPrompt(input.userPrompt);
  const dimensionsAreSupported =
    Number.isSafeInteger(input.referenceDimensions.width) &&
    Number.isSafeInteger(input.referenceDimensions.height) &&
    input.referenceDimensions.width > 0 &&
    input.referenceDimensions.height > 0 &&
    input.referenceDimensions.width <= 8_192 &&
    input.referenceDimensions.height <= 8_192;
  const plan = prepareImageWorkbenchRun({
    catalog: input.catalog,
    projectId: input.projectId,
    nodeId: base.id,
    model: input.model,
    references: {
      assets: [input.markedReference],
      bindings: [
        {
          assetId: input.markedReference.id,
          kind: 'image',
          role: 'reference',
        },
      ],
    },
    operation: { task: 'image_edit', capability: 'i2i' },
    prompt,
    interfaceMode: 'images',
    quality: 'auto',
    width: dimensionsAreSupported ? input.referenceDimensions.width : null,
    height: dimensionsAreSupported ? input.referenceDimensions.height : null,
    aspectRatio: 'auto',
    count: 1,
  });
  const configNode: ConfigNode = {
    ...base,
    data: {
      ...base.data,
      task: 'image_edit',
      capability: 'i2i',
      providerId: plan.model.providerId,
      model: plan.model.model,
      prompt,
      parameters: {
        canvasOperation: CREATIVE_IMAGE_MASK_EDIT_OPERATION,
        userPrompt: input.userPrompt.trim(),
        sourceNodeId: input.sourceNode.id,
        sourceAssetId: input.sourceAsset.id,
        markedReferenceAssetId: input.markedReference.id,
        referenceWidth: input.referenceDimensions.width,
        referenceHeight: input.referenceDimensions.height,
      },
      inputAssetIds: [input.markedReference.id],
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
    throw new Error(`无法连接局部编辑配置节点：${validation.code}。`);
  }
  return { configNode, connection, plan };
}

export function canvasImageMaskEditResumeRequests(
  document: CreativeProjectDocument
): CreativeWorkbenchResumeRequest[] {
  const owners = new Map(
    document.nodes
      .filter(isCanvasImageMaskEditConfig)
      .map((node) => [node.id, node])
  );
  return workbenchResumeRequestsFromDocument(document).filter(
    (request) =>
      request.reference.owner.kind === 'canvas_node' &&
      owners.has(request.reference.owner.nodeId)
  );
}

export function canvasImageMaskEditConfigForReference(
  document: Pick<CreativeProjectDocument, 'projectId' | 'nodes'>,
  reference: CreativeTaskReference
): ConfigNode {
  if (
    !isCanvasNodeTaskOwner(reference.owner) ||
    reference.owner.projectId !== document.projectId
  ) {
    throw new Error('局部编辑任务不属于当前画布项目。');
  }
  const owner = reference.owner;
  const node = document.nodes.find(
    (candidate) => candidate.id === owner.nodeId
  );
  if (!isCanvasImageMaskEditConfig(node)) {
    throw new Error('局部编辑任务缺少 canonical 配置节点。');
  }
  if (
    node.data.taskId !== reference.taskId ||
    node.data.providerId !== reference.providerId ||
    node.data.model !== reference.model ||
    node.data.task !== reference.task ||
    node.data.capability !== reference.capability
  ) {
    throw new Error('局部编辑任务与配置节点身份不一致。');
  }
  return node;
}

export function canvasImageMaskEditConfigFromTask(
  document: Pick<CreativeProjectDocument, 'projectId' | 'nodes'>,
  task: CreativeTask
): ConfigNode {
  return canvasImageMaskEditConfigForReference(document, {
    taskId: task.taskId,
    owner: task.owner,
    providerId: task.providerId,
    model: task.model,
    task: task.task,
    capability: task.capability,
  });
}

export function reconcileCanvasImageMaskEditConfig(
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
          ? (task.error?.message ?? '图片编辑失败。')
          : task.status === 'canceled'
            ? '图片编辑已取消。'
            : null,
    },
  };
}

export function canvasImageMaskEditResultPosition(
  nodes: readonly CreativeCanvasNode[],
  config: ConfigNode
): { x: number; y: number } {
  return nextCanvasImageTaskPosition(
    nodes,
    config,
    CREATIVE_CANVAS_PRODUCT_NODE_SIZES.image
  );
}
