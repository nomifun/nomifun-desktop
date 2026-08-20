/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { describe, expect, test } from 'bun:test';
import React from 'react';
import { renderToStaticMarkup } from 'react-dom/server';

import type { CreativeAssetLibraryPort } from '../types';
import CreativeAssetLibraryPage from './CreativeAssetLibraryPage';

const inertClient: CreativeAssetLibraryPort = {
  list: async () => ({ items: [], total: 0 }),
  upload: async () => { throw new Error('not invoked during SSR'); },
  createText: async () => { throw new Error('not invoked during SSR'); },
  update: async () => { throw new Error('not invoked during SSR'); },
  remove: async () => undefined,
  renameCollection: async () => 0,
  url: (assetId, variant = 'original') => `/api/workshop/files/${assetId}${variant === 'thumbnail' ? '?thumb=1' : ''}`,
};

describe('CreativeAssetLibraryPage', () => {
  test('renders the source-aligned global asset surface with honest upload limits', () => {
    const html = renderToStaticMarkup(<CreativeAssetLibraryPage client={inertClient} locale='zh-CN' />);
    expect(html.includes('data-creative-asset-library-page="true"')).toBe(true);
    expect(html.includes('data-creative-asset-library="true"')).toBe(true);
    expect(html.includes('data-asset-scope="library"')).toBe(true);
    expect(html.includes('单文件最大 64 MB')).toBe(true);
    expect(html.includes('暂不支持手动上传音频')).toBe(true);
    expect(html.includes('accept="image/*,video/*"')).toBe(true);
    expect(html.includes('重命名合集')).toBe(true);
  });
});
