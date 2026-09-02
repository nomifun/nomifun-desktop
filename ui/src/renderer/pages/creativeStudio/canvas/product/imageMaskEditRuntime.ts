/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { isBackendHttpError } from '@/common/adapter/httpBridge';

import type { CreativeAsset, CreativeAssetPort } from '../../assets';
import type { CreativeCanvasNode, CreativeSize } from '../../domain';
import {
  isTerminalCreativeTaskStatus,
  type CreateCreativeTaskInput,
  type CreativeTask,
  type CreativeTaskReference,
} from '../../tasks';
import type {
  CreativeWorkbenchRuntimeController,
  CreativeWorkbenchRuntimeSnapshot,
} from '../../workbenches/runtime';
import { canvasCommands, validateCanvasConnection } from '../core';
import type { CreativeCanvasEditorHandle } from '../editor';
import {
  canvasImageMaskEditConfigForReference,
  canvasImageMaskEditConfigFromTask,
  canvasImageMaskEditResultPosition,
  reconcileCanvasImageMaskEditConfig,
} from './imageMaskEditCanvas';
import { creativeStudioProductText } from './i18n';
import { creativeNodeFromAsset } from './nodeFactory';

export type CanvasImageMaskEditEditorPort = Pick<
  CreativeCanvasEditorHandle,
  | 'addPendingTask'
  | 'dispatch'
  | 'getPendingTaskIds'
  | 'getState'
  | 'removePendingTask'
>;

export interface CanvasImageMaskEditAssetPort extends CreativeAssetPort {
  get(assetId: string): Promise<CreativeAsset>;
}

export type CanvasImageMaskEditAdmission =
  | { kind: 'admitted'; taskId: string }
  | { kind: 'submission_failure'; order: number; error: Error };

const taskDocument = (
  editor: CanvasImageMaskEditEditorPort,
  projectId: string
) => ({ projectId, nodes: editor.getState().document.nodes });

const requiredImageMaskOperation = (
  node: Extract<CreativeCanvasNode, { type: 'config' }>
) => {
  const operation = node.data.operation;
  if (operation?.kind !== 'image-mask-edit') {
    throw new Error(
      creativeStudioProductText(
        'creativeStudio.canvas.errors.mask.missingOperation',
        '局部编辑配置缺少 canonical operation。'
      )
    );
  }
  return operation;
};

export function creativeTaskReferenceFromInput(
  input: CreateCreativeTaskInput
): CreativeTaskReference {
  return {
    taskId: input.idempotencyKey,
    owner: structuredClone(input.owner),
    providerId: input.providerId,
    model: input.model,
    task: input.task,
    capability: input.capability,
  };
}

export function isCanvasImageMaskEditTaskNotFound(error: unknown): boolean {
  return isBackendHttpError(error) && error.status === 404;
}

/** The canonical config owner and pending id are flushed before the first POST. */
export async function persistCanvasImageMaskEditPendingTask(input: {
  editor: CanvasImageMaskEditEditorPort;
  projectId: string;
  reference: CreativeTaskReference;
}): Promise<void> {
  canvasImageMaskEditConfigForReference(
    taskDocument(input.editor, input.projectId),
    input.reference
  );
  await input.editor.addPendingTask(input.reference.taskId);
}

/** Reflect backend queued/running state without creating user undo history. */
export function reconcileCanvasImageMaskEditTask(input: {
  editor: CanvasImageMaskEditEditorPort;
  projectId: string;
  task: CreativeTask;
}): void {
  const config = canvasImageMaskEditConfigFromTask(
    taskDocument(input.editor, input.projectId),
    input.task
  );
  input.editor.dispatch(
    canvasCommands.reconcileRuntimeNode(
      reconcileCanvasImageMaskEditConfig(config, input.task)
    )
  );
}

/**
 * Project terminal state into the canvas. Result assets and connections are
 * idempotent so a failed CAS flush can safely retry the exact settled task.
 */
export async function settleCanvasImageMaskEditTask(input: {
  editor: CanvasImageMaskEditEditorPort;
  projectId: string;
  task: CreativeTask;
  assets: CanvasImageMaskEditAssetPort;
  viewportSize: CreativeSize;
  onAsset?: (asset: CreativeAsset) => void;
}): Promise<void> {
  if (!isTerminalCreativeTaskStatus(input.task.status)) {
    throw new Error(
      creativeStudioProductText(
        'creativeStudio.canvas.errors.mask.nonTerminalRemovalRejected',
        '拒绝将非终态局部编辑任务移出 pending 列表。'
      )
    );
  }
  const initialConfig = canvasImageMaskEditConfigFromTask(
    taskDocument(input.editor, input.projectId),
    input.task
  );
  input.editor.dispatch(
    canvasCommands.reconcileRuntimeNode(
      reconcileCanvasImageMaskEditConfig(initialConfig, input.task)
    )
  );

  if (input.task.status === 'succeeded') {
    if (input.task.resultAssetIds.length === 0) {
      throw new Error(
        creativeStudioProductText(
          'creativeStudio.canvas.errors.mask.missingResult',
          '局部编辑任务成功但没有返回真实图片素材。'
        )
      );
    }
    const resultIds = [...new Set(input.task.resultAssetIds)];
    if (resultIds.length !== input.task.resultAssetIds.length) {
      throw new Error(
        creativeStudioProductText(
          'creativeStudio.canvas.errors.mask.duplicateResults',
          '局部编辑任务返回了重复的结果素材。'
        )
      );
    }
    const { sourceAssetId, markedReferenceAssetId } =
      requiredImageMaskOperation(initialConfig);
    if (
      resultIds.some(
        (assetId) =>
          assetId === sourceAssetId || assetId === markedReferenceAssetId
      )
    ) {
      throw new Error(
        creativeStudioProductText(
          'creativeStudio.canvas.errors.mask.reusedInputAsset',
          '局部编辑结果错误地复用了输入素材，已停止写入画布。'
        )
      );
    }

    const resultAssets = await Promise.all(
      resultIds.map((assetId) => input.assets.get(assetId))
    );
    if (
      resultAssets.some(
        (asset, index) =>
          asset.id !== resultIds[index] || asset.kind !== 'image'
      )
    ) {
      throw new Error(
        creativeStudioProductText(
          'creativeStudio.canvas.errors.mask.resultResolutionFailed',
          '局部编辑结果未解析为对应的真实图片素材。'
        )
      );
    }

    const resultNodeIds: string[] = [];
    const at = Date.now();
    const mergeKey = `image-mask-edit:${initialConfig.id}:${input.task.taskId}`;
    for (const asset of resultAssets) {
      input.onAsset?.(asset);
      let state = input.editor.getState();
      let resultNode = state.document.nodes.find(
        (node): node is Extract<CreativeCanvasNode, { type: 'image' }> =>
          node.type === 'image' && node.data.assetId === asset.id
      );
      if (!resultNode) {
        const config = state.document.nodes.find(
          (node): node is Extract<CreativeCanvasNode, { type: 'config' }> =>
            node.id === initialConfig.id && node.type === 'config'
        );
        if (!config) {
          throw new Error(
            creativeStudioProductText(
              'creativeStudio.canvas.errors.mask.configRemoved',
              '局部编辑任务记录在结果写入前丢失。'
            )
          );
        }
        const created = creativeNodeFromAsset(
          asset,
          state,
          input.viewportSize,
          {
            position: canvasImageMaskEditResultPosition(
              state.document.nodes,
              config
            ),
          }
        );
        if (created.type !== 'image') {
          throw new Error(
            creativeStudioProductText(
              'creativeStudio.canvas.errors.mask.nodeConstructionFailed',
              '局部编辑结果未能构造成图片节点。'
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
              'creativeStudio.canvas.errors.mask.connectResultFailed',
              '无法连接局部编辑结果：{{code}}。',
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

  // This flush is deliberately last: it persists terminal config, results,
  // connections, and pending removal in one latest-document CAS write.
  await input.editor.removePendingTask(input.task.taskId);
}

/** Remove only a confirmed-404 orphan; transport ambiguity must remain pending. */
export async function orphanCanvasImageMaskEditTask(input: {
  editor: CanvasImageMaskEditEditorPort;
  projectId: string;
  reference: CreativeTaskReference;
}): Promise<void> {
  const config = canvasImageMaskEditConfigForReference(
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

/** Resolve as soon as POST is admitted or becomes an explicitly retryable slot. */
export function waitForCanvasImageMaskEditAdmission(input: {
  controller: Pick<CreativeWorkbenchRuntimeController, 'subscribe'>;
  idempotencyKey: string;
  start: () => Promise<CreativeWorkbenchRuntimeSnapshot>;
}): Promise<CanvasImageMaskEditAdmission> {
  let operation: Promise<CreativeWorkbenchRuntimeSnapshot>;
  try {
    // start() synchronously clears an older failure before its first await.
    operation = input.start();
  } catch (error) {
    return Promise.reject(error);
  }

  return new Promise((resolve, reject) => {
    let settled = false;
    let unsubscribe: () => void = () => undefined;
    const finish = (result: CanvasImageMaskEditAdmission): void => {
      if (settled) return;
      settled = true;
      unsubscribe();
      resolve(result);
    };
    const subscribed = input.controller.subscribe((snapshot) => {
      const admitted = snapshot.entries.find(
        (entry) => entry.task.taskId === input.idempotencyKey
      );
      if (admitted) {
        finish({ kind: 'admitted', taskId: admitted.task.taskId });
        return;
      }
      const failure = snapshot.submissionFailures.find(
        (candidate) => candidate.input.idempotencyKey === input.idempotencyKey
      );
      if (failure) {
        finish({
          kind: 'submission_failure',
          order: failure.order,
          error: failure.error,
        });
      }
    });
    unsubscribe = subscribed;
    if (settled) unsubscribe();
    void operation.then(
      () => {
        if (settled) return;
        settled = true;
        unsubscribe();
        reject(
          new Error(
            creativeStudioProductText(
              'creativeStudio.canvas.errors.mask.endedBeforeAdmission',
              '局部编辑任务在后端确认接收前结束。'
            )
          )
        );
      },
      (error) => {
        if (settled) return;
        settled = true;
        unsubscribe();
        reject(error);
      }
    );
  });
}

export function canvasImageMaskEditTaskFromSnapshot(
  snapshot: CreativeWorkbenchRuntimeSnapshot,
  taskId: string
): CreativeTask | null {
  return (
    snapshot.entries.find((entry) => entry.task.taskId === taskId)?.task ?? null
  );
}
