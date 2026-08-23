/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { describe, expect, test } from 'bun:test';

import {
  createDefaultImageWorkbenchDraft,
  createDefaultVideoWorkbenchDraft,
  createImageWorkbenchDraft,
  createVideoWorkbenchDraft,
} from './types';
import {
  readStandaloneWorkbenchDraft,
  standaloneWorkbenchDraftStorageKey,
  writeStandaloneWorkbenchDraft,
  STANDALONE_WORKBENCH_DRAFT_MAX_PROMPT_LENGTH,
  STANDALONE_WORKBENCH_DRAFT_MAX_SERIALIZED_LENGTH,
  type StandaloneWorkbenchDraftStorage,
} from './storage';

const PROVIDER_A = '0190f5fe-7c00-7a00-8000-000000000101';
const IMAGE_ASSET_A = '0190f5fe-7c00-7a00-8000-000000000201';
const IMAGE_ASSET_B = '0190f5fe-7c00-7a00-8000-000000000202';

const memoryStorage = (): StandaloneWorkbenchDraftStorage & {
  values: Map<string, string>;
  removed: string[];
} => {
  const values = new Map<string, string>();
  const removed: string[] = [];
  return {
    getItem: (key) => values.get(key) ?? null,
    setItem: (key, value) => {
      values.set(key, value);
    },
    removeItem: (key) => {
      removed.push(key);
      values.delete(key);
    },
    values,
    removed,
  };
};

describe('standalone workbench session draft storage', () => {
  test('uses one versioned document per workbench kind without Canvas or Project identity', () => {
    const imageKey = standaloneWorkbenchDraftStorageKey('image');
    const videoKey = standaloneWorkbenchDraftStorageKey('video');

    expect(imageKey).not.toBe(videoKey);
    expect(imageKey.endsWith(':image')).toBe(true);
    expect(videoKey.endsWith(':video')).toBe(true);
    for (const key of [imageKey, videoKey]) {
      expect(key.includes('projectId')).toBe(false);
      expect(key.includes('project_id')).toBe(false);
      expect(key.includes('canvasId')).toBe(false);
      expect(key.includes('canvas_id')).toBe(false);
    }
  });

  test('round-trips image prompt, exact model, parameters, asset IDs, and layout only', () => {
    const storage = memoryStorage();
    const draft = createImageWorkbenchDraft({
      layout: 'bottom',
      prompt: '雨夜中的霓虹街道',
      settings: {
        model: { providerId: PROVIDER_A, model: 'image-exact-v2' },
        interfaceMode: 'responses',
        quality: 'high',
        width: 1536,
        height: 1024,
        aspectRatio: '3:2',
        count: 4,
      },
      referenceAssetIds: [IMAGE_ASSET_A, IMAGE_ASSET_B],
    });

    expect(writeStandaloneWorkbenchDraft(draft, storage)).toBe(true);
    expect(readStandaloneWorkbenchDraft('image', storage)).toEqual(draft);

    const raw = storage.values.get(standaloneWorkbenchDraftStorageKey('image'));
    expect(raw).toBeDefined();
    const document = JSON.parse(raw as string) as Record<string, unknown>;
    expect(Object.keys(document).sort()).toEqual(
      [
        'layout',
        'model',
        'parameters',
        'prompt',
        'referenceAssetIds',
        'version',
        'workbenchKind',
      ].sort()
    );
    for (const transient of [
      'busy',
      'error',
      'modal',
      'pickerOpen',
      'assets',
      'references',
      'selectedResultIds',
    ]) {
      expect(Object.hasOwn(document, transient)).toBe(false);
    }
  });

  test('round-trips the independent video draft without widening its product gate', () => {
    const storage = memoryStorage();
    const draft = createVideoWorkbenchDraft({
      layout: 'bottom',
      prompt: '固定镜头，晨雾缓慢散去',
      model: { providerId: PROVIDER_A, model: 'video-exact-v1' },
      resolution: '720p',
      aspect: '9:16',
      duration: '10',
      taskCount: 1,
      referenceAssetIds: [IMAGE_ASSET_A],
    });

    expect(writeStandaloneWorkbenchDraft(draft, storage)).toBe(true);
    expect(readStandaloneWorkbenchDraft('video', storage)).toEqual(draft);
    expect(readStandaloneWorkbenchDraft('image', storage)).toBe(null);
  });

  test('fails closed and clears corrupt, unknown-version, cross-kind, and oversized values', () => {
    const cases: Array<{ kind: 'image' | 'video'; raw: string }> = [
      { kind: 'image', raw: '{not-json' },
      {
        kind: 'image',
        raw: JSON.stringify({ ...createDefaultImageWorkbenchDraft(), version: 2 }),
      },
      {
        kind: 'video',
        raw: JSON.stringify({
          ...createDefaultVideoWorkbenchDraft(),
          workbenchKind: 'image',
        }),
      },
      {
        kind: 'image',
        raw: 'x'.repeat(STANDALONE_WORKBENCH_DRAFT_MAX_SERIALIZED_LENGTH + 1),
      },
    ];

    for (const { kind, raw } of cases) {
      const storage = memoryStorage();
      const key = standaloneWorkbenchDraftStorageKey(kind);
      storage.values.set(key, raw);

      expect(readStandaloneWorkbenchDraft(kind, storage)).toBe(null);
      expect(storage.values.has(key)).toBe(false);
      expect(storage.removed.includes(key)).toBe(true);
    }
  });

  test('rejects overlong or malformed state instead of restoring stale prior content', () => {
    const storage = memoryStorage();
    const valid = createDefaultImageWorkbenchDraft();
    valid.prompt = 'previous';
    expect(writeStandaloneWorkbenchDraft(valid, storage)).toBe(true);

    const overlong = createDefaultImageWorkbenchDraft();
    overlong.prompt = 'x'.repeat(STANDALONE_WORKBENCH_DRAFT_MAX_PROMPT_LENGTH + 1);
    expect(writeStandaloneWorkbenchDraft(overlong, storage)).toBe(false);
    expect(readStandaloneWorkbenchDraft('image', storage)).toBe(null);

    const duplicateReferences = {
      ...createDefaultImageWorkbenchDraft(),
      referenceAssetIds: [IMAGE_ASSET_A, IMAGE_ASSET_A],
    };
    expect(
      writeStandaloneWorkbenchDraft(duplicateReferences, storage)
    ).toBe(false);

    const unknownField = {
      ...createDefaultVideoWorkbenchDraft(),
      busy: true,
    };
    storage.values.set(
      standaloneWorkbenchDraftStorageKey('video'),
      JSON.stringify(unknownField)
    );
    expect(readStandaloneWorkbenchDraft('video', storage)).toBe(null);
  });

  test('tolerates unavailable sessionStorage reads, writes, and invalid-value cleanup', () => {
    const unavailable: StandaloneWorkbenchDraftStorage = {
      getItem: () => {
        throw new Error('blocked get');
      },
      setItem: () => {
        throw new Error('blocked set');
      },
      removeItem: () => {
        throw new Error('blocked remove');
      },
    };

    expect(readStandaloneWorkbenchDraft('image', unavailable)).toBe(null);
    expect(
      writeStandaloneWorkbenchDraft(createDefaultImageWorkbenchDraft(), unavailable)
    ).toBe(false);

    const cleanupBlocked: StandaloneWorkbenchDraftStorage = {
      getItem: () => '{broken',
      setItem: () => undefined,
      removeItem: () => {
        throw new Error('blocked remove');
      },
    };
    expect(readStandaloneWorkbenchDraft('video', cleanupBlocked)).toBe(null);

    const quotaStorage = memoryStorage();
    const imageKey = standaloneWorkbenchDraftStorageKey('image');
    quotaStorage.values.set(imageKey, JSON.stringify(createDefaultImageWorkbenchDraft()));
    quotaStorage.setItem = () => {
      throw new Error('quota exceeded');
    };
    expect(
      writeStandaloneWorkbenchDraft(createDefaultImageWorkbenchDraft(), quotaStorage)
    ).toBe(false);
    expect(quotaStorage.values.has(imageKey)).toBe(false);
  });
});
