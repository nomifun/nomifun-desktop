/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import type { CreativeAsset, CreativeAssetPort } from '../../assets';
import type { CreativeCanvasNode, CreativeSize } from '../../domain';
import {
  isTerminalCreativeTaskStatus,
  type CreativeTask,
  type CreativeTaskReference,
} from '../../tasks';
import { canvasCommands, validateCanvasConnection } from '../core';
import type { CreativeCanvasEditorHandle } from '../editor';
import {
  canvasImageComposeConfigForReference,
  canvasImageComposeConfigFromTask,
  canvasImageComposeResultPosition,
  canvasImageComposeSourceNodeId,
  clearCanvasImageComposeDraftModel,
  reconcileCanvasImageComposeConfig,
} from './canvasImageComposerCanvas';
import { creativeStudioProductText } from './i18n';
import { creativeNodeFromAsset } from './nodeFactory';

export type CanvasImageComposerEditorPort = Pick<
  CreativeCanvasEditorHandle,
  | 'addPendingTask'
  | 'dispatch'
  | 'getState'
  | 'removePendingTask'
>;

export interface CanvasImageComposerAssetPort extends CreativeAssetPort {
  get(assetId: string): Promise<CreativeAsset>;
}

const taskDocument = (
  editor: CanvasImageComposerEditorPort,
  projectId: string
) => ({ projectId, nodes: editor.getState().document.nodes });

/** Validate and durably flush the exact config owner before POST. */
export async function persistCanvasImageComposePendingTask(input: {
  editor: CanvasImageComposerEditorPort;
  projectId: string;
  reference: CreativeTaskReference;
}): Promise<void> {
  canvasImageComposeConfigForReference(
    taskDocument(input.editor, input.projectId),
    input.reference
  );
  await input.editor.addPendingTask(input.reference.taskId);
}

/** Reflect authoritative queued/running state without user undo history. */
export function reconcileCanvasImageComposeTask(input: {
  editor: CanvasImageComposerEditorPort;
  projectId: string;
  task: CreativeTask;
}): void {
  const config = canvasImageComposeConfigFromTask(
    taskDocument(input.editor, input.projectId),
    input.task
  );
  input.editor.dispatch(
    canvasCommands.reconcileRuntimeNode(
      reconcileCanvasImageComposeConfig(config, input.task)
    )
  );
}

/**
 * Project a terminal task into real image assets and graph edges. The operation
 * is idempotent, so an interrupted terminal CAS can safely repeat.
 */
export async function settleCanvasImageComposeTask(input: {
  editor: CanvasImageComposerEditorPort;
  projectId: string;
  task: CreativeTask;
  assets: CanvasImageComposerAssetPort;
  viewportSize: CreativeSize;
  onAsset?: (asset: CreativeAsset) => void;
}): Promise<void> {
  if (!isTerminalCreativeTaskStatus(input.task.status)) {
    throw new Error(
      creativeStudioProductText(
        'creativeStudio.canvas.errors.image.nonTerminalRemovalRejected',
        '拒绝将非终态图片创作任务移出 pending 列表。'
      )
    );
  }
  const initialConfig = canvasImageComposeConfigFromTask(
    taskDocument(input.editor, input.projectId),
    input.task
  );
  input.editor.dispatch(
    canvasCommands.reconcileRuntimeNode(
      reconcileCanvasImageComposeConfig(initialConfig, input.task)
    )
  );

  if (input.task.status === 'succeeded') {
    if (input.task.resultAssetIds.length === 0) {
      throw new Error(
        creativeStudioProductText(
          'creativeStudio.canvas.errors.image.missingResult',
          '图片创作任务成功但没有返回真实图片素材。'
        )
      );
    }
    const resultIds = [...new Set(input.task.resultAssetIds)];
    if (resultIds.length !== input.task.resultAssetIds.length) {
      throw new Error(
        creativeStudioProductText(
          'creativeStudio.canvas.errors.image.duplicateResults',
          '图片创作任务返回了重复的结果素材。'
        )
      );
    }
    if (resultIds.some((assetId) => initialConfig.data.inputAssetIds.includes(assetId))) {
      throw new Error(
        creativeStudioProductText(
          'creativeStudio.canvas.errors.image.reusedInputAsset',
          '图片创作结果错误地复用了输入素材，已停止写入画布。'
        )
      );
    }
    const sourceNodeId = canvasImageComposeSourceNodeId(initialConfig);
    const sourceAssetParameter = initialConfig.data.operation?.sourceAssetId;
    if (
      sourceAssetParameter !== null &&
      (typeof sourceAssetParameter !== 'string' || !sourceAssetParameter.trim())
    ) {
      throw new Error(
        creativeStudioProductText(
          'creativeStudio.canvas.errors.image.invalidSourceAssetId',
          '图片创作配置的 sourceAssetId 非法。'
        )
      );
    }
    const replacesEmptySource = sourceAssetParameter === null;
    const resultAssets = await Promise.all(
      resultIds.map((assetId) => input.assets.get(assetId))
    );
    if (
      resultAssets.some(
        (asset, index) => asset.id !== resultIds[index] || asset.kind !== 'image'
      )
    ) {
      throw new Error(
        creativeStudioProductText(
          'creativeStudio.canvas.errors.image.resultResolutionFailed',
          '图片创作结果未解析为对应的真实图片素材。'
        )
      );
    }

    const resultNodeIds: string[] = [];
    const at = Date.now();
    const mergeKey = `image-compose:${initialConfig.id}:${input.task.taskId}`;
    for (const [index, asset] of resultAssets.entries()) {
      input.onAsset?.(asset);
      let state = input.editor.getState();
      const source = state.document.nodes.find(
        (node): node is Extract<CreativeCanvasNode, { type: 'image' }> =>
          node.id === sourceNodeId && node.type === 'image'
      );
      if (!source) {
        throw new Error(
          creativeStudioProductText(
            'creativeStudio.canvas.errors.image.sourceRemoved',
            '图片创作源节点在结果写入前被移除。'
          )
        );
      }
      let resultNode: Extract<CreativeCanvasNode, { type: 'image' }> | undefined;
      if (replacesEmptySource && index === 0) {
        if (source.data.assetId !== null && source.data.assetId !== asset.id) {
          throw new Error(
            creativeStudioProductText(
              'creativeStudio.canvas.errors.image.sourceOccupied',
              '空图片节点在任务完成前已关联其他素材，已停止覆盖。'
            )
          );
        }
        resultNode = source.data.assetId === asset.id
          ? source
          : clearCanvasImageComposeDraftModel({
              ...source,
              data: {
                ...source.data,
                assetId: asset.id,
                caption: asset.title,
                alt: asset.title,
                naturalSize:
                  asset.width && asset.height
                    ? { width: asset.width, height: asset.height }
                    : null,
              },
            });
        if (source.data.assetId !== asset.id) {
          input.editor.dispatch(canvasCommands.reconcileRuntimeNode(resultNode));
          state = input.editor.getState();
        }
      } else {
        resultNode = state.document.nodes.find(
          (node): node is Extract<CreativeCanvasNode, { type: 'image' }> =>
            node.type === 'image' && node.data.assetId === asset.id
        );
      }
      if (!resultNode) {
        const config = state.document.nodes.find(
          (node): node is Extract<CreativeCanvasNode, { type: 'config' }> =>
            node.id === initialConfig.id && node.type === 'config'
        );
        if (!config) {
          throw new Error(
            creativeStudioProductText(
              'creativeStudio.canvas.errors.image.configRemoved',
              '图片创作配置节点在结果写入前被移除。'
            )
          );
        }
        const created = creativeNodeFromAsset(asset, state, input.viewportSize, {
          position: canvasImageComposeResultPosition(state.document.nodes, config),
        });
        if (created.type !== 'image') {
          throw new Error(
            creativeStudioProductText(
              'creativeStudio.canvas.errors.image.nodeConstructionFailed',
              '图片创作结果未能构造成图片节点。'
            )
          );
        }
        resultNode = created;
        input.editor.dispatch(
          canvasCommands.addNode(resultNode, { at, mergeKey })
        );
        state = input.editor.getState();
      }
      const connected = state.document.connections.some(
        (connection) =>
          connection.sourceNodeId === initialConfig.id &&
          connection.targetNodeId === resultNode.id
      );
      if (!connected && !(replacesEmptySource && index === 0)) {
        const connection = {
          sourceNodeId: initialConfig.id,
          targetNodeId: resultNode.id,
          sourceHandle: 'source',
          targetHandle: 'target',
        };
        const validation = validateCanvasConnection(state.document, connection);
        if (!validation.ok) {
          throw new Error(
            creativeStudioProductText(
              'creativeStudio.canvas.errors.image.connectResultFailed',
              '无法连接图片创作结果：{{code}}。',
              { code: validation.code }
            )
          );
        }
        input.editor.dispatch(
          canvasCommands.connect(initialConfig.id, resultNode.id, {
            sourceHandle: 'source',
            targetHandle: 'target',
            at,
            mergeKey,
          })
        );
      }
      resultNodeIds.push(resultNode.id);
    }
    input.editor.dispatch(canvasCommands.setSelection(resultNodeIds));
  }

  await input.editor.removePendingTask(input.task.taskId);
}

/** Remove only a confirmed-404 orphan; ambiguous transport remains pending. */
export async function orphanCanvasImageComposeTask(input: {
  editor: CanvasImageComposerEditorPort;
  projectId: string;
  reference: CreativeTaskReference;
}): Promise<void> {
  const config = canvasImageComposeConfigForReference(
    taskDocument(input.editor, input.projectId),
    input.reference
  );
  input.editor.dispatch(
    canvasCommands.reconcileRuntimeNode({
      ...config,
      locked: false,
      data: {
        ...config.data,
        status: 'failed',
        resultAssetIds: [],
        errorMessage: creativeStudioProductText(
          'creativeStudio.canvas.errors.taskMissingRecoveryCleared',
          '服务器未找到该任务；已确认清理恢复标记。'
        ),
      },
    })
  );
  await input.editor.removePendingTask(input.reference.taskId);
}
