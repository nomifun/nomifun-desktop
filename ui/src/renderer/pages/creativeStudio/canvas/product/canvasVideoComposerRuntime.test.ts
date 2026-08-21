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
  CanvasVideoComposerAssetPort,
  CanvasVideoComposerEditorPort,
} from './canvasVideoComposerRuntime';
import {
  orphanCanvasVideoComposeTask,
  persistCanvasVideoComposePendingTask,
  reconcileCanvasVideoComposeTask,
  settleCanvasVideoComposeTask,
} from './canvasVideoComposerRuntime';

const PROJECT_ID = testUuid(500);
const PROVIDER_ID = testUuid(501);
const TASK_ID = testUuid(502);
const INPUT_IMAGE_ASSET_ID = testUuid(503);
const RESULT_ASSET_ID = testUuid(504);
const SECOND_RESULT_ASSET_ID = testUuid(505);
const POSTER_ASSET_ID = testUuid(506);

type VideoNode = Extract<CreativeCanvasNode, { type: 'video' }>;
type ConfigNode = Extract<CreativeCanvasNode, { type: 'config' }>;

const sourceNode = (): VideoNode => {
  const base = testNode('video', 1, { width: 420, height: 236 });
  return {
    ...base,
    data: {
      ...base.data,
      posterAssetId: POSTER_ASSET_ID,
      autoplay: true,
      loop: true,
      muted: true,
      trimStartMs: 120,
      trimEndMs: 4_200,
      composer: {
        prompt: '清晨湖面的慢镜头',
        model: { providerId: PROVIDER_ID, model: 'video-v1' },
        resolution: '1080p',
        aspectRatio: '16:9',
        seconds: 5,
      },
    },
  };
};

const configNode = (
  options: {
    sourceAssetId?: string | null;
    inputAssetIds?: string[];
  } = {}
): ConfigNode => {
  const source = sourceNode();
  const base = testNode('config', 2, {
    x: 460,
    width: 440,
    height: 240,
    locked: true,
  });
  return {
    ...base,
    data: {
      ...base.data,
      task: 'video_generation',
      capability: 'i2v',
      providerId: PROVIDER_ID,
      model: 'video-v1',
      prompt: '让参考图自然动起来',
      operation: {
        kind: 'video-node-compose',
        sourceNodeId: source.id,
        sourceAssetId: options.sourceAssetId ?? null,
      },
      parameters: {
        resolution: '1080p',
        aspect: '16:9',
        seconds: 5,
      },
      inputAssetIds: options.inputAssetIds ?? [INPUT_IMAGE_ASSET_ID],
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
  model: 'video-v1',
  task: 'video_generation',
  capability: 'i2v',
});

const task = (
  status: CreativeTask['status'],
  node = configNode(),
  resultAssetIds = status === 'succeeded' ? [RESULT_ASSET_ID] : []
): CreativeTask => ({
  ...reference(node),
  parameters: {
    resolution: '1080p',
    aspect: '16:9',
    seconds: 5,
  },
  status,
  error:
    status === 'failed' ? { kind: 'provider', message: 'provider failed', httpStatus: 500 } : null,
  resultAssetIds,
  attempt: 1,
  submittedAt: 1,
  startedAt: status === 'queued' ? null : 2,
  finishedAt: status === 'succeeded' || status === 'failed' || status === 'canceled' ? 3 : null,
});

const videoAsset = (id: string, title: string): CreativeAsset => ({
  id,
  kind: 'video',
  title,
  collection: null,
  tags: [],
  mimeType: 'video/mp4',
  width: 1920,
  height: 1080,
  bytes: 1_024,
  inLibrary: true,
  textContent: null,
  origin: null,
  originalUrl: `/api/creative-studio/files/${id}`,
  thumbnailUrl: null,
  createdAt: 1,
  updatedAt: 1,
});

const RESULT_ASSET = videoAsset(RESULT_ASSET_ID, 'Generated video');
const SECOND_RESULT_ASSET = videoAsset(SECOND_RESULT_ASSET_ID, 'Second generated video');

const assetPort = (
  values: readonly CreativeAsset[] = [RESULT_ASSET, SECOND_RESULT_ASSET]
): CanvasVideoComposerAssetPort => ({
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
          id: testUuid(507),
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
  const editor: CanvasVideoComposerEditorPort = {
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

const expectRejected = async (promise: Promise<unknown>, message?: string): Promise<void> => {
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

describe('canvas video composer runtime integration', () => {
  test('flushes the exact owner before POST and reconciles running without history', async () => {
    const config = configNode();
    const harness = editorHarness(config, sourceNode(), []);

    await persistCanvasVideoComposePendingTask({
      editor: harness.editor,
      projectId: PROJECT_ID,
      reference: reference(config),
    });
    expect(harness.events.at(-1)).toBe(`pending:add:${TASK_ID}`);
    expect(harness.pending()).toEqual([TASK_ID]);

    reconcileCanvasVideoComposeTask({
      editor: harness.editor,
      projectId: PROJECT_ID,
      task: task('running', config),
    });
    expect(harness.events.at(-1)).toBe('node/reconcile-runtime');
    expect(harness.state().document.nodes.find((node) => node.id === config.id)).toMatchObject({
      locked: true,
      data: { status: 'running' },
    });

    await expectRejected(
      persistCanvasVideoComposePendingTask({
        editor: harness.editor,
        projectId: PROJECT_ID,
        reference: { ...reference(config), model: 'wrong-model' },
      }),
      '身份不一致'
    );
    expect(harness.events.at(-1)).toBe('node/reconcile-runtime');
  });

  test('fills the empty source, derives extra results, and removes pending last', async () => {
    const source = sourceNode();
    const config = configNode();
    const harness = editorHarness(config, source);
    const seenAssets: string[] = [];

    await settleCanvasVideoComposeTask({
      editor: harness.editor,
      projectId: PROJECT_ID,
      task: task('succeeded', config, [RESULT_ASSET_ID, SECOND_RESULT_ASSET_ID]),
      assets: assetPort(),
      viewportSize: { width: 1440, height: 900 },
      onAsset: (asset) => seenAssets.push(asset.id),
    });

    const state = harness.state();
    expect(state.document.nodes.find((node) => node.id === source.id)).toMatchObject({
      id: source.id,
      type: 'video',
      data: {
        assetId: RESULT_ASSET_ID,
        posterAssetId: POSTER_ASSET_ID,
        autoplay: true,
        loop: true,
        muted: true,
        trimStartMs: 120,
        trimEndMs: 4_200,
        composer: {
          prompt: '清晨湖面的慢镜头',
          model: null,
          resolution: '1080p',
          aspectRatio: '16:9',
          seconds: 5,
        },
      },
    });
    const derived = state.document.nodes.filter(
      (node) => node.type === 'video' && node.data.assetId === SECOND_RESULT_ASSET_ID
    );
    expect(derived).toHaveLength(1);
    expect(
      state.document.connections.some(
        (connection) =>
          connection.sourceNodeId === config.id && connection.targetNodeId === source.id
      )
    ).toBe(false);
    expect(
      state.document.connections.some(
        (connection) =>
          connection.sourceNodeId === config.id && connection.targetNodeId === derived[0]?.id
      )
    ).toBe(true);
    expect(state.selection.nodeIds).toEqual([source.id, derived[0]?.id]);
    expect(seenAssets).toEqual([RESULT_ASSET_ID, SECOND_RESULT_ASSET_ID]);
    expect(harness.pending()).toEqual([]);
    expect(harness.events.at(-1)).toBe(`pending:remove:${TASK_ID}`);
  });

  test('repeats terminal settlement without duplicate nodes or connections', async () => {
    const source = sourceNode();
    const config = configNode();
    const harness = editorHarness(config, source);
    const succeeded = task('succeeded', config, [RESULT_ASSET_ID, SECOND_RESULT_ASSET_ID]);
    const settle = () =>
      settleCanvasVideoComposeTask({
        editor: harness.editor,
        projectId: PROJECT_ID,
        task: succeeded,
        assets: assetPort(),
        viewportSize: { width: 1440, height: 900 },
      });

    await settle();
    await settle();

    expect(
      harness
        .state()
        .document.nodes.filter(
          (node) =>
            node.type === 'video' &&
            (node.data.assetId === RESULT_ASSET_ID || node.data.assetId === SECOND_RESULT_ASSET_ID)
        )
    ).toHaveLength(2);
    expect(
      harness
        .state()
        .document.connections.filter((connection) => connection.sourceNodeId === config.id)
    ).toHaveLength(1);
    expect(harness.events.at(-1)).toBe(`pending:remove:${TASK_ID}`);
  });

  test('refuses duplicate, reused-input, mismatched and wrong-kind results', async () => {
    const cases: Array<{
      name: string;
      config?: ConfigNode;
      resultIds: string[];
      assets?: CanvasVideoComposerAssetPort;
      message: string;
    }> = [
      {
        name: 'duplicate',
        resultIds: [RESULT_ASSET_ID, RESULT_ASSET_ID],
        message: '重复',
      },
      {
        name: 'reused input',
        resultIds: [INPUT_IMAGE_ASSET_ID],
        message: '复用了输入素材',
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
        message: '真实视频素材',
      },
      {
        name: 'wrong kind',
        resultIds: [RESULT_ASSET_ID],
        assets: assetPort([{ ...RESULT_ASSET, kind: 'image', mimeType: 'image/png' }]),
        message: '真实视频素材',
      },
    ];

    for (const entry of cases) {
      const config = entry.config ?? configNode();
      const harness = editorHarness(config);
      await expectRejected(
        settleCanvasVideoComposeTask({
          editor: harness.editor,
          projectId: PROJECT_ID,
          task: task('succeeded', config, entry.resultIds),
          assets: entry.assets ?? assetPort(),
          viewportSize: { width: 1440, height: 900 },
        }),
        entry.message
      );
      expect(harness.pending()).toEqual([TASK_ID]);
      expect(
        harness
          .state()
          .document.nodes.filter((node) => node.type === 'video' && node.data.assetId !== null)
      ).toHaveLength(0);
      expect(harness.events.at(-1)).not.toBe(`pending:remove:${TASK_ID}`);
    }
  });

  test('fails closed for future filled-source operations and nonterminal settlement', async () => {
    const futureConfig = configNode({ sourceAssetId: INPUT_IMAGE_ASSET_ID });
    const futureHarness = editorHarness(futureConfig);
    await expectRejected(
      settleCanvasVideoComposeTask({
        editor: futureHarness.editor,
        projectId: PROJECT_ID,
        task: task('succeeded', futureConfig),
        assets: assetPort(),
        viewportSize: { width: 1440, height: 900 },
      })
    );
    expect(futureHarness.pending()).toEqual([TASK_ID]);

    const config = configNode();
    const occupied = sourceNode();
    occupied.data.assetId = testUuid(509);
    const occupiedHarness = editorHarness(config, occupied);
    await expectRejected(
      settleCanvasVideoComposeTask({
        editor: occupiedHarness.editor,
        projectId: PROJECT_ID,
        task: task('succeeded', config),
        assets: assetPort(),
        viewportSize: { width: 1440, height: 900 },
      }),
      '已关联其他素材'
    );
    expect(occupiedHarness.pending()).toEqual([TASK_ID]);

    const queuedHarness = editorHarness(config);
    await expectRejected(
      settleCanvasVideoComposeTask({
        editor: queuedHarness.editor,
        projectId: PROJECT_ID,
        task: task('queued', config),
        assets: assetPort(),
        viewportSize: { width: 1440, height: 900 },
      }),
      '非终态'
    );
    expect(queuedHarness.pending()).toEqual([TASK_ID]);
  });

  test('retains failed and canceled configs before clearing pending', async () => {
    for (const status of ['failed', 'canceled'] as const) {
      const config = configNode();
      const harness = editorHarness(config);
      await settleCanvasVideoComposeTask({
        editor: harness.editor,
        projectId: PROJECT_ID,
        task: task(status, config),
        assets: assetPort(),
        viewportSize: { width: 1440, height: 900 },
      });
      expect(harness.state().document.nodes.find((node) => node.id === config.id)).toMatchObject({
        locked: false,
        data: {
          status,
          resultAssetIds: [],
          errorMessage: status === 'failed' ? 'provider failed' : '视频创作已取消。',
        },
      });
      expect(harness.events.at(-1)).toBe(`pending:remove:${TASK_ID}`);
      expect(harness.pending()).toEqual([]);
    }
  });

  test('confirmed orphan retains a failed config and removes only its pending id', async () => {
    const otherTaskId = testUuid(508);
    const config = configNode();
    const harness = editorHarness(config, sourceNode(), [TASK_ID, otherTaskId]);

    await orphanCanvasVideoComposeTask({
      editor: harness.editor,
      projectId: PROJECT_ID,
      reference: reference(config),
    });

    expect(harness.pending()).toEqual([otherTaskId]);
    expect(harness.state().document.nodes.find((node) => node.id === config.id)).toMatchObject({
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
