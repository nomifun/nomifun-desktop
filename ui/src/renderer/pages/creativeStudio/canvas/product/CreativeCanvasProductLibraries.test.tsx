/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { describe, expect, test } from 'bun:test';
import React from 'react';
import { renderToStaticMarkup } from 'react-dom/server';

import type { CreativeAsset, UseCreativeAssetsResult } from '../../assets';
import { CreativeCanvasProductAssetLibrary } from './CreativeCanvasProductLibraries';

const asset: CreativeAsset = {
  id: 'asset-real-1',
  kind: 'image',
  title: '海边参考图',
  collection: '参考',
  tags: ['场景'],
  mimeType: 'image/png',
  width: 1280,
  height: 720,
  bytes: 2048,
  inLibrary: true,
  textContent: null,
  origin: null,
  originalUrl: 'nomifun://asset/asset-real-1/original',
  thumbnailUrl: 'nomifun://asset/asset-real-1/thumbnail',
  createdAt: 1,
  updatedAt: 2,
};

const state = (overrides: Partial<UseCreativeAssetsResult> = {}): UseCreativeAssetsResult => ({
  assets: [asset],
  total: 1,
  loading: false,
  loadingMore: false,
  mutating: false,
  error: null,
  mutationError: null,
  hasMore: false,
  reload: async () => undefined,
  loadMore: async () => undefined,
  upload: async () => asset,
  createText: async () => asset,
  update: async () => asset,
  remove: async () => undefined,
  renameCollection: async () => 0,
  url: () => asset.originalUrl,
  ...overrides,
});

describe('CreativeCanvasProductAssetLibrary', () => {
  test('renders authoritative records and enables insertion only for real selection', () => {
    const markup = renderToStaticMarkup(
      <CreativeCanvasProductAssetLibrary
        state={state()}
        search=''
        kind='all'
        selectedIds={new Set([asset.id])}
        onSearchChange={() => undefined}
        onKindChange={() => undefined}
        onToggleAsset={() => undefined}
        onInsert={() => undefined}
      />
    );

    expect(markup.includes('data-product-asset-library')).toBe(true);
    expect(markup.includes('海边参考图')).toBe(true);
    expect(asset.thumbnailUrl !== null && markup.includes(asset.thumbnailUrl)).toBe(true);
    expect(markup.includes('aria-pressed="true"')).toBe(true);
    expect(markup.includes('1 项真实素材')).toBe(true);
    expect(markup.includes('fake')).toBe(false);
  });

  test('shows the real port error without claiming an empty successful library', () => {
    const markup = renderToStaticMarkup(
      <CreativeCanvasProductAssetLibrary
        state={state({ assets: [], total: 0, error: new Error('backend unavailable') })}
        search=''
        kind='all'
        selectedIds={new Set()}
        onSearchChange={() => undefined}
        onKindChange={() => undefined}
        onToggleAsset={() => undefined}
        onInsert={() => undefined}
      />
    );
    expect(markup.includes('data-state="error"')).toBe(true);
    expect(markup.includes('backend unavailable')).toBe(true);
    expect(markup.includes('素材库加载失败')).toBe(true);
  });
});
