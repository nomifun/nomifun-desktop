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
  canvasVideoComposeConfigForReference,
  canvasVideoComposeConfigFromTask,
  canvasVideoComposeResultPosition,
  canvasVideoComposeSourceAssetId,
  canvasVideoComposeSourceNodeId,
  clearCanvasVideoComposeDraftModel,
  reconcileCanvasVideoComposeConfig,
} from './canvasVideoComposerCanvas';
import { creativeStudioProductText } from './i18n';
import { creativeNodeFromAsset } from './nodeFactory';

export type CanvasVideoComposerEditorPort = Pick<
  CreativeCanvasEditorHandle,
  'addPendingTask' | 'dispatch' | 'getState' | 'removePendingTask'
>;

export interface CanvasVideoComposerAssetPort extends CreativeAssetPort {
  get(assetId: string): Promise<CreativeAsset>;
}

const taskDocument = (editor: CanvasVideoComposerEditorPort, projectId: string) => ({
  projectId,
  nodes: editor.getState().document.nodes,
});

/** Validate and durably flush the exact config owner before POST. */
export async function persistCanvasVideoComposePendingTask(input: {
  editor: CanvasVideoComposerEditorPort;
  projectId: string;
  reference: CreativeTaskReference;
}): Promise<void> {
  canvasVideoComposeConfigForReference(
    taskDocument(input.editor, input.projectId),
    input.reference
  );
  await input.editor.addPendingTask(input.reference.taskId);
}

/** Reflect authoritative queued/running state without user undo history. */
export function reconcileCanvasVideoComposeTask(input: {
  editor: CanvasVideoComposerEditorPort;
  projectId: string;
  task: CreativeTask;
}): void {
  if (isTerminalCreativeTaskStatus(input.task.status)) {
    throw new Error(
      creativeStudioProductText(
        'creativeStudio.canvas.errors.video.terminalSettlementRequired',
        '视频创作终态必须通过 settlement 写入画布。'
      )
    );
  }
  const config = canvasVideoComposeConfigFromTask(
    taskDocument(input.editor, input.projectId),
    input.task
  );
  input.editor.dispatch(
    canvasCommands.reconcileRuntimeNode(reconcileCanvasVideoComposeConfig(config, input.task))
  );
}

/**
 * Project a terminal task into real video assets and graph edges. The current
 * t2v/i2v contract owns an empty video node: the first result fills that node,
 * while any additional results become config-linked derived video nodes.
 * Repeating an interrupted terminal CAS is idempotent.
 */
export async function settleCanvasVideoComposeTask(input: {
  editor: CanvasVideoComposerEditorPort;
  projectId: string;
  task: CreativeTask;
  assets: CanvasVideoComposerAssetPort;
  viewportSize: CreativeSize;
  onAsset?: (asset: CreativeAsset) => void;
}): Promise<void> {
  if (!isTerminalCreativeTaskStatus(input.task.status)) {
    throw new Error(
      creativeStudioProductText(
        'creativeStudio.canvas.errors.video.nonTerminalRemovalRejected',
        '拒绝将非终态视频创作任务移出 pending 列表。'
      )
    );
  }
  const initialConfig = canvasVideoComposeConfigFromTask(
    taskDocument(input.editor, input.projectId),
    input.task
  );
  input.editor.dispatch(
    canvasCommands.reconcileRuntimeNode(
      reconcileCanvasVideoComposeConfig(initialConfig, input.task)
    )
  );

  if (input.task.status === 'succeeded') {
    if (input.task.resultAssetIds.length === 0) {
      throw new Error(
        creativeStudioProductText(
          'creativeStudio.canvas.errors.video.missingResult',
          '视频创作任务成功但没有返回真实视频素材。'
        )
      );
    }
    const resultIds = [...new Set(input.task.resultAssetIds)];
    if (resultIds.length !== input.task.resultAssetIds.length) {
      throw new Error(
        creativeStudioProductText(
          'creativeStudio.canvas.errors.video.duplicateResults',
          '视频创作任务返回了重复的结果素材。'
        )
      );
    }
    if (resultIds.some((assetId) => initialConfig.data.inputAssetIds.includes(assetId))) {
      throw new Error(
        creativeStudioProductText(
          'creativeStudio.canvas.errors.video.reusedInputAsset',
          '视频创作结果错误地复用了输入素材，已停止写入画布。'
        )
      );
    }
    if (canvasVideoComposeSourceAssetId(initialConfig) !== null) {
      throw new Error(
        creativeStudioProductText(
          'creativeStudio.canvas.errors.video.resultRequiresEmptyNode',
          '当前视频创作只允许空视频节点承接 t2v/i2v 结果。'
        )
      );
    }

    const sourceNodeId = canvasVideoComposeSourceNodeId(initialConfig);
    const sourceBeforeAssets = input.editor
      .getState()
      .document.nodes.find(
        (node): node is Extract<CreativeCanvasNode, { type: 'video' }> =>
          node.id === sourceNodeId && node.type === 'video'
      );
    if (!sourceBeforeAssets) {
      throw new Error(
        creativeStudioProductText(
          'creativeStudio.canvas.errors.video.sourceRemoved',
          '视频创作源节点在结果写入前被移除。'
        )
      );
    }
    if (
      sourceBeforeAssets.data.assetId !== null &&
      sourceBeforeAssets.data.assetId !== resultIds[0]
    ) {
      throw new Error(
        creativeStudioProductText(
          'creativeStudio.canvas.errors.video.sourceOccupied',
          '空视频节点在任务完成前已关联其他素材，已停止覆盖。'
        )
      );
    }

    const resultAssets = await Promise.all(resultIds.map((assetId) => input.assets.get(assetId)));
    if (
      resultAssets.some((asset, index) => asset.id !== resultIds[index] || asset.kind !== 'video')
    ) {
      throw new Error(
        creativeStudioProductText(
          'creativeStudio.canvas.errors.video.resultResolutionFailed',
          '视频创作结果未解析为对应的真实视频素材。'
        )
      );
    }

    const resultNodeIds: string[] = [];
    const at = Date.now();
    const mergeKey = `video-compose:${initialConfig.id}:${input.task.taskId}`;
    for (const [index, asset] of resultAssets.entries()) {
      input.onAsset?.(asset);
      let state = input.editor.getState();
      const source = state.document.nodes.find(
        (node): node is Extract<CreativeCanvasNode, { type: 'video' }> =>
          node.id === sourceNodeId && node.type === 'video'
      );
      if (!source) {
        throw new Error(
          creativeStudioProductText(
            'creativeStudio.canvas.errors.video.sourceRemoved',
            '视频创作源节点在结果写入前被移除。'
          )
        );
      }

      let resultNode: Extract<CreativeCanvasNode, { type: 'video' }> | undefined;
      if (index === 0) {
        if (source.data.assetId !== null && source.data.assetId !== asset.id) {
          throw new Error(
            creativeStudioProductText(
              'creativeStudio.canvas.errors.video.sourceOccupied',
              '空视频节点在任务完成前已关联其他素材，已停止覆盖。'
            )
          );
        }
        const reconciledSource = clearCanvasVideoComposeDraftModel({
          ...source,
          data: { ...source.data, assetId: asset.id },
        });
        if (source.data.assetId !== asset.id || Boolean(source.data.composer?.model)) {
          input.editor.dispatch(canvasCommands.reconcileRuntimeNode(reconciledSource));
          state = input.editor.getState();
        }
        resultNode = reconciledSource;
      } else {
        resultNode = state.document.nodes.find(
          (node): node is Extract<CreativeCanvasNode, { type: 'video' }> =>
            node.id !== sourceNodeId && node.type === 'video' && node.data.assetId === asset.id
        );
        if (!resultNode) {
          const config = state.document.nodes.find(
            (node): node is Extract<CreativeCanvasNode, { type: 'config' }> =>
              node.id === initialConfig.id && node.type === 'config'
          );
          if (!config) {
            throw new Error(
              creativeStudioProductText(
                'creativeStudio.canvas.errors.video.configRemoved',
                '视频创作配置节点在结果写入前被移除。'
              )
            );
          }
          const created = creativeNodeFromAsset(asset, state, input.viewportSize, {
            position: canvasVideoComposeResultPosition(state.document.nodes, config),
          });
          if (created.type !== 'video') {
            throw new Error(
              creativeStudioProductText(
                'creativeStudio.canvas.errors.video.nodeConstructionFailed',
                '视频创作结果未能构造成视频节点。'
              )
            );
          }
          resultNode = created;
          input.editor.dispatch(canvasCommands.addNode(resultNode, { at, mergeKey }));
          state = input.editor.getState();
        }
        const connected = state.document.connections.some(
          (connection) =>
            connection.sourceNodeId === initialConfig.id &&
            connection.targetNodeId === resultNode?.id
        );
        if (!connected) {
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
                'creativeStudio.canvas.errors.video.connectResultFailed',
                '无法连接视频创作结果：{{code}}。',
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
      }
      resultNodeIds.push(resultNode.id);
    }
    input.editor.dispatch(canvasCommands.setSelection(resultNodeIds));
  }

  await input.editor.removePendingTask(input.task.taskId);
}

/** Remove only a confirmed-404 orphan; ambiguous transport remains pending. */
export async function orphanCanvasVideoComposeTask(input: {
  editor: CanvasVideoComposerEditorPort;
  projectId: string;
  reference: CreativeTaskReference;
}): Promise<void> {
  const config = canvasVideoComposeConfigForReference(
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
