/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { describe, expect, test } from 'bun:test';

import { parseAssetId } from '@/common/types/ids';

import type { CreativeAsset } from './types';
import { creativeAssetMatchesQuery } from './useCreativeAssets';

const ASSET: CreativeAsset = {
  id: parseAssetId('0190f5fe-7c00-7a00-8000-000000000001'),
  kind: 'image',
  title: 'Golden hour portrait',
  collection: 'People',
  tags: ['warm', 'portrait'],
  mimeType: 'image/png',
  width: 100,
  height: 100,
  bytes: 10,
  inLibrary: true,
  textContent: null,
  origin: null,
  originalUrl: '/asset',
  thumbnailUrl: '/thumb',
  createdAt: 1,
  updatedAt: 1,
};

describe('creativeAssetMatchesQuery', () => {
  test('matches kind, collection, tag, library and search filters together', () => {
    expect(
      creativeAssetMatchesQuery(ASSET, {
        kind: 'image',
        collection: 'People',
        tag: 'warm',
        inLibrary: true,
        search: 'HOUR PORTRAIT',
      })
    ).toBe(true);
    expect(creativeAssetMatchesQuery(ASSET, { kind: 'video' })).toBe(false);
    expect(creativeAssetMatchesQuery(ASSET, { tag: 'cold' })).toBe(false);
  });

  test('treats the ungrouped filter as mutually exclusive with collection', () => {
    expect(creativeAssetMatchesQuery(ASSET, { ungrouped: true })).toBe(false);
    expect(creativeAssetMatchesQuery({ ...ASSET, collection: null }, { ungrouped: true })).toBe(true);
  });
});
