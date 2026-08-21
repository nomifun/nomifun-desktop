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
import type { CanvasImageComposerEditorPort } from './canvasImageComposerRuntime';
import {
  orphanCanvasImageComposeTask,
  persistCanvasImageComposePendingTask,
  reconcileCanvasImageComposeTask,
  settleCanvasImageComposeTask,
} from './canvasImageComposerRuntime';

const PROJECT_ID = testUuid(400);
const PROVIDER_ID = testUuid(401);
const TASK_ID = testUuid(402);
const SOURCE_ASSET_ID = testUuid(403);
const RESULT_ASSET_ID = testUuid(404);
const SECOND_RESULT_ASSET_ID = testUuid(406);

const sourceNode = (): Extract<CreativeCanvasNode, { type: 'image' }> => {
  const base = testNode('image', 1, { width: 340, height: 240 });
  return { ...base, data: { ...base.data, assetId: SOURCE_ASSET_ID } };
};

const emptySourceNode = (): Extract<CreativeCanvasNode, { type: 'image' }> => {
  const base = testNode('image', 1, { width: 340, height: 240 });
  return {
    ...base,
    data: {
      ...base.data,
      composer: {
        prompt: '生成后继续编辑',
        model: { providerId: PROVIDER_ID, model: 'generate-v1' },
        interfaceMode: 'images',
        quality: 'auto',
        width: 1024,
        height: 1024,
        aspectRatio: '1:1',
        count: 2,
      },
    },
  };
};

const configNode = (): Extract<CreativeCanvasNode, { type: 'config' }> => {
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
      task: 'image_edit',
      capability: 'i2i',
      providerId: PROVIDER_ID,
      model: 'edit-v1',
      prompt: '改成清晨',
      operation: {
        kind: 'image-node-compose',
        sourceNodeId: sourceNode().id,
        sourceAssetId: SOURCE_ASSET_ID,
      },
      parameters: { prompt: '改成清晨' },
      inputAssetIds: [SOURCE_ASSET_ID],
      taskId: TASK_ID,
      status: 'queued',
    },
  };
};

const generationConfigNode = (): Extract<CreativeCanvasNode, { type: 'config' }> => {
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
      task: 'image_generation',
      capability: 't2i',
      providerId: PROVIDER_ID,
      model: 'generate-v1',
      prompt: '清晨湖面上的薄雾',
      operation: {
        kind: 'image-node-compose',
        sourceNodeId: emptySourceNode().id,
        sourceAssetId: null,
      },
      parameters: { prompt: '清晨湖面上的薄雾' },
      inputAssetIds: [],
      taskId: TASK_ID,
      status: 'queued',
    },
  };
};

const reference = (node = configNode()): CreativeTaskReference => ({
  taskId: TASK_ID,
  owner: { kind: 'canvas_node', projectId: PROJECT_ID, nodeId: node.id },
  providerId: PROVIDER_ID,
  model: 'edit-v1',
  task: 'image_edit',
  capability: 'i2i',
});

const task = (
  status: CreativeTask['status'],
  node = configNode()
): CreativeTask => ({
  ...reference(node),
  parameters: { prompt: '改成清晨' },
  status,
  error:
    status === 'failed'
      ? { kind: 'provider', message: 'provider failed', httpStatus: 500 }
      : null,
  resultAssetIds: status === 'succeeded' ? [RESULT_ASSET_ID] : [],
  attempt: 1,
  submittedAt: 1,
  startedAt: status === 'queued' ? null : 2,
  finishedAt:
    status === 'succeeded' || status === 'failed' || status === 'canceled'
      ? 3
      : null,
});

const generationTask = (
  node = generationConfigNode()
): CreativeTask => ({
  taskId: TASK_ID,
  owner: { kind: 'canvas_node', projectId: PROJECT_ID, nodeId: node.id },
  providerId: PROVIDER_ID,
  model: 'generate-v1',
  task: 'image_generation',
  capability: 't2i',
  parameters: { prompt: '清晨湖面上的薄雾' },
  status: 'succeeded',
  error: null,
  resultAssetIds: [RESULT_ASSET_ID, SECOND_RESULT_ASSET_ID],
  attempt: 1,
  submittedAt: 1,
  startedAt: 2,
  finishedAt: 3,
});

const resultAsset: CreativeAsset = {
  id: RESULT_ASSET_ID,
  kind: 'image',
  title: 'Edited image',
  collection: null,
  tags: [],
  mimeType: 'image/png',
  width: 1024,
  height: 1024,
  bytes: 100,
  inLibrary: true,
  textContent: null,
  origin: null,
  originalUrl: `/api/creative-studio/files/${RESULT_ASSET_ID}`,
  thumbnailUrl: null,
  createdAt: 1,
  updatedAt: 1,
};

const secondResultAsset: CreativeAsset = {
  ...resultAsset,
  id: SECOND_RESULT_ASSET_ID,
  title: 'Second generated image',
  originalUrl: `/api/creative-studio/files/${SECOND_RESULT_ASSET_ID}`,
};

const editorHarness = (
  config = configNode(),
  source: Extract<CreativeCanvasNode, { type: 'image' }> = sourceNode()
) => {
  let state: CanvasState = createInitialCanvasState({
    document: {
      nodes: [source, config],
      connections: [
        {
          id: testUuid(405),
          sourceNodeId: source.id,
          targetNodeId: config.id,
          sourceHandle: 'source',
          targetHandle: 'target',
        },
      ],
    },
  });
  let pending = [TASK_ID];
  const events: string[] = [];
  const editor: CanvasImageComposerEditorPort = {
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

const assets = {
  async get(assetId: string) {
    expect(assetId).toBe(RESULT_ASSET_ID);
    return resultAsset;
  },
  async list() {
    return { items: [resultAsset], total: 1 };
  },
  async upload() {
    return resultAsset;
  },
  async update() {
    return resultAsset;
  },
  async remove() {},
  url: () => resultAsset.originalUrl,
};

describe('canvas image composer runtime integration', () => {
  test('flushes exact owner before submission and reflects running state', async () => {
    const config = configNode();
    const harness = editorHarness(config);
    await persistCanvasImageComposePendingTask({
      editor: harness.editor,
      projectId: PROJECT_ID,
      reference: reference(config),
    });
    expect(harness.events.at(-1)).toBe(`pending:add:${TASK_ID}`);

    reconcileCanvasImageComposeTask({
      editor: harness.editor,
      projectId: PROJECT_ID,
      task: task('running', config),
    });
    expect(harness.state().document.nodes.find((node) => node.id === config.id)).toMatchObject({
      locked: true,
      data: { status: 'running' },
    });
  });

  test('settles real output idempotently and removes pending last', async () => {
    const config = configNode();
    const harness = editorHarness(config);
    const succeeded = task('succeeded', config);
    await settleCanvasImageComposeTask({
      editor: harness.editor,
      projectId: PROJECT_ID,
      task: succeeded,
      assets,
      viewportSize: { width: 1440, height: 900 },
    });
    const resultNodes = harness.state().document.nodes.filter(
      (node) => node.type === 'image' && node.data.assetId === RESULT_ASSET_ID
    );
    expect(resultNodes).toHaveLength(1);
    expect(
      harness.state().document.connections.some(
        (connection) =>
          connection.sourceNodeId === config.id &&
          connection.targetNodeId === resultNodes[0]?.id
      )
    ).toBe(true);
    expect(harness.pending()).toEqual([]);
    expect(harness.events.at(-1)).toBe(`pending:remove:${TASK_ID}`);

    await settleCanvasImageComposeTask({
      editor: harness.editor,
      projectId: PROJECT_ID,
      task: succeeded,
      assets,
      viewportSize: { width: 1440, height: 900 },
    });
    expect(
      harness.state().document.nodes.filter(
        (node) => node.type === 'image' && node.data.assetId === RESULT_ASSET_ID
      )
    ).toHaveLength(1);
    expect(
      harness.state().document.connections.filter(
        (connection) => connection.sourceNodeId === config.id
      )
    ).toHaveLength(1);
  });

  test('fills an empty source with the first t2i result and links only extra results', async () => {
    const source = emptySourceNode();
    const config = generationConfigNode();
    const harness = editorHarness(config, source);
    const generated = generationTask(config);
    const generationAssets = {
      ...assets,
      async get(assetId: string) {
        if (assetId === RESULT_ASSET_ID) return resultAsset;
        if (assetId === SECOND_RESULT_ASSET_ID) return secondResultAsset;
        throw new Error(`Unexpected asset ${assetId}`);
      },
    };

    await settleCanvasImageComposeTask({
      editor: harness.editor,
      projectId: PROJECT_ID,
      task: generated,
      assets: generationAssets,
      viewportSize: { width: 1440, height: 900 },
    });

    const state = harness.state();
    expect(state.document.nodes.find((node) => node.id === source.id)).toMatchObject({
      id: source.id,
      type: 'image',
      data: {
        assetId: RESULT_ASSET_ID,
        composer: {
          prompt: '生成后继续编辑',
          model: null,
          count: 2,
        },
      },
    });
    const extraNodes = state.document.nodes.filter(
      (node) => node.type === 'image' && node.data.assetId === SECOND_RESULT_ASSET_ID
    );
    expect(extraNodes).toHaveLength(1);
    expect(
      state.document.connections.some(
        (connection) =>
          connection.sourceNodeId === config.id &&
          connection.targetNodeId === source.id
      )
    ).toBe(false);
    expect(
      state.document.connections.some(
        (connection) =>
          connection.sourceNodeId === config.id &&
          connection.targetNodeId === extraNodes[0]?.id
      )
    ).toBe(true);
    expect(harness.pending()).toEqual([]);
    expect(harness.events.at(-1)).toBe(`pending:remove:${TASK_ID}`);

    await settleCanvasImageComposeTask({
      editor: harness.editor,
      projectId: PROJECT_ID,
      task: generated,
      assets: generationAssets,
      viewportSize: { width: 1440, height: 900 },
    });
    expect(
      harness.state().document.nodes.filter(
        (node) =>
          node.type === 'image' &&
          (node.data.assetId === RESULT_ASSET_ID ||
            node.data.assetId === SECOND_RESULT_ASSET_ID)
      )
    ).toHaveLength(2);
    expect(
      harness.state().document.connections.filter(
        (connection) => connection.sourceNodeId === config.id
      )
    ).toHaveLength(1);
  });

  test('confirmed orphan clears only its pending id and retains failed config', async () => {
    const config = configNode();
    const harness = editorHarness(config);
    await orphanCanvasImageComposeTask({
      editor: harness.editor,
      projectId: PROJECT_ID,
      reference: reference(config),
    });
    expect(harness.pending()).toEqual([]);
    expect(harness.state().document.nodes.find((node) => node.id === config.id)).toMatchObject({
      locked: false,
      data: { status: 'failed', resultAssetIds: [] },
    });
  });
});
