/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { describe, expect, test } from 'bun:test';

import type { CreativeAsset } from '../../assets';
import { testUuid } from '../../canvas/core/testFixtures';
import type { CreativeTask } from '../../tasks';
import { hydrateStandaloneTaskReferences } from './references';

const PROJECT_ID = testUuid(820);
const IMAGE_ID = testUuid(821);
const MASK_ID = testUuid(822);
const baseTask: CreativeTask = {
  taskId: testUuid(823),
  owner: { kind: 'standalone_workbench', projectId: PROJECT_ID, workbenchKind: 'image' },
  providerId: testUuid(824),
  model: 'image-v1',
  task: 'image_edit',
  capability: 'inpaint',
  parameters: { prompt: 'Edit' },
  inputs: [
    { assetId: IMAGE_ID, kind: 'image', role: 'reference' },
    { assetId: MASK_ID, kind: 'image', role: 'mask' },
  ],
  status: 'failed',
  error: { kind: 'provider', message: 'failed', httpStatus: 500 },
  resultAssetIds: [],
  attempt: 1,
  submittedAt: 1,
  startedAt: 2,
  finishedAt: 3,
  deletedAt: null,
};

const asset = (id: string, kind: CreativeAsset['kind'] = 'image'): CreativeAsset => ({
  id,
  kind,
  title: id,
  collection: null,
  tags: [],
  mimeType: kind === 'image' ? 'image/png' : 'video/mp4',
  width: 1,
  height: 1,
  bytes: 1,
  inLibrary: true,
  textContent: null,
  origin: null,
  originalUrl: `/assets/${id}`,
  thumbnailUrl: null,
  createdAt: 1,
  updatedAt: 1,
});

describe('standalone history reference hydration', () => {
  test('reads every input by exact id and preserves binding order and roles', async () => {
    const result = await hydrateStandaloneTaskReferences(baseTask, {
      get: async (id) => asset(id),
    });
    expect(result.bindings).toEqual(baseTask.inputs);
    expect(result.assets.map((item) => item.id)).toEqual([IMAGE_ID, MASK_ID]);
  });

  test('fails closed on legacy or media-kind drift', async () => {
    for (const candidate of [
      { ...baseTask, inputs: null },
      baseTask,
    ]) {
      let error: unknown = null;
      try {
        await hydrateStandaloneTaskReferences(candidate, {
          get: async (id) => asset(id, candidate.inputs === null ? 'image' : 'video'),
        });
      } catch (reason) {
        error = reason;
      }
      expect(error instanceof Error).toBe(true);
    }
  });
});
