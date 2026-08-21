/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import type { CreativeAsset, CreativeAssetPort } from '../../assets';
import type { CreativeCanvasNode } from '../../domain';
import {
  isTerminalCreativeTaskStatus,
  type CreativeTask,
  type CreativeTaskReference,
} from '../../tasks';
import { canvasCommands } from '../core';
import type { CreativeCanvasEditorHandle } from '../editor';
import {
  canvasAudioComposeConfigForReference,
  canvasAudioComposeConfigFromTask,
  canvasAudioComposeSourceAssetId,
  canvasAudioComposeSourceNodeId,
  reconcileCanvasAudioComposeConfig,
} from './canvasAudioComposerCanvas';

export type CanvasAudioComposerEditorPort = Pick<
  CreativeCanvasEditorHandle,
  'addPendingTask' | 'dispatch' | 'getState' | 'removePendingTask'
>;

export interface CanvasAudioComposerAssetPort extends CreativeAssetPort {
  get(assetId: string): Promise<CreativeAsset>;
}

const taskDocument = (
  editor: CanvasAudioComposerEditorPort,
  projectId: string
) => ({ projectId, nodes: editor.getState().document.nodes });

/** Validate and durably flush the exact config owner before POST. */
export async function persistCanvasAudioComposePendingTask(input: {
  editor: CanvasAudioComposerEditorPort;
  projectId: string;
  reference: CreativeTaskReference;
}): Promise<void> {
  canvasAudioComposeConfigForReference(
    taskDocument(input.editor, input.projectId),
    input.reference
  );
  await input.editor.addPendingTask(input.reference.taskId);
}

/** Reflect authoritative queued/running state without user undo history. */
export function reconcileCanvasAudioComposeTask(input: {
  editor: CanvasAudioComposerEditorPort;
  projectId: string;
  task: CreativeTask;
}): void {
  if (isTerminalCreativeTaskStatus(input.task.status)) {
    throw new Error('音频创作终态必须通过 settlement 写入画布。');
  }
  const config = canvasAudioComposeConfigFromTask(
    taskDocument(input.editor, input.projectId),
    input.task
  );
  input.editor.dispatch(
    canvasCommands.reconcileRuntimeNode(
      reconcileCanvasAudioComposeConfig(config, input.task)
    )
  );
}

/**
 * Settle one terminal TTS task into its existing empty audio node. The current
 * contract accepts exactly one real audio asset and never creates a derived
 * node, so an interrupted terminal CAS can safely replay this operation.
 */
export async function settleCanvasAudioComposeTask(input: {
  editor: CanvasAudioComposerEditorPort;
  projectId: string;
  task: CreativeTask;
  assets: CanvasAudioComposerAssetPort;
  onAsset?: (asset: CreativeAsset) => void;
}): Promise<void> {
  if (!isTerminalCreativeTaskStatus(input.task.status)) {
    throw new Error('拒绝将非终态音频创作任务移出 pending 列表。');
  }
  const initialConfig = canvasAudioComposeConfigFromTask(
    taskDocument(input.editor, input.projectId),
    input.task
  );
  input.editor.dispatch(
    canvasCommands.reconcileRuntimeNode(
      reconcileCanvasAudioComposeConfig(initialConfig, input.task)
    )
  );

  if (input.task.status === 'succeeded') {
    if (input.task.resultAssetIds.length !== 1) {
      throw new Error('音频创作任务必须恰好返回一个真实音频素材。');
    }
    if (initialConfig.data.inputAssetIds.length !== 0) {
      throw new Error('当前 TTS 音频创作不允许输入素材。');
    }
    if (canvasAudioComposeSourceAssetId(initialConfig) !== null) {
      throw new Error('当前音频创作只允许空音频节点承接 TTS 结果。');
    }

    const resultAssetId = input.task.resultAssetIds[0];
    const sourceNodeId = canvasAudioComposeSourceNodeId(initialConfig);
    const sourceBeforeAsset = input.editor
      .getState()
      .document.nodes.find(
        (node): node is Extract<CreativeCanvasNode, { type: 'audio' }> =>
          node.id === sourceNodeId && node.type === 'audio'
      );
    if (!sourceBeforeAsset) {
      throw new Error('音频创作源节点在结果写入前被移除。');
    }
    if (
      sourceBeforeAsset.data.assetId !== null &&
      sourceBeforeAsset.data.assetId !== resultAssetId
    ) {
      throw new Error('空音频节点在任务完成前已关联其他素材，已停止覆盖。');
    }

    const asset = await input.assets.get(resultAssetId);
    if (asset.id !== resultAssetId || asset.kind !== 'audio') {
      throw new Error('音频创作结果未解析为对应的真实音频素材。');
    }
    input.onAsset?.(asset);

    const source = input.editor
      .getState()
      .document.nodes.find(
        (node): node is Extract<CreativeCanvasNode, { type: 'audio' }> =>
          node.id === sourceNodeId && node.type === 'audio'
      );
    if (!source) {
      throw new Error('音频创作源节点在结果写入前被移除。');
    }
    if (source.data.assetId !== null && source.data.assetId !== asset.id) {
      throw new Error('空音频节点在任务完成前已关联其他素材，已停止覆盖。');
    }
    if (source.data.assetId !== asset.id || source.data.title !== asset.title) {
      input.editor.dispatch(
        canvasCommands.reconcileRuntimeNode({
          ...source,
          data: {
            ...source.data,
            assetId: asset.id,
            title: asset.title,
            composer: source.data.composer
              ? {
                  ...source.data.composer,
                  model: null,
                }
              : null,
          },
        })
      );
    }
    input.editor.dispatch(canvasCommands.setSelection([source.id]));
  }

  await input.editor.removePendingTask(input.task.taskId);
}

/** Remove only a confirmed-404 orphan; ambiguous transport remains pending. */
export async function orphanCanvasAudioComposeTask(input: {
  editor: CanvasAudioComposerEditorPort;
  projectId: string;
  reference: CreativeTaskReference;
}): Promise<void> {
  const config = canvasAudioComposeConfigForReference(
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
        errorMessage: '服务器未找到该任务；已确认清理恢复标记。',
      },
    })
  );
  await input.editor.removePendingTask(input.reference.taskId);
}
