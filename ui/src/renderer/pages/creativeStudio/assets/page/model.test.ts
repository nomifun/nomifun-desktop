/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { describe, expect, test } from 'bun:test';

import type { CreativeAsset } from '../types';
import {
  CREATIVE_ASSET_MANUAL_UPLOAD_LIMIT_BYTES,
  buildGlobalCreativeAssetQuery,
  creativeAssetCacheIsComplete,
  creativeAssetDownloadName,
  creativeAssetPageCount,
  creativeAssetPageIsLoaded,
  creativeAssetPageSlice,
  creativeAssetPageSliceFromCompleteCache,
  creativeAssetQuerySearch,
  normalizeCreativeAssetEditDraft,
  normalizeCreativeTextAssetForm,
  validateCreativeAssetManualUpload,
  validateCreativeCollectionRename,
} from './model';
import { creativeAssetUploadQueueReducer } from './uploadQueue';

describe('creative asset library route model', () => {
  test('builds a global library-only paginated query without inventing a canvas scope', () => {
    expect(buildGlobalCreativeAssetQuery('  campaign  ', 'video')).toEqual({
      inLibrary: true,
      kind: 'video',
      search: 'campaign',
      sort: 'updated_desc',
    });
    expect(buildGlobalCreativeAssetQuery('   ', 'all')).toEqual({
      inLibrary: true,
      kind: undefined,
      search: undefined,
      sort: 'updated_desc',
    });
  });

  test('lets an explicit Enter submission flush ahead of the pending debounce value', () => {
    expect(creativeAssetQuerySearch('old debounce', 'submitted now')).toBe('submitted now');
    expect(creativeAssetQuerySearch('old debounce', '')).toBe('');
    expect(creativeAssetQuerySearch('debounced value', null)).toBe('debounced value');
  });

  test('computes real ten-item pages from a fully synchronized backend cache', () => {
    const items = Array.from({ length: 14 }, (_, index) => index + 1);
    expect(creativeAssetPageCount(0, 10)).toBe(1);
    expect(creativeAssetPageCount(14, 10)).toBe(2);
    expect(creativeAssetPageSlice(items, 1, 10)).toEqual([1, 2, 3, 4, 5, 6, 7, 8, 9, 10]);
    expect(creativeAssetPageSlice(items, 2, 10)).toEqual([11, 12, 13, 14]);
  });

  test('only requires enough loaded assets to render the requested page', () => {
    expect(creativeAssetPageIsLoaded(10, 100, 1, 10)).toBe(true);
    expect(creativeAssetPageIsLoaded(10, 100, 2, 10)).toBe(false);
    expect(creativeAssetPageIsLoaded(20, 100, 2, 10)).toBe(true);
    expect(creativeAssetPageIsLoaded(14, 14, 2, 10)).toBe(true);
    expect(creativeAssetPageIsLoaded(0, 0, 1, 10)).toBe(true);
  });

  test('never exposes a stale second page while reload has only restored backend page one', () => {
    const firstBackendPage = Array.from({ length: 10 }, (_, index) => index + 1);
    const fullySynchronized = Array.from({ length: 20 }, (_, index) => index + 1);

    expect(creativeAssetCacheIsComplete(firstBackendPage.length, 20)).toBe(false);
    expect(creativeAssetPageSliceFromCompleteCache(firstBackendPage, 20, 2, 10)).toEqual([]);
    expect(creativeAssetCacheIsComplete(fullySynchronized.length, 20)).toBe(true);
    expect(creativeAssetPageSliceFromCompleteCache(fullySynchronized, 20, 2, 10)).toEqual([
      11, 12, 13, 14, 15, 16, 17, 18, 19, 20,
    ]);
  });

  test('reflows a complete cache across the deletion boundary without losing the next item', () => {
    const fullySynchronized = Array.from({ length: 12 }, (_, index) => `asset-${index + 1}`);
    const afterDeletingFirst = fullySynchronized.slice(1);

    expect(creativeAssetPageSliceFromCompleteCache(afterDeletingFirst, 11, 1, 10)).toEqual([
      'asset-2', 'asset-3', 'asset-4', 'asset-5', 'asset-6',
      'asset-7', 'asset-8', 'asset-9', 'asset-10', 'asset-11',
    ]);
    expect(creativeAssetPageSliceFromCompleteCache(afterDeletingFirst, 11, 2, 10)).toEqual(['asset-12']);
  });

  test('enforces the visible manual-upload capability contract before calling the backend', () => {
    expect(validateCreativeAssetManualUpload({ name: 'image.png', type: 'image/png', size: 1024 })).toEqual({
      accepted: true,
      rejection: null,
    });
    expect(validateCreativeAssetManualUpload({ name: 'clip.mp4', type: 'video/mp4', size: 2048 }).accepted).toBe(true);
    expect(validateCreativeAssetManualUpload({ name: 'voice.wav', type: 'audio/wav', size: 2048 }).rejection).toBe('audio_unsupported');
    expect(validateCreativeAssetManualUpload({ name: 'asset.pdf', type: 'application/pdf', size: 2048 }).rejection).toBe('unsupported_media_type');
    expect(validateCreativeAssetManualUpload({
      name: 'large.png',
      type: 'image/png',
      size: CREATIVE_ASSET_MANUAL_UPLOAD_LIMIT_BYTES + 1,
    }).rejection).toBe('file_too_large');
  });

  test('normalizes only metadata fields the backend can actually update', () => {
    expect(normalizeCreativeAssetEditDraft({
      title: '  Hero  ',
      collection: '  Launch  ',
      tags: ['blue', ' blue ', '', 'wide'],
      inLibrary: true,
    })).toEqual({ title: 'Hero', collection: 'Launch', tags: ['blue', 'wide'], inLibrary: true });
    expect(normalizeCreativeTextAssetForm({
      title: ' Copy ',
      textContent: ' Body ',
      collection: ' Notes ',
      tags: ['draft', 'draft'],
      inLibrary: true,
    })).toEqual({
      title: 'Copy',
      textContent: 'Body',
      collection: 'Notes',
      tags: ['draft'],
      inLibrary: true,
    });
  });

  test('validates collection rename and permits the backend-supported ungroup operation', () => {
    expect(validateCreativeCollectionRename({ from: '', to: 'new' })).toBe('请输入当前合集名称。');
    expect(validateCreativeCollectionRename({ from: 'same', to: ' same ' })).toBe('新合集名称需要与当前名称不同。');
    expect(validateCreativeCollectionRename({ from: 'old', to: '' })).toBeNull();
  });

  test('derives a safe download name while retaining the real original URL elsewhere', () => {
    const asset = {
      title: 'Hero: 01',
      mimeType: 'image/jpeg',
      kind: 'image',
    } satisfies Pick<CreativeAsset, 'title' | 'mimeType' | 'kind'>;
    expect(creativeAssetDownloadName(asset)).toBe('Hero- 01.jpg');
  });

  test('upload queue only reports completion after a real upload resolution action', () => {
    const queued = creativeAssetUploadQueueReducer([], {
      type: 'enqueue',
      item: { id: 'upload-1', fileName: 'hero.png', percent: 0, status: 'uploading' },
    });
    const progressed = creativeAssetUploadQueueReducer(queued, { type: 'progress', id: 'upload-1', percent: 47 });
    expect(progressed[0]).toMatchObject({ percent: 47, status: 'uploading' });
    const completed = creativeAssetUploadQueueReducer(progressed, { type: 'complete', id: 'upload-1' });
    expect(completed[0]).toMatchObject({ percent: 100, status: 'completed' });
    const restarted = creativeAssetUploadQueueReducer(
      creativeAssetUploadQueueReducer(progressed, { type: 'fail', id: 'upload-1', error: 'offline' }),
      { type: 'restart', id: 'upload-1' }
    );
    expect(restarted[0]).toEqual({ id: 'upload-1', fileName: 'hero.png', percent: 0, status: 'uploading', error: undefined });
  });
});
