/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { describe, expect, test } from 'bun:test';

import type { CreativeAsset } from '../../assets';
import type { CreativeCanvasNode } from '../../domain';
import type { CreativeTask, CreativeTaskReference } from '../../tasks';
import type {
  CreativeWorkbenchRuntimeListener,
  CreativeWorkbenchRuntimeSnapshot,
} from '../../workbenches/runtime';
import {
  canvasReducer,
  createInitialCanvasState,
  type CanvasCommand,
  type CanvasState,
} from '../core';
import { testNode, testUuid } from '../core/testFixtures';
import type { CanvasImageMaskEditEditorPort } from './imageMaskEditRuntime';
import {
  orphanCanvasImageMaskEditTask,
  persistCanvasImageMaskEditPendingTask,
  settleCanvasImageMaskEditTask,
  waitForCanvasImageMaskEditAdmission,
} from './imageMaskEditRuntime';

const PROJECT_ID = testUuid(100);
const PROVIDER_ID = testUuid(101);
const TASK_ID = testUuid(102);
const SOURCE_ASSET_ID = testUuid(103);
const MARKED_ASSET_ID = testUuid(104);
const RESULT_ASSET_ID = testUuid(105);

const configNode = (): Extract<CreativeCanvasNode, { type: 'config' }> => {
  const base = testNode('config', 1, {
    x: 400,
    y: 0,
    width: 340,
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
      operation: {
        kind: 'image-mask-edit',
        sourceNodeId: testUuid(106),
        sourceAssetId: SOURCE_ASSET_ID,
        markedReferenceAssetId: MARKED_ASSET_ID,
      },
      parameters: { prompt: 'masked edit' },
      inputAssetIds: [MARKED_ASSET_ID],
      taskId: TASK_ID,
      status: 'running',
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
  parameters: {},
  status,
  error:
    status === 'failed'
      ? { kind: 'provider', message: 'provider failed', httpStatus: 500 }
      : null,
  resultAssetIds: status === 'succeeded' ? [RESULT_ASSET_ID] : [],
  attempt: 1,
  submittedAt: 1,
  startedAt: 2,
  finishedAt: status === 'queued' || status === 'running' ? null : 3,
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
  originalUrl: `/api/creative-studio/assets/${RESULT_ASSET_ID}/file`,
  thumbnailUrl: null,
  createdAt: 1,
  updatedAt: 1,
};

const editorHarness = (node = configNode()) => {
  let state: CanvasState = createInitialCanvasState({
    document: { nodes: [node], connections: [] },
  });
  let pending = [TASK_ID];
  const events: string[] = [];
  const editor: CanvasImageMaskEditEditorPort = {
    dispatch(command: CanvasCommand) {
      state = canvasReducer(state, command);
      events.push(command.type);
      return state;
    },
    getState: () => state,
    getPendingTaskIds: () => [...pending],
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

const IDLE_SNAPSHOT: CreativeWorkbenchRuntimeSnapshot = {
  state: 'idle',
  entries: [],
  submissionFailures: [],
  submittingCount: 0,
  recoveringCount: 0,
  requestError: null,
};

describe('canvas image mask edit runtime integration', () => {
  test('flushes the exact config owner before task submission', async () => {
    const node = configNode();
    const harness = editorHarness(node);
    await persistCanvasImageMaskEditPendingTask({
      editor: harness.editor,
      projectId: PROJECT_ID,
      reference: reference(node),
    });
    expect(harness.pending()).toEqual([TASK_ID]);
    expect(harness.events.at(-1)).toBe(`pending:add:${TASK_ID}`);
  });

  test('settles a successful task idempotently and removes pending last', async () => {
    const node = configNode();
    const harness = editorHarness(node);
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
    const succeeded = task('succeeded', node);

    await settleCanvasImageMaskEditTask({
      editor: harness.editor,
      projectId: PROJECT_ID,
      task: succeeded,
      assets,
      viewportSize: { width: 1440, height: 900 },
    });
    const first = harness.state();
    const resultNodes = first.document.nodes.filter(
      (candidate) =>
        candidate.type === 'image' && candidate.data.assetId === RESULT_ASSET_ID
    );
    expect(resultNodes).toHaveLength(1);
    expect(first.document.connections).toHaveLength(1);
    expect(first.document.connections[0]).toMatchObject({
      sourceNodeId: node.id,
      targetNodeId: resultNodes[0]?.id,
    });
    expect(
      first.document.nodes.find((candidate) => candidate.id === node.id)
    ).toMatchObject({ locked: false, data: { status: 'succeeded' } });
    expect(harness.pending()).toEqual([]);
    expect(harness.events.at(-1)).toBe(`pending:remove:${TASK_ID}`);

    await settleCanvasImageMaskEditTask({
      editor: harness.editor,
      projectId: PROJECT_ID,
      task: succeeded,
      assets,
      viewportSize: { width: 1440, height: 900 },
    });
    expect(
      harness
        .state()
        .document.nodes.filter(
          (candidate) =>
            candidate.type === 'image' &&
            candidate.data.assetId === RESULT_ASSET_ID
        )
    ).toHaveLength(1);
    expect(harness.state().document.connections).toHaveLength(1);
  });

  test('cleans only a confirmed orphan reference and retains its failed config', async () => {
    const node = configNode();
    const harness = editorHarness(node);
    await orphanCanvasImageMaskEditTask({
      editor: harness.editor,
      projectId: PROJECT_ID,
      reference: reference(node),
    });
    expect(harness.pending()).toEqual([]);
    expect(harness.state().document.nodes[0]).toMatchObject({
      locked: false,
      data: { status: 'failed', resultAssetIds: [] },
    });
  });

  test('resolves admission independently from the long-running worker promise', async () => {
    const listeners = new Set<CreativeWorkbenchRuntimeListener>();
    let snapshot = IDLE_SNAPSHOT;
    const controller = {
      subscribe(listener: CreativeWorkbenchRuntimeListener) {
        listeners.add(listener);
        listener(snapshot);
        return () => listeners.delete(listener);
      },
    };
    const worker = {
      finish: null as
        ((value: CreativeWorkbenchRuntimeSnapshot) => void) | null,
    };
    const admission = waitForCanvasImageMaskEditAdmission({
      controller,
      idempotencyKey: TASK_ID,
      start: () =>
        new Promise((resolve) => {
          worker.finish = resolve;
          queueMicrotask(() => {
            snapshot = {
              ...IDLE_SNAPSHOT,
              state: 'running',
              entries: [
                {
                  order: 0,
                  task: task('running'),
                  outputs: [],
                  requestError: null,
                  retryInput: null,
                  outputKind: 'image',
                },
              ],
            };
            for (const listener of listeners) listener(snapshot);
          });
        }),
    });
    expect(await admission).toEqual({ kind: 'admitted', taskId: TASK_ID });
    expect(worker.finish).not.toBeNull();
    worker.finish?.(snapshot);
  });

  test('returns the exact retry slot when submission outcome is unresolved', async () => {
    const listeners = new Set<CreativeWorkbenchRuntimeListener>();
    const controller = {
      subscribe(listener: CreativeWorkbenchRuntimeListener) {
        listeners.add(listener);
        listener(IDLE_SNAPSHOT);
        return () => listeners.delete(listener);
      },
    };
    const failure = new Error('response lost');
    const input = {
      idempotencyKey: TASK_ID,
      owner: reference().owner,
      providerId: PROVIDER_ID,
      model: 'edit-v1',
      task: 'image_edit' as const,
      capability: 'i2i' as const,
      parameters: {},
      inputs: [{ assetId: MARKED_ASSET_ID, role: 'reference' as const }],
    };
    const result = waitForCanvasImageMaskEditAdmission({
      controller,
      idempotencyKey: TASK_ID,
      start: async () => {
        queueMicrotask(() => {
          const snapshot: CreativeWorkbenchRuntimeSnapshot = {
            ...IDLE_SNAPSHOT,
            state: 'request_error',
            requestError: failure,
            submissionFailures: [
              { order: 3, input, outputKind: 'image', error: failure },
            ],
          };
          for (const listener of listeners) listener(snapshot);
        });
        return IDLE_SNAPSHOT;
      },
    });
    expect(await result).toEqual({
      kind: 'submission_failure',
      order: 3,
      error: failure,
    });
  });
});
