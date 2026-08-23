/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { describe, expect, test } from 'bun:test';

import type { CreativeAsset, CreativeAssetKind } from '../../assets';
import {
  hydrateStandaloneWorkbenchDraftReferences,
  isExactWorkbenchDraftModelAvailable,
} from './hydration';

const PROVIDER_A = '0190f5fe-7c00-7a00-8000-000000000101';
const PROVIDER_B = '0190f5fe-7c00-7a00-8000-000000000102';
const ASSET_A = '0190f5fe-7c00-7a00-8000-000000000201';
const ASSET_B = '0190f5fe-7c00-7a00-8000-000000000202';
const ASSET_C = '0190f5fe-7c00-7a00-8000-000000000203';
const ASSET_D = '0190f5fe-7c00-7a00-8000-000000000204';

const asset = (id: string, kind: CreativeAssetKind = 'image'): CreativeAsset => ({
  id,
  kind,
  title: id,
  collection: null,
  tags: [],
  mimeType: kind === 'video' ? 'video/mp4' : 'image/png',
  width: 1024,
  height: 1024,
  bytes: 100,
  inLibrary: true,
  textContent: null,
  origin: null,
  originalUrl: `/assets/${id}`,
  thumbnailUrl: `/assets/${id}/thumbnail`,
  createdAt: 1,
  updatedAt: 1,
});

describe('standalone workbench draft asset hydration', () => {
  test('hydrates every image reference by exact ID and preserves draft order', async () => {
    const calls: string[] = [];
    const result = await hydrateStandaloneWorkbenchDraftReferences(
      'image',
      [ASSET_A, ASSET_B],
      {
        async get(assetId) {
          calls.push(assetId);
          if (assetId === ASSET_A) {
            await new Promise((resolve) => setTimeout(resolve, 2));
          }
          return asset(assetId);
        },
      }
    );

    expect(calls).toEqual([ASSET_A, ASSET_B]);
    expect(result.retainedReferenceAssetIds).toEqual([ASSET_A, ASSET_B]);
    expect(result.assets.map((item) => item.id)).toEqual([ASSET_A, ASSET_B]);
    expect(result.unavailableReferenceAssetIds).toEqual([]);
  });

  test('drops missing, wrong-kind, and mismatched assets without inventing placeholders', async () => {
    const result = await hydrateStandaloneWorkbenchDraftReferences(
      'image',
      [ASSET_A, ASSET_B, ASSET_C, ASSET_D],
      {
        async get(assetId) {
          if (assetId === ASSET_B) throw new Error('404');
          if (assetId === ASSET_C) return asset(assetId, 'video');
          if (assetId === ASSET_D) return asset(ASSET_A);
          return asset(assetId);
        },
      }
    );

    expect(result.assets).toEqual([asset(ASSET_A)]);
    expect(result.retainedReferenceAssetIds).toEqual([ASSET_A]);
    expect(result.unavailableReferenceAssetIds).toEqual([
      ASSET_B,
      ASSET_C,
      ASSET_D,
    ]);
  });

  test('keeps video restoration at one real image and fails closed for malformed IDs', async () => {
    const calls: string[] = [];
    const malformed = 'asset-not-a-uuid';
    const result = await hydrateStandaloneWorkbenchDraftReferences(
      'video',
      [ASSET_A, ASSET_B, malformed],
      {
        async get(assetId) {
          calls.push(assetId);
          return asset(assetId);
        },
      }
    );

    expect(calls).toEqual([ASSET_A]);
    expect(result.retainedReferenceAssetIds).toEqual([ASSET_A]);
    expect(result.unavailableReferenceAssetIds).toEqual([ASSET_B, malformed]);
  });

  test('never substitutes a same-named model from a different Provider', () => {
    const selected = { providerId: PROVIDER_A, model: 'exact-model' };

    expect(
      isExactWorkbenchDraftModelAvailable(selected, [
        { providerId: PROVIDER_A, model: 'exact-model' },
      ])
    ).toBe(true);
    expect(
      isExactWorkbenchDraftModelAvailable(selected, [
        { providerId: PROVIDER_B, model: 'exact-model' },
        { providerId: PROVIDER_A, model: 'other-model' },
      ])
    ).toBe(false);
    expect(isExactWorkbenchDraftModelAvailable(selected, [])).toBe(false);
  });
});
