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
import { canvasCommands, validateCanvasConnection } from '../core';
import type { CreativeCanvasEditorHandle } from '../editor';
import {
  canvasAudioComposeConfigForReference,
  canvasAudioComposeConfigFromTask,
  canvasAudioComposeSourceAssetId,
  canvasAudioComposeSourceNodeId,
  reconcileCanvasAudioComposeConfig,
} from './canvasAudioComposerCanvas';
import { creativeStudioProductText } from './i18n';
import { canvasTaskResultPosition } from './imageTaskCanvasLayout';
import { creativeNodeFromHistoricalAsset } from './nodeFactory';

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
    throw new Error(
      creativeStudioProductText(
        'creativeStudio.canvas.errors.audio.terminalSettlementRequired',
        '音频创作终态必须通过 settlement 写入画布。'
      )
    );
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
 * Settle one TTS result without overwriting an earlier parallel result. An
 * occupied source gets a config-linked output; terminal CAS replay is idempotent.
 */
export async function settleCanvasAudioComposeTask(input: {
  editor: CanvasAudioComposerEditorPort;
  projectId: string;
  task: CreativeTask;
  assets: CanvasAudioComposerAssetPort;
  onAsset?: (asset: CreativeAsset) => void;
}): Promise<void> {
  if (!isTerminalCreativeTaskStatus(input.task.status)) {
    throw new Error(
      creativeStudioProductText(
        'creativeStudio.canvas.errors.audio.nonTerminalRemovalRejected',
        '拒绝将非终态音频创作任务移出 pending 列表。'
      )
    );
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
      throw new Error(
        creativeStudioProductText(
          'creativeStudio.canvas.errors.audio.singleResultRequired',
          '音频创作任务必须恰好返回一个真实音频素材。'
        )
      );
    }
    if (initialConfig.data.inputAssetIds.length !== 0) {
      throw new Error(
        creativeStudioProductText(
          'creativeStudio.canvas.errors.audio.inputAssetsUnsupported',
          '当前 TTS 音频创作不允许输入素材。'
        )
      );
    }
    if (canvasAudioComposeSourceAssetId(initialConfig) !== null) {
      throw new Error(
        creativeStudioProductText(
          'creativeStudio.canvas.errors.audio.resultRequiresEmptyNode',
          '当前音频创作只允许空音频节点承接 TTS 结果。'
        )
      );
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
      throw new Error(
        creativeStudioProductText(
          'creativeStudio.canvas.errors.audio.sourceRemoved',
          '音频创作源节点在结果写入前被移除。'
        )
      );
    }
    const asset = await input.assets.get(resultAssetId);
    if (asset.id !== resultAssetId || asset.kind !== 'audio') {
      throw new Error(
        creativeStudioProductText(
          'creativeStudio.canvas.errors.audio.resultResolutionFailed',
          '音频创作结果未解析为对应的真实音频素材。'
        )
      );
    }
    input.onAsset?.(asset);

    const source = input.editor
      .getState()
      .document.nodes.find(
        (node): node is Extract<CreativeCanvasNode, { type: 'audio' }> =>
          node.id === sourceNodeId && node.type === 'audio'
      );
    if (!source) {
      throw new Error(
        creativeStudioProductText(
          'creativeStudio.canvas.errors.audio.sourceRemoved',
          '音频创作源节点在结果写入前被移除。'
        )
      );
    }
    if (source.data.assetId === null || source.data.assetId === asset.id) {
      if (source.data.assetId !== asset.id || source.data.title !== asset.title) {
        input.editor.dispatch(
          canvasCommands.reconcileRuntimeNode({
            ...source,
            data: {
              ...source.data,
              assetId: asset.id,
              title: asset.title,
              composer: source.data.composer
                ? { ...source.data.composer, model: null }
                : null,
            },
          })
        );
      }
    } else {
      let state = input.editor.getState();
      let result = state.document.nodes.find(
        (node): node is Extract<CreativeCanvasNode, { type: 'audio' }> =>
          node.type === 'audio' && node.data.assetId === asset.id
      );
      const mergeKey = `audio-compose:${initialConfig.id}:${input.task.taskId}`;
      const at = Date.now();
      if (!result) {
        const created = creativeNodeFromHistoricalAsset(asset, state, { width: 1, height: 1 });
        if (created.type !== 'audio') {
          throw new Error(creativeStudioProductText(
            'creativeStudio.canvas.errors.audio.nodeConstructionFailed',
            '音频创作结果未能构造成音频节点。'
          ));
        }
        const config = canvasAudioComposeConfigFromTask(taskDocument(input.editor, input.projectId), input.task);
        created.position = canvasTaskResultPosition(state.document.nodes, config, created.size);
        result = created;
        input.editor.dispatch(canvasCommands.addNode(result, { at, mergeKey, select: false }));
        state = input.editor.getState();
      }
      const connection = {
        sourceNodeId: initialConfig.id,
        targetNodeId: result.id,
        sourceHandle: 'source',
        targetHandle: 'target',
      };
      if (!state.document.connections.some((edge) =>
        edge.sourceNodeId === connection.sourceNodeId && edge.targetNodeId === connection.targetNodeId
      )) {
        const validation = validateCanvasConnection(state.document, connection);
        if (!validation.ok) {
          throw new Error(creativeStudioProductText(
            'creativeStudio.canvas.errors.audio.connectResultFailed',
            '无法连接音频创作结果：{{code}}。',
            { code: validation.code }
          ));
        }
        input.editor.dispatch(canvasCommands.connect(initialConfig.id, result.id, { at, mergeKey, select: false }));
      }
    }
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
        errorMessage: creativeStudioProductText(
          'creativeStudio.canvas.errors.taskMissingRecoveryCleared',
          '服务器未找到该任务；已确认清理恢复标记。'
        ),
      },
    })
  );
  await input.editor.removePendingTask(input.reference.taskId);
}
