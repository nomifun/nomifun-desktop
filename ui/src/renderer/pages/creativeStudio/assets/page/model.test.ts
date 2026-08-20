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
  creativeAssetDownloadName,
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
