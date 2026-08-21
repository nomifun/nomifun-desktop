/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { describe, expect, test } from 'bun:test';

import type { CreativeAsset } from '../../assets';
import type { CreativeCanvasNode } from '../../domain';
import type { CreativeTask, CreativeTaskReference } from '../../tasks';
import {
  canvasReducer,
  createInitialCanvasState,
  type CanvasCommand,
  type CanvasState,
} from '../core';
import { testNode, testUuid } from '../core/testFixtures';
import type {
  CanvasAudioComposerAssetPort,
  CanvasAudioComposerEditorPort,
} from './canvasAudioComposerRuntime';
import {
  orphanCanvasAudioComposeTask,
  persistCanvasAudioComposePendingTask,
  reconcileCanvasAudioComposeTask,
  settleCanvasAudioComposeTask,
} from './canvasAudioComposerRuntime';

const PROJECT_ID = testUuid(610);
const PROVIDER_ID = testUuid(611);
const TASK_ID = testUuid(612);
const RESULT_ASSET_ID = testUuid(613);
const SECOND_RESULT_ASSET_ID = testUuid(614);

type AudioNode = Extract<CreativeCanvasNode, { type: 'audio' }>;
type ConfigNode = Extract<CreativeCanvasNode, { type: 'config' }>;

const sourceNode = (): AudioNode => {
  const base = testNode('audio', 1, { width: 340, height: 160 });
  return {
    ...base,
    data: {
      ...base.data,
      title: '未生成的旁白',
      loop: true,
      volume: 0.65,
      trimStartMs: 120,
      trimEndMs: 4_200,
      composer: {
        prompt: '一段温暖的产品旁白',
        model: { providerId: PROVIDER_ID, model: 'audio-v1' },
        voice: 'nova',
        format: 'wav',
      },
    },
  };
};

const configNode = (inputAssetIds: string[] = []): ConfigNode => {
  const source = sourceNode();
  const base = testNode('config', 2, {
    x: 420,
    width: 440,
    height: 240,
    locked: true,
  });
  return {
    ...base,
    data: {
      ...base.data,
      task: 'speech_synthesis',
      capability: 'tts',
      providerId: PROVIDER_ID,
      model: 'audio-v1',
      prompt: '一段温暖的产品旁白',
      operation: {
        kind: 'audio-node-compose',
        sourceNodeId: source.id,
        sourceAssetId: null,
      },
      parameters: {
        prompt: '一段温暖的产品旁白',
        voice: 'nova',
        format: 'wav',
      },
      inputAssetIds,
      taskId: TASK_ID,
      resultAssetIds: [],
      status: 'queued',
      errorMessage: null,
    },
  };
};

const reference = (node = configNode()): CreativeTaskReference => ({
  taskId: TASK_ID,
  owner: { kind: 'canvas_node', projectId: PROJECT_ID, nodeId: node.id },
  providerId: PROVIDER_ID,
  model: 'audio-v1',
  task: 'speech_synthesis',
  capability: 'tts',
});

const task = (
  status: CreativeTask['status'],
  node = configNode(),
  resultAssetIds = status === 'succeeded' ? [RESULT_ASSET_ID] : []
): CreativeTask => ({
  ...reference(node),
  parameters: {
    prompt: '一段温暖的产品旁白',
    voice: 'nova',
    format: 'wav',
  },
  inputs: [],
  status,
  error:
    status === 'failed'
      ? { kind: 'provider', message: 'provider failed', httpStatus: 500 }
      : null,
  resultAssetIds,
  attempt: 1,
  submittedAt: 1,
  startedAt: status === 'queued' ? null : 2,
  finishedAt:
    status === 'succeeded' || status === 'failed' || status === 'canceled'
      ? 3
      : null,
});

const audioAsset = (id: string, title: string): CreativeAsset => ({
  id,
  kind: 'audio',
  title,
  collection: null,
  tags: [],
  mimeType: 'audio/wav',
  width: null,
  height: null,
  bytes: 1_024,
  inLibrary: true,
  textContent: null,
  origin: null,
  originalUrl: `/api/creative-studio/files/${id}`,
  thumbnailUrl: null,
  createdAt: 1,
  updatedAt: 1,
});

const RESULT_ASSET = audioAsset(RESULT_ASSET_ID, '生成的产品旁白');
const SECOND_RESULT_ASSET = audioAsset(
  SECOND_RESULT_ASSET_ID,
  '多余的音频结果'
);

const assetPort = (
  values: readonly CreativeAsset[] = [RESULT_ASSET]
): CanvasAudioComposerAssetPort => ({
  async get(assetId) {
    const asset = values.find((candidate) => candidate.id === assetId);
    if (!asset) throw new Error(`Unexpected asset ${assetId}`);
    return asset;
  },
  async list() {
    return { items: [...values], total: values.length };
  },
  async upload() {
    return values[0] as CreativeAsset;
  },
  async update() {
    return values[0] as CreativeAsset;
  },
  async remove() {},
  url: (assetId) => `/api/creative-studio/files/${assetId}`,
});

const editorHarness = (
  config = configNode(),
  source = sourceNode(),
  pendingTaskIds: string[] = [TASK_ID]
) => {
  let state: CanvasState = createInitialCanvasState({
    document: {
      nodes: [source, config],
      connections: [
        {
          id: testUuid(615),
          sourceNodeId: source.id,
          targetNodeId: config.id,
          sourceHandle: 'source',
          targetHandle: 'target',
        },
      ],
    },
  });
  let pending = [...pendingTaskIds];
  const events: string[] = [];
  const editor: CanvasAudioComposerEditorPort = {
    dispatch(command: CanvasCommand) {
      state = canvasReducer(state, command);
      events.push(command.type);
      return state;
    },
    getState: () => state,
    async addPendingTask(taskId) {
      pending = [...new Set([...pending, taskId])];
      events.push(`pending:add:${taskId}`);
    },
    async removePendingTask(taskId) {
      pending = pending.filter((candidate) => candidate !== taskId);
      events.push(`pending:remove:${taskId}`);
    },
  };
  return { editor, events, pending: () => pending, state: () => state };
};

const expectRejected = async (
  promise: Promise<unknown>,
  message?: string
): Promise<void> => {
  let rejection: unknown = null;
  try {
    await promise;
  } catch (error) {
    rejection = error;
  }
  if (!(rejection instanceof Error)) {
    throw new Error('预期 Promise 被拒绝且返回 Error。');
  }
  if (message) {
    expect(rejection.message.includes(message)).toBe(true);
  }
};

describe('canvas audio composer runtime integration', () => {
  test('flushes the exact owner before POST and reconciles running without history', async () => {
    const config = configNode();
    const harness = editorHarness(config, sourceNode(), []);

    await persistCanvasAudioComposePendingTask({
      editor: harness.editor,
      projectId: PROJECT_ID,
      reference: reference(config),
    });
    expect(harness.events.at(-1)).toBe(`pending:add:${TASK_ID}`);
    expect(harness.pending()).toEqual([TASK_ID]);

    reconcileCanvasAudioComposeTask({
      editor: harness.editor,
      projectId: PROJECT_ID,
      task: task('running', config),
    });
    expect(harness.events.at(-1)).toBe('node/reconcile-runtime');
    expect(
      harness.state().document.nodes.find((node) => node.id === config.id)
    ).toMatchObject({ locked: true, data: { status: 'running' } });

    await expectRejected(
      persistCanvasAudioComposePendingTask({
        editor: harness.editor,
        projectId: PROJECT_ID,
        reference: { ...reference(config), model: 'wrong-model' },
      }),
      '身份不一致'
    );
    expect(harness.events.at(-1)).toBe('node/reconcile-runtime');
  });

  test('fills the same audio node with one real result and removes pending last', async () => {
    const source = sourceNode();
    const config = configNode();
    const harness = editorHarness(config, source);
    const seenAssets: string[] = [];

    await settleCanvasAudioComposeTask({
      editor: harness.editor,
      projectId: PROJECT_ID,
      task: task('succeeded', config),
      assets: assetPort(),
      onAsset: (asset) => seenAssets.push(asset.id),
    });

    const state = harness.state();
    expect(state.document.nodes.find((node) => node.id === source.id)).toMatchObject({
      id: source.id,
      type: 'audio',
      data: {
        assetId: RESULT_ASSET_ID,
        title: '生成的产品旁白',
        loop: true,
        volume: 0.65,
        trimStartMs: 120,
        trimEndMs: 4_200,
        composer: {
          prompt: '一段温暖的产品旁白',
          model: null,
          voice: 'nova',
          format: 'wav',
        },
      },
    });
    expect(state.document.nodes).toHaveLength(2);
    expect(
      state.document.connections.some(
        (connection) =>
          connection.sourceNodeId === config.id &&
          connection.targetNodeId === source.id
      )
    ).toBe(false);
    expect(state.selection.nodeIds).toEqual([source.id]);
    expect(seenAssets).toEqual([RESULT_ASSET_ID]);
    expect(harness.pending()).toEqual([]);
    expect(harness.events.at(-1)).toBe(`pending:remove:${TASK_ID}`);
  });

  test('repeats terminal settlement without duplicate nodes or connections', async () => {
    const source = sourceNode();
    const config = configNode();
    const harness = editorHarness(config, source);
    const succeeded = task('succeeded', config);
    const settle = () =>
      settleCanvasAudioComposeTask({
        editor: harness.editor,
        projectId: PROJECT_ID,
        task: succeeded,
        assets: assetPort(),
      });

    await settle();
    await settle();

    expect(harness.state().document.nodes).toHaveLength(2);
    expect(harness.state().document.connections).toHaveLength(1);
    expect(
      harness.state().document.nodes.filter(
        (node) => node.type === 'audio' && node.data.assetId === RESULT_ASSET_ID
      )
    ).toHaveLength(1);
    expect(harness.events.at(-1)).toBe(`pending:remove:${TASK_ID}`);
  });

  test('refuses missing, multiple, mismatched, wrong-kind and occupied results', async () => {
    const cases: Array<{
      name: string;
      resultIds: string[];
      assets?: CanvasAudioComposerAssetPort;
      source?: AudioNode;
      message: string;
    }> = [
      { name: 'missing', resultIds: [], message: '恰好返回一个' },
      {
        name: 'multiple',
        resultIds: [RESULT_ASSET_ID, SECOND_RESULT_ASSET_ID],
        message: '恰好返回一个',
      },
      {
        name: 'asset id mismatch',
        resultIds: [RESULT_ASSET_ID],
        assets: {
          ...assetPort(),
          async get() {
            return SECOND_RESULT_ASSET;
          },
        },
        message: '真实音频素材',
      },
      {
        name: 'wrong kind',
        resultIds: [RESULT_ASSET_ID],
        assets: assetPort([
          { ...RESULT_ASSET, kind: 'video', mimeType: 'video/mp4' },
        ]),
        message: '真实音频素材',
      },
      {
        name: 'occupied source',
        resultIds: [RESULT_ASSET_ID],
        source: {
          ...sourceNode(),
          data: { ...sourceNode().data, assetId: SECOND_RESULT_ASSET_ID },
        },
        message: '已关联其他素材',
      },
    ];

    for (const entry of cases) {
      const config = configNode();
      const harness = editorHarness(config, entry.source ?? sourceNode());
      await expectRejected(
        settleCanvasAudioComposeTask({
          editor: harness.editor,
          projectId: PROJECT_ID,
          task: task('succeeded', config, entry.resultIds),
          assets: entry.assets ?? assetPort(),
        }),
        entry.message
      );
      expect(harness.pending()).toEqual([TASK_ID]);
      expect(harness.events.at(-1)).not.toBe(`pending:remove:${TASK_ID}`);
    }
  });

  test('fails closed for referenced configs and nonterminal settlement', async () => {
    const referencedConfig = configNode([testUuid(616)]);
    const referencedHarness = editorHarness(referencedConfig);
    await expectRejected(
      settleCanvasAudioComposeTask({
        editor: referencedHarness.editor,
        projectId: PROJECT_ID,
        task: task('succeeded', referencedConfig),
        assets: assetPort(),
      })
    );
    expect(referencedHarness.pending()).toEqual([TASK_ID]);

    const config = configNode();
    const queuedHarness = editorHarness(config);
    await expectRejected(
      settleCanvasAudioComposeTask({
        editor: queuedHarness.editor,
        projectId: PROJECT_ID,
        task: task('queued', config),
        assets: assetPort(),
      }),
      '非终态'
    );
    expect(queuedHarness.pending()).toEqual([TASK_ID]);
  });

  test('retains failed and canceled configs before clearing pending', async () => {
    for (const status of ['failed', 'canceled'] as const) {
      const config = configNode();
      const harness = editorHarness(config);
      await settleCanvasAudioComposeTask({
        editor: harness.editor,
        projectId: PROJECT_ID,
        task: task(status, config),
        assets: assetPort(),
      });
      expect(
        harness.state().document.nodes.find((node) => node.id === config.id)
      ).toMatchObject({
        locked: false,
        data: {
          status,
          resultAssetIds: [],
          errorMessage:
            status === 'failed' ? 'provider failed' : '音频创作已取消。',
        },
      });
      expect(harness.events.at(-1)).toBe(`pending:remove:${TASK_ID}`);
      expect(harness.pending()).toEqual([]);
    }
  });

  test('confirmed orphan retains a failed config and removes only its pending id', async () => {
    const otherTaskId = testUuid(617);
    const config = configNode();
    const harness = editorHarness(config, sourceNode(), [TASK_ID, otherTaskId]);

    await orphanCanvasAudioComposeTask({
      editor: harness.editor,
      projectId: PROJECT_ID,
      reference: reference(config),
    });

    expect(harness.pending()).toEqual([otherTaskId]);
    expect(
      harness.state().document.nodes.find((node) => node.id === config.id)
    ).toMatchObject({
      locked: false,
      data: {
        status: 'failed',
        resultAssetIds: [],
        errorMessage: '服务器未找到该任务；已确认清理恢复标记。',
      },
    });
    expect(harness.events.at(-1)).toBe(`pending:remove:${TASK_ID}`);
  });
});
