/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { describe, expect, test } from 'bun:test';

import { BackendHttpError } from '@/common/adapter/httpBridge';

import { createEmptyCreativeProjectDocument } from '../domain/schema';
import type { CreativeProjectDocument } from '../domain/schema';
import type { CreativeTaskPort } from './port';
import {
  CreativeTaskPollTimeoutError,
  CreativeTaskProgressGuard,
  CreativeTaskRequestFence,
  pendingCreativeTaskReferences,
  pollCreativeTask,
  projectCreativeTaskOutput,
  recoverPendingCreativeTasks,
} from './runtime';
import { CreativeTaskContractError } from './types';
import type {
  CreativeTask,
  CreativeTaskIdentity,
  CreativeTaskReference,
  CreativeTaskStatus,
} from './types';

const PROJECT_ID = '0190f5fe-7c00-7a00-8000-000000000011';
const NODE_ID = '0190f5fe-7c00-7a00-8000-000000000012';
const PROVIDER_ID = '0190f5fe-7c00-7a00-8000-000000000013';
const TASK_ID = '0190f5fe-7c00-7a00-8000-000000000014';
const SECOND_TASK_ID = '0190f5fe-7c00-7a00-8000-000000000015';
const ASSET_ID = '0190f5fe-7c00-7a00-8000-000000000016';

const identity: CreativeTaskIdentity = {
  projectId: PROJECT_ID,
  nodeId: NODE_ID,
  providerId: PROVIDER_ID,
  model: 'image-model-v1',
  task: 'image_generation',
  capability: 't2i',
};
const reference: CreativeTaskReference = { taskId: TASK_ID, ...identity };

function task(
  status: CreativeTaskStatus,
  overrides: Partial<CreativeTask> = {}
): CreativeTask {
  const terminal = status === 'succeeded' || status === 'failed' || status === 'canceled';
  return {
    taskId: TASK_ID,
    ...identity,
    parameters: { prompt: 'Aurora' },
    status,
    error: status === 'failed' ? { kind: 'provider_error', message: 'failed', httpStatus: null } : null,
    resultAssetIds: status === 'succeeded' ? [ASSET_ID] : [],
    attempt: status === 'queued' ? 0 : 1,
    submittedAt: 100,
    startedAt: status === 'queued' ? null : 110,
    finishedAt: terminal ? 150 : null,
    ...overrides,
  };
}

function portWithGet(get: CreativeTaskPort['get']): CreativeTaskPort {
  return {
    create: async () => task('queued'),
    get,
    cancel: async () => task('canceled'),
  };
}

async function caught(promise: Promise<unknown>): Promise<unknown> {
  try {
    await promise;
    return null;
  } catch (error) {
    return error;
  }
}

describe('creative task polling', () => {
  test('reports queued/running/succeeded transitions and projects committed asset ids', async () => {
    const sequence = [task('queued'), task('running'), task('succeeded')];
    const observed: CreativeTaskStatus[] = [];
    let waits = 0;
    const terminal = await pollCreativeTask(
      portWithGet(async () => sequence.shift() ?? task('succeeded')),
      reference,
      {
        intervalMs: 0,
        wait: async () => {
          waits += 1;
        },
        onTask: (update) => observed.push(update.status),
      }
    );

    expect(observed).toEqual(['queued', 'running', 'succeeded']);
    expect(waits).toBe(2);
    expect(projectCreativeTaskOutput(terminal)).toEqual({
      taskId: TASK_ID,
      projectId: PROJECT_ID,
      nodeId: NODE_ID,
      assetIds: [ASSET_ID],
    });
  });

  test('keeps failed and canceled terminal states authoritative', async () => {
    const failed = await pollCreativeTask(portWithGet(async () => task('failed')), reference);
    const canceled = await pollCreativeTask(portWithGet(async () => task('canceled')), reference);

    expect(failed.status).toBe('failed');
    expect(failed.error?.kind).toBe('provider_error');
    expect(projectCreativeTaskOutput(failed)).toBe(null);
    expect(canceled.status).toBe('canceled');
    expect(projectCreativeTaskOutput(canceled)).toBe(null);
  });

  test('honors AbortSignal while waiting', async () => {
    const controller = new AbortController();
    const error = await caught(
      pollCreativeTask(portWithGet(async () => task('running')), reference, {
        intervalMs: 1,
        signal: controller.signal,
        wait: async (_delay, signal) => {
          controller.abort();
          if (signal?.aborted) {
            const aborted = new Error('aborted');
            aborted.name = 'AbortError';
            throw aborted;
          }
        },
      })
    );
    expect(error instanceof Error ? error.name : '').toBe('AbortError');
  });

  test('throws a local timeout without fabricating a terminal task', async () => {
    const times = [0, 50, 100];
    const error = await caught(
      pollCreativeTask(portWithGet(async () => task('running')), reference, {
        intervalMs: 0,
        maxWaitMs: 50,
        now: () => times.shift() ?? 100,
        wait: async () => undefined,
      })
    );
    expect(error instanceof CreativeTaskPollTimeoutError).toBe(true);
    expect((error as CreativeTaskPollTimeoutError).taskId).toBe(TASK_ID);
  });

  test('rejects an out-of-scope response even when a custom port fails to validate it', async () => {
    const error = await caught(
      pollCreativeTask(
        portWithGet(async () => task('running', { nodeId: '0190f5fe-7c00-7a00-8000-000000000099' })),
        reference
      )
    );
    expect(error instanceof CreativeTaskContractError).toBe(true);
    expect((error as CreativeTaskContractError).code).toBe('ownership_mismatch');
  });

  test('rejects a backend status regression instead of repainting running as queued', async () => {
    const sequence = [task('running'), task('queued')];
    const error = await caught(
      pollCreativeTask(
        portWithGet(async () => sequence.shift() ?? task('queued')),
        reference,
        { intervalMs: 0, wait: async () => undefined }
      )
    );
    expect(error instanceof CreativeTaskContractError).toBe(true);
    expect((error as CreativeTaskContractError).field).toBe('status');
  });
});
describe('pending task recovery', () => {
  test('derives exact pending references from canonical config-node ownership', () => {
    const document = createEmptyCreativeProjectDocument(PROJECT_ID);
    document.pendingTaskIds = [TASK_ID];
    document.nodes = [
      {
        id: NODE_ID,
        type: 'config',
        position: { x: 0, y: 0 },
        size: { width: 320, height: 240 },
        groupId: null,
        zIndex: 1,
        locked: false,
        data: {
          task: 'image_generation',
          capability: 't2i',
          providerId: PROVIDER_ID,
          model: 'image-model-v1',
          prompt: 'Aurora',
          negativePrompt: '',
          parameters: {},
          inputAssetIds: [],
          taskId: TASK_ID,
          resultAssetIds: [],
          status: 'running',
          errorMessage: null,
        },
      },
    ];

    expect(pendingCreativeTaskReferences(document)).toEqual([reference]);
  });

  test('fails closed when a pending id has no exact owner or has an unsupported capability', () => {
    const missingOwner = createEmptyCreativeProjectDocument(PROJECT_ID);
    missingOwner.pendingTaskIds = [TASK_ID];
    let missingError: unknown;
    try {
      pendingCreativeTaskReferences(missingOwner);
    } catch (error) {
      missingError = error;
    }

    const invalidCapability = createEmptyCreativeProjectDocument(PROJECT_ID);
    invalidCapability.pendingTaskIds = [TASK_ID];
    invalidCapability.nodes = [
      {
        id: NODE_ID,
        type: 'config',
        position: { x: 0, y: 0 },
        size: { width: 320, height: 240 },
        groupId: null,
        zIndex: 1,
        locked: false,
        data: {
          task: 'image_generation',
          capability: 'text-to-image',
          providerId: PROVIDER_ID,
          model: 'image-model-v1',
          prompt: '',
          negativePrompt: '',
          parameters: {},
          inputAssetIds: [],
          taskId: TASK_ID,
          resultAssetIds: [],
          status: 'running',
          errorMessage: null,
        },
      },
    ];
    let capabilityError: unknown;
    try {
      pendingCreativeTaskReferences(invalidCapability);
    } catch (error) {
      capabilityError = error;
    }

    expect((missingError as CreativeTaskContractError).code).toBe('ownership_mismatch');
    expect((capabilityError as CreativeTaskContractError).code).toBe('invalid_request');
  });

  test('recovers pending tasks in parallel and returns outputs only for succeeded tasks', async () => {
    const secondReference: CreativeTaskReference = {
      ...reference,
      taskId: SECOND_TASK_ID,
      nodeId: '0190f5fe-7c00-7a00-8000-000000000017',
    };
    const port = portWithGet(async (requested) =>
      requested.taskId === TASK_ID
        ? task('succeeded')
        : task('failed', {
            taskId: SECOND_TASK_ID,
            nodeId: secondReference.nodeId,
          })
    );
    const recovery = await recoverPendingCreativeTasks(port, [reference, secondReference]);

    expect(recovery.tasks.map((entry) => entry.status)).toEqual(['succeeded', 'failed']);
    expect(recovery.outputs).toEqual([
      { taskId: TASK_ID, projectId: PROJECT_ID, nodeId: NODE_ID, assetIds: [ASSET_ID] },
    ]);
    expect(recovery.issues).toEqual([]);
  });

  test('isolates a missing orphan without discarding another recoverable task', async () => {
    const secondReference: CreativeTaskReference = {
      ...reference,
      taskId: SECOND_TASK_ID,
      nodeId: '0190f5fe-7c00-7a00-8000-000000000017',
    };
    const port = portWithGet(async (requested) => {
      if (requested.taskId === TASK_ID) {
        throw new BackendHttpError({
          method: 'GET',
          path: `/api/creation/tasks/${TASK_ID}`,
          status: 404,
          body: { error: 'missing' },
        });
      }
      return task('succeeded', {
        taskId: SECOND_TASK_ID,
        nodeId: secondReference.nodeId,
      });
    });
    const recovery = await recoverPendingCreativeTasks(port, [reference, secondReference]);

    expect(recovery.tasks.map((entry) => entry.taskId)).toEqual([SECOND_TASK_ID]);
    expect(recovery.outputs.map((entry) => entry.taskId)).toEqual([SECOND_TASK_ID]);
    expect(recovery.issues).toHaveLength(1);
    expect(recovery.issues[0]?.reference.taskId).toBe(TASK_ID);
    expect(recovery.issues[0]?.kind).toBe('orphaned');
  });
});

describe('CreativeTaskProgressGuard', () => {
  test('keeps every terminal status irreversible and immutable', () => {
    const guard = new CreativeTaskProgressGuard();
    guard.observe(task('succeeded'));
    let statusError: unknown;
    try {
      guard.observe(task('canceled'));
    } catch (error) {
      statusError = error;
    }
    expect(statusError instanceof CreativeTaskContractError).toBe(true);

    const immutable = new CreativeTaskProgressGuard();
    immutable.observe(task('failed'));
    let mutationError: unknown;
    try {
      immutable.observe(task('failed', { attempt: 2 }));
    } catch (error) {
      mutationError = error;
    }
    expect(mutationError instanceof CreativeTaskContractError).toBe(true);
  });
});

describe('CreativeTaskRequestFence', () => {
  test('isolates late responses from superseded operations', () => {
    const fence = new CreativeTaskRequestFence();
    const stale = fence.begin();
    const current = fence.begin();
    const effects: string[] = [];

    expect(fence.commit(stale, () => effects.push('stale'))).toBe(false);
    expect(fence.commit(current, () => effects.push('current'))).toBe(true);
    fence.invalidate();
    expect(fence.commit(current, () => effects.push('late'))).toBe(false);
    expect(effects).toEqual(['current']);
  });
});
