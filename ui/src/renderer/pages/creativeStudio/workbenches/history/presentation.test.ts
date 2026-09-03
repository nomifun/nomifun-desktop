/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { describe, expect, test } from 'bun:test';

import type { CreativeAssetPort } from '../../assets';
import { testUuid } from '../../canvas/core/testFixtures';
import type { CreativeTask } from '../../tasks';
import { mapImageWorkbenchRuntimeResults, type CreativeWorkbenchRuntimeSnapshot } from '../runtime';
import {
  standaloneHistoryResumeRequests,
  standaloneHistoryRuntimeSnapshot,
} from './presentation';

const ASSET_A = testUuid(811);
const ASSET_B = testUuid(812);
const scope = { workbenchKind: 'image' as const };
const task = (status: CreativeTask['status']): CreativeTask => ({
  taskId: testUuid(813),
  owner: { kind: 'standalone_workbench', workbenchKind: 'image' },
  providerId: testUuid(814),
  model: 'image-v1',
  task: 'image_generation',
  capability: 't2i',
  parameters: { prompt: 'Aurora' },
  inputs: [],
  status,
  error: null,
  resultAssetIds: status === 'succeeded' ? [ASSET_A, ASSET_B] : [],
  attempt: 1,
  submittedAt: 10,
  startedAt: status === 'queued' ? null : 11,
  finishedAt: status === 'succeeded' ? 12 : null,
  deletedAt: null,
});
const runtime: CreativeWorkbenchRuntimeSnapshot = {
  state: 'idle',
  entries: [],
  submissionFailures: [],
  submittingCount: 0,
  recoveringCount: 0,
  requestError: null,
};
const assets = {
  url: (assetId: string) => `/assets/${assetId}`,
} as CreativeAssetPort;

describe('standalone history presentation', () => {
  test('preserves historical output identity and flags tombstones for placeholder rendering', () => {
    const original = { ...task('succeeded'), inputs: [{ assetId: ASSET_B, kind: 'image' as const, role: 'reference' as const }] };
    const snapshot = standaloneHistoryRuntimeSnapshot(scope, [original], runtime, assets, new Map([
      [ASSET_A, 'available' as const], [ASSET_B, 'deleted' as const],
    ]));
    expect(snapshot.entries[0]?.task.resultAssetIds).toEqual([ASSET_A, ASSET_B]);
    const result = mapImageWorkbenchRuntimeResults(snapshot)[0];
    expect(result?.hasDeletedInputs).toBe(true);
    expect(result?.retryable).toBe(false);
    if (result?.status !== 'succeeded') throw new Error('Expected historical success');
    expect(result.outputs.map((output) => output.availability)).toEqual(['available', 'deleted']);
  });
  test('keeps a durable multi-output task as one runtime entry', () => {
    const snapshot = standaloneHistoryRuntimeSnapshot(scope, [task('succeeded')], runtime, assets);
    expect(snapshot.entries).toHaveLength(1);
    expect(snapshot.entries[0]?.outputs.map((output) => output.assetId)).toEqual([
      ASSET_A,
      ASSET_B,
    ]);
  });

  test('creates resume requests only from exact active owner rows', () => {
    const queued = task('queued');
    expect(standaloneHistoryResumeRequests(scope, [queued])[0]).toMatchObject({
      reference: { taskId: queued.taskId, owner: queued.owner },
      outputKind: 'image',
      retryInput: null,
    });
    let error: unknown = null;
    try {
      standaloneHistoryResumeRequests(scope, [task('succeeded')]);
    } catch (reason) {
      error = reason;
    }
    expect(error instanceof Error).toBe(true);
  });
});
