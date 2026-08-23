/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { describe, expect, test } from 'bun:test';

import { testUuid } from '../../canvas/core/testFixtures';
import {
  CreativeTaskContractError,
  type CreateCreativeTaskInput,
  type CreativeTask,
} from '../../tasks';
import type {
  CreativeWorkbenchRuntimeEntry,
  CreativeWorkbenchRuntimeSnapshot,
} from '../runtime';
import {
  canRetryStandaloneWorkbenchHistoryTask,
  isExactStandaloneWorkbenchHistoryTask,
  mergeStandaloneWorkbenchHistory,
  type StandaloneWorkbenchHistoryScope,
} from './model';

const scope: StandaloneWorkbenchHistoryScope = {
  workbenchKind: 'image',
};

const task = (
  index: number,
  submittedAt: number,
  overrides: Partial<CreativeTask> = {}
): CreativeTask => ({
  taskId: testUuid(index),
  owner: {
    kind: 'standalone_workbench',
    workbenchKind: 'image',
  },
  providerId: testUuid(520),
  model: 'image-v1',
  task: 'image_generation',
  capability: 't2i',
  parameters: { prompt: `prompt-${index}`, count: 2 },
  inputs: [],
  status: 'queued',
  error: null,
  resultAssetIds: [],
  attempt: 1,
  submittedAt,
  startedAt: null,
  finishedAt: null,
  deletedAt: null,
  ...overrides,
});

const retryInput = (value: CreativeTask): CreateCreativeTaskInput => ({
  idempotencyKey: testUuid(590),
  owner: { ...value.owner },
  providerId: value.providerId,
  model: value.model,
  task: value.task,
  capability: value.capability,
  parameters: structuredClone(value.parameters),
  inputs: structuredClone(value.inputs ?? []),
});

const liveEntry = (
  value: CreativeTask,
  overrides: Partial<CreativeWorkbenchRuntimeEntry> = {}
): CreativeWorkbenchRuntimeEntry => ({
  order: 0,
  task: value,
  outputs: [],
  requestError: null,
  retryInput: retryInput(value),
  outputKind: 'image',
  ...overrides,
});

const runtime = (
  entries: readonly CreativeWorkbenchRuntimeEntry[]
): CreativeWorkbenchRuntimeSnapshot => ({
  state: entries.length === 0 ? 'idle' : 'mixed',
  entries,
  submissionFailures: [],
  submittingCount: 0,
  recoveringCount: 0,
  requestError: null,
});

const captureError = (action: () => void): Error | null => {
  try {
    action();
    return null;
  } catch (error) {
    return error instanceof Error ? error : new Error(String(error));
  }
};

describe('standalone workbench history model', () => {
  test('merges live over durable, keeps multi-output tasks whole and sorts stably', () => {
    const olderDurable = task(510, 100);
    const firstResultId = testUuid(530);
    const secondResultId = testUuid(531);
    const olderLive: CreativeTask = {
      ...olderDurable,
      status: 'succeeded',
      resultAssetIds: [firstResultId, secondResultId],
      startedAt: 101,
      finishedAt: 110,
    };
    const newerLowerId = task(511, 200, {
      status: 'failed',
      error: { kind: 'provider_error', message: 'failed', httpStatus: 500 },
      finishedAt: 210,
    });
    const newerHigherId = task(512, 200, {
      status: 'succeeded',
      resultAssetIds: [testUuid(532)],
      startedAt: 201,
      finishedAt: 205,
    });
    const live = liveEntry(olderLive, {
      outputs: [
        {
          assetId: firstResultId,
          kind: 'image',
          url: `/assets/${firstResultId}`,
        },
        {
          assetId: secondResultId,
          kind: 'image',
          url: `/assets/${secondResultId}`,
        },
      ],
    });

    const result = mergeStandaloneWorkbenchHistory({
      scope,
      durableTasks: [olderDurable, newerLowerId, newerHigherId],
      runtime: runtime([live]),
    });

    expect(result.map((item) => item.task.taskId)).toEqual([
      newerHigherId.taskId,
      newerLowerId.taskId,
      olderLive.taskId,
    ]);
    const mergedLive = result[2];
    expect(mergedLive?.task).toBe(olderLive);
    expect(mergedLive?.source).toBe('live');
    expect(mergedLive?.runtimeEntry).toBe(live);
    expect(mergedLive?.runtimeEntry?.outputs).toHaveLength(2);
    expect(result).toHaveLength(3);
    expect(result[0]?.canRetry).toBe(false);
    expect(result[1]?.canRetry).toBe(true);
    expect(result[2]?.canRetry).toBe(false);
  });

  test('deduplicates byte-equivalent durable rows but rejects any duplicate drift', () => {
    const original = task(513, 300);
    const exactCopy = structuredClone(original);
    expect(
      mergeStandaloneWorkbenchHistory({
        scope,
        durableTasks: [original, exactCopy],
        runtime: runtime([]),
      })
    ).toHaveLength(1);

    const error = captureError(() =>
      mergeStandaloneWorkbenchHistory({
        scope,
        durableTasks: [original, { ...exactCopy, status: 'running' }],
        runtime: runtime([]),
      })
    );
    expect(error instanceof CreativeTaskContractError).toBe(true);
    if (error instanceof CreativeTaskContractError) {
      expect(error.code).toBe('invalid_response');
      expect(error.field).toBe('durableTasks[1]');
    }
  });

  test('never lets stale live progress downgrade durable state', () => {
    const resultId = testUuid(533);
    const durableTerminal = task(534, 320, {
      status: 'succeeded',
      resultAssetIds: [resultId],
      startedAt: 321,
      finishedAt: 330,
    });
    const staleLive = task(534, 320);
    const merged = mergeStandaloneWorkbenchHistory({
      scope,
      durableTasks: [durableTerminal],
      runtime: runtime([liveEntry(staleLive)]),
    });
    expect(merged[0]?.task).toBe(durableTerminal);
    expect(merged[0]?.source).toBe('durable');
    expect(merged[0]?.runtimeEntry).toBeNull();

    const conflictingTerminal = task(534, 320, {
      status: 'failed',
      error: { kind: 'provider_error', message: 'late failure', httpStatus: 500 },
      startedAt: 321,
      finishedAt: 331,
    });
    const error = captureError(() =>
      mergeStandaloneWorkbenchHistory({
        scope,
        durableTasks: [durableTerminal],
        runtime: runtime([liveEntry(conflictingTerminal)]),
      })
    );
    expect(error instanceof CreativeTaskContractError).toBe(true);
  });

  test('binds live outputs to the exact succeeded task result order', () => {
    const firstResultId = testUuid(535);
    const secondResultId = testUuid(536);
    const succeeded = task(537, 340, {
      status: 'succeeded',
      resultAssetIds: [firstResultId, secondResultId],
      startedAt: 341,
      finishedAt: 350,
    });
    for (const entry of [
      liveEntry(succeeded, {
        outputs: [
          { assetId: secondResultId, kind: 'image', url: '/second.png' },
          { assetId: firstResultId, kind: 'image', url: '/first.png' },
        ],
      }),
      liveEntry(succeeded, {
        outputs: [
          { assetId: firstResultId, kind: 'image', url: null },
          { assetId: secondResultId, kind: 'image', url: '/second.png' },
        ],
      }),
      liveEntry(task(538, 339, { status: 'failed' }), {
        outputs: [
          { assetId: firstResultId, kind: 'image', url: '/unexpected.png' },
        ],
      }),
    ]) {
      const error = captureError(() =>
        mergeStandaloneWorkbenchHistory({
          scope,
          durableTasks: [],
          runtime: runtime([entry]),
        })
      );
      expect(error instanceof CreativeTaskContractError).toBe(true);
    }

    const unresolved = liveEntry(succeeded, {
      outputs: [],
      requestError: new Error('asset mapping unavailable'),
    });
    expect(
      mergeStandaloneWorkbenchHistory({
        scope,
        durableTasks: [],
        runtime: runtime([unresolved]),
      })[0]?.runtimeEntry
    ).toBe(unresolved);
  });

  test('accepts legacy project provenance but rejects cross-workbench task owners', () => {
    const valid = task(514, 400);
    const foreignKind = task(516, 398, {
      owner: {
        kind: 'standalone_workbench',
        workbenchKind: 'video',
      },
      task: 'video_generation',
      capability: 't2v',
    });
    const canvasOwned = task(517, 397, {
      owner: {
        kind: 'canvas_node',
        canvasId: testUuid(500),
        nodeId: testUuid(518),
      },
    });
    const retired = task(518, 396, { deletedAt: 500 });

    const legacyProject = task(515, 399);
    expect(
      mergeStandaloneWorkbenchHistory({
        scope,
        durableTasks: [valid, legacyProject],
        runtime: runtime([]),
      })
    ).toHaveLength(2);
    const candidates = [foreignKind, canvasOwned, retired];
    for (const candidate of candidates) {
      const error = captureError(() =>
        mergeStandaloneWorkbenchHistory({
          scope,
          durableTasks: [valid, candidate],
          runtime: runtime([]),
        })
      );
      expect(error instanceof CreativeTaskContractError).toBe(true);
      if (error instanceof CreativeTaskContractError) {
        expect(error.code).toBe('ownership_mismatch');
      }
    }

    expect(isExactStandaloneWorkbenchHistoryTask(valid, scope)).toBe(true);
    expect(isExactStandaloneWorkbenchHistoryTask(legacyProject, scope)).toBe(true);
    expect(isExactStandaloneWorkbenchHistoryTask(foreignKind, scope)).toBe(false);
  });

  test('rejects live immutable identity drift and duplicate live task ids', () => {
    const durable = task(519, 500);
    const drifted = { ...durable, model: 'different-model' };
    const driftError = captureError(() =>
      mergeStandaloneWorkbenchHistory({
        scope,
        durableTasks: [durable],
        runtime: runtime([liveEntry(drifted)]),
      })
    );
    expect(driftError instanceof CreativeTaskContractError).toBe(true);
    if (driftError instanceof CreativeTaskContractError) {
      expect(driftError.code).toBe('invalid_response');
      expect(driftError.field).toBe('runtime.entries.task');
    }

    const entry = liveEntry(durable);
    const duplicateError = captureError(() =>
      mergeStandaloneWorkbenchHistory({
        scope,
        durableTasks: [],
        runtime: runtime([entry, { ...entry, order: 1 }]),
      })
    );
    expect(duplicateError instanceof CreativeTaskContractError).toBe(true);
    if (duplicateError instanceof CreativeTaskContractError) {
      expect(duplicateError.field).toBe('runtime.entries[1]');
    }
  });

  test('rejects live retry or output data that escapes the exact scope', () => {
    const value = task(521, 600, { status: 'failed' });
    const wrongRetry = retryInput(value);
    wrongRetry.owner = {
      kind: 'standalone_workbench',
      workbenchKind: 'video',
    };
    const retryError = captureError(() =>
      mergeStandaloneWorkbenchHistory({
        scope,
        durableTasks: [],
        runtime: runtime([liveEntry(value, { retryInput: wrongRetry })]),
      })
    );
    expect(retryError instanceof CreativeTaskContractError).toBe(true);
    if (retryError instanceof CreativeTaskContractError) {
      expect(retryError.field).toBe('runtime.entries[0].retryInput');
    }

    const legacy = task(529, 599, { status: 'failed', inputs: null });
    const legacyRetryError = captureError(() =>
      mergeStandaloneWorkbenchHistory({
        scope,
        durableTasks: [],
        runtime: runtime([liveEntry(legacy)]),
      })
    );
    expect(legacyRetryError instanceof CreativeTaskContractError).toBe(true);

    const outputError = captureError(() =>
      mergeStandaloneWorkbenchHistory({
        scope,
        durableTasks: [],
        runtime: runtime([
          liveEntry(value, {
            outputKind: 'video',
            outputs: [
              { assetId: testUuid(522), kind: 'video', url: '/video.mp4' },
            ],
          }),
        ]),
      })
    );
    expect(outputError instanceof CreativeTaskContractError).toBe(true);
    if (outputError instanceof CreativeTaskContractError) {
      expect(outputError.field).toBe('runtime.entries[0].outputKind');
    }
  });

  test('allows retry only for failed or canceled tasks with exact input snapshots', () => {
    const failed = task(523, 700, { status: 'failed', inputs: [] });
    const canceled = task(524, 699, {
      status: 'canceled',
      inputs: [
        { assetId: testUuid(540), kind: 'image', role: 'reference' },
      ],
    });
    const legacyFailed = task(525, 698, { status: 'failed', inputs: null });
    const succeeded = task(526, 697, { status: 'succeeded', inputs: [] });
    const running = task(527, 696, { status: 'running', inputs: [] });

    expect(canRetryStandaloneWorkbenchHistoryTask(failed)).toBe(true);
    expect(canRetryStandaloneWorkbenchHistoryTask(canceled)).toBe(true);
    expect(canRetryStandaloneWorkbenchHistoryTask(legacyFailed)).toBe(false);
    expect(canRetryStandaloneWorkbenchHistoryTask(succeeded)).toBe(false);
    expect(canRetryStandaloneWorkbenchHistoryTask(running)).toBe(false);
  });

  test('rejects a task whose model task cannot belong to the owner workbench', () => {
    const mismatched = task(528, 800, {
      task: 'video_generation',
      capability: 't2v',
    });
    const error = captureError(() =>
      mergeStandaloneWorkbenchHistory({
        scope,
        durableTasks: [mismatched],
        runtime: runtime([]),
      })
    );
    expect(error instanceof CreativeTaskContractError).toBe(true);
    if (error instanceof CreativeTaskContractError) {
      expect(error.code).toBe('ownership_mismatch');
    }
  });
});
