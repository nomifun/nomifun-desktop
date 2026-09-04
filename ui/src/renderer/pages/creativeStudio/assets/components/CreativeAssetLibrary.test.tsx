/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { describe, expect, test } from 'bun:test';
import i18next from 'i18next';
import { readFileSync } from 'node:fs';
import React from 'react';
import { renderToStaticMarkup } from 'react-dom/server';
import { I18nextProvider, initReactI18next } from 'react-i18next';

import type { CreativeAsset } from '../types';
import CreativeAssetLibrary, { submitCreativeAssetLibrarySearch } from './CreativeAssetLibrary';
import CreativeAssetUploadQueue from './CreativeAssetUploadQueue';
import { createCreativeAssetLibraryLabels } from './types';
import type { CreativeAssetLibraryState } from './types';

const testI18n = i18next.createInstance();
await testI18n.use(initReactI18next).init({
  lng: 'zh-CN',
  fallbackLng: 'zh-CN',
  resources: { 'zh-CN': { translation: {} } },
});
const testLabels = createCreativeAssetLibraryLabels(testI18n.t.bind(testI18n));

const asset = (id: string, kind: CreativeAsset['kind']): CreativeAsset => ({
  id,
  kind,
  title: `${kind} asset`,
  collection: 'Campaign',
  tags: ['hero', kind],
  mimeType: kind === 'text' ? null : `${kind}/example`,
  width: kind === 'image' || kind === 'video' ? 1280 : null,
  height: kind === 'image' || kind === 'video' ? 720 : null,
  bytes: kind === 'text' ? null : 2048,
  inLibrary: true,
  textContent: kind === 'text' ? 'A reusable creative prompt' : null,
  origin: null,
  originalUrl: `/api/creative-studio/files/${id}`,
  thumbnailUrl: kind === 'image' ? `/api/creative-studio/files/${id}?thumb=1` : null,
  createdAt: 1_777_000_000_000,
  updatedAt: 1_777_000_000_000,
});

const assets = (['image', 'video', 'audio', 'text'] as const).map((kind, index) => asset(`asset-${index}`, kind));

const state = (overrides: Partial<CreativeAssetLibraryState> = {}): CreativeAssetLibraryState => ({
  assets,
  total: assets.length,
  loading: false,
  loadingMore: false,
  mutating: false,
  error: null,
  mutationError: null,
  hasMore: false,
  reload: async () => undefined,
  loadMore: async () => undefined,
  ...overrides,
});

const renderLibrary = (overrides: Partial<React.ComponentProps<typeof CreativeAssetLibrary>> = {}) =>
  renderToStaticMarkup(
    <I18nextProvider i18n={testI18n}>
      <CreativeAssetLibrary
        state={state()}
        search=''
        kind='all'
        scope='library'
        view='grid'
        selectedIds={new Set(['asset-0', 'asset-1'])}
        onSearchChange={() => undefined}
        onKindChange={() => undefined}
        onScopeChange={() => undefined}
        onViewChange={() => undefined}
        onSelectionChange={() => undefined}
        onUploadFiles={() => undefined}
        onCreateText={() => undefined}
        onOpenAsset={() => undefined}
        onEditAsset={() => undefined}
        onDownloadAsset={() => undefined}
        onRemoveAsset={() => undefined}
        onSetSelectedLibrary={() => undefined}
        onInsertSelected={() => undefined}
        onDownloadSelected={() => undefined}
        onRemoveSelected={() => undefined}
        {...overrides}
      />
    </I18nextProvider>
  );

describe('CreativeAssetLibrary', () => {
  test('renders the controlled source-aligned library surface and every media kind', () => {
    const html = renderLibrary();

    expect(html.includes('data-creative-asset-library="true"')).toBe(true);
    expect(html.includes('data-asset-scope="library"')).toBe(true);
    expect(html.includes('data-asset-view="grid"')).toBe(true);
    for (const kind of ['image', 'video', 'audio', 'text']) {
      expect(html.includes(`data-asset-kind="${kind}"`)).toBe(true);
    }
    expect(html.includes('/api/creative-studio/files/asset-0?thumb=1')).toBe(true);
    expect(html.includes('data-asset-media-state="audio"')).toBe(true);
    expect(html.includes('audio/example')).toBe(true);
    expect(html.includes('A reusable creative prompt')).toBe(true);
  });

  test('exposes selection and multi-action intent without owning selected state', () => {
    const html = renderLibrary();

    expect(html.includes('data-asset-selection-bar="true"')).toBe(true);
    expect(html.includes('已选择 2 项')).toBe(true);
    for (const label of ['移出素材库', '插入画布', '下载', '删除']) {
      expect(html.includes(label)).toBe(true);
    }
    expect(html.match(/type="checkbox"/g)?.length).toBe(4);
  });

  test('switches to the controlled list presentation without changing asset identity', () => {
    const html = renderLibrary({ view: 'list', selectedIds: new Set() });
    expect(html.includes('data-asset-view="list"')).toBe(true);
    expect(html.match(/data-asset-id=/g)?.length).toBe(4);
    expect(html.match(/<time /g)?.length).toBe(4);
    expect(html.includes('aria-label="素材类型"')).toBe(true);
    expect(html.includes('aria-label="素材范围"')).toBe(true);
    expect(html.includes('aria-label="显示方式"')).toBe(true);
  });

  test('distinguishes loading, error, plain empty and filtered empty states', () => {
    const loading = renderLibrary({ state: state({ assets: [], total: 0, loading: true }), selectedIds: new Set() });
    expect(loading.includes('data-asset-state="loading"')).toBe(true);
    expect(loading.includes('aria-busy="true"')).toBe(true);

    const error = renderLibrary({
      state: state({ assets: [], total: 0, error: new Error('backend unavailable') }),
      selectedIds: new Set(),
    });
    expect(error.includes('data-asset-state="error"')).toBe(true);
    expect(error.includes('backend unavailable')).toBe(true);

    const empty = renderLibrary({ state: state({ assets: [], total: 0 }), selectedIds: new Set() });
    expect(empty.includes(testLabels.emptyTitle)).toBe(true);

    const canvasEmpty = renderLibrary({
      state: state({ assets: [], total: 0 }),
      scope: 'canvas',
      selectedIds: new Set(),
    });
    expect(canvasEmpty.includes(testLabels.canvasEmptyTitle)).toBe(true);

    const filtered = renderLibrary({
      state: state({ assets: [], total: 0 }),
      search: 'missing',
      selectedIds: new Set(),
    });
    expect(filtered.includes(testLabels.filteredEmptyTitle)).toBe(true);
  });

  test('renders typed upload progress, completion and failure records', () => {
    const html = renderToStaticMarkup(
      <I18nextProvider i18n={testI18n}>
        <CreativeAssetUploadQueue
          labels={testLabels}
          items={[
            { id: 'a', fileName: 'upload.png', percent: 42, status: 'uploading' },
            { id: 'b', fileName: 'complete.mp4', percent: 100, status: 'completed' },
            { id: 'c', fileName: 'failed.png', percent: 17, status: 'error', error: 'too large' },
          ]}
          onCancel={() => undefined}
          onRetry={() => undefined}
          onDismiss={() => undefined}
        />
      </I18nextProvider>
    );

    expect(html.includes('data-asset-upload-queue="true"')).toBe(true);
    expect(html.includes('aria-valuenow="42"')).toBe(true);
    expect(html.includes('data-upload-status="completed"')).toBe(true);
    expect(html.includes('too large')).toBe(true);
  });

  test('renders the explicit source-page appearance without changing the default panel contract', () => {
    const html = renderLibrary({
      appearance: 'source-page',
      selectable: false,
      selectedIds: new Set(),
      labels: {
        title: '我的素材',
        description: '收藏常用素材，按类型和标题快速查找。',
        kindFilter: '类型',
      },
      uploadHint: '图片和视频，单文件最大 64 MB',
      pagination: {
        page: 1,
        pageSize: 10,
        total: 14,
        onPageChange: () => undefined,
      },
      onRenameCollection: () => undefined,
    });

    expect(html.includes('data-asset-appearance="source-page"')).toBe(true);
    expect(html.includes('<h1>我的素材</h1>')).toBe(true);
    expect(html.includes('role="search"')).toBe(true);
    expect(html.includes('type="search" aria-label="搜索"')).toBe(true);
    expect(html.includes('aria-label="素材范围"')).toBe(false);
    expect(html.includes('aria-label="显示方式"')).toBe(false);
    expect(html.includes('type="checkbox"')).toBe(false);
    expect(html.match(/aria-haspopup="menu"/g)?.length).toBe(assets.length);
    expect(html.match(/aria-expanded="false"/g)?.length).toBe(assets.length);
    expect(html.includes('role="menuitem"')).toBe(false);
    for (const item of assets) {
      expect(html.includes(`aria-label="更多：${item.title}"`)).toBe(true);
    }
    const imageCard = html.match(/<article\b[^>]*data-asset-id="asset-0"[\s\S]*?<\/article>/)?.[0] ?? '';
    const footer = imageCard.match(/<footer\b[\s\S]*?<\/footer>/)?.[0] ?? '';
    expect(footer.includes('1280 × 720')).toBe(true);
    expect(footer.includes('2.0 KB')).toBe(true);
    expect(footer.includes('image/example')).toBe(true);
    expect(footer.includes('aria-haspopup="menu"')).toBe(true);
    expect(html.includes('重命名合集')).toBe(true);
    expect(html.includes('图片和视频，单文件最大 64 MB')).toBe(true);
    expect(html.includes('aria-label="素材分页"')).toBe(true);
    expect(html.includes('10 条/页')).toBe(true);
    expect(html.indexOf('>文本</button>')).toBeLessThan(html.indexOf('>图片</button>'));
  });

  test('fully disables selection and source pagination outside their explicit contracts', () => {
    const nonSelectable = renderLibrary({
      selectable: false,
      selectedIds: new Set(['asset-0', 'asset-1']),
    });
    expect(nonSelectable.includes('type="checkbox"')).toBe(false);
    expect(nonSelectable.includes('data-asset-selection-bar')).toBe(false);

    const defaultWithSourcePaginationProps = renderLibrary({
      pagination: { page: 1, pageSize: 10, total: 14, onPageChange: () => undefined },
    });
    expect(defaultWithSourcePaginationProps.includes('aria-label="素材分页"')).toBe(false);
  });

  test('submits the controlled search immediately for Enter and the search button', () => {
    let prevented = false;
    let submitted = '';
    submitCreativeAssetLibrarySearch(
      { preventDefault: () => { prevented = true; } },
      'hero title',
      (value) => { submitted = value; }
    );
    expect(prevented).toBe(true);
    expect(submitted).toBe('hero title');
  });

  test('keeps compact and reduced-motion layouts explicit', () => {
    const css = readFileSync(new URL('./CreativeAssetLibrary.module.css', import.meta.url), 'utf8');
    expect(css.includes('@media (max-width: 820px)')).toBe(true);
    expect(css.includes('@media (max-width: 560px)')).toBe(true);
    expect(css.includes('@media (hover: none)')).toBe(true);
    expect(css.includes('@media (prefers-reduced-motion: reduce)')).toBe(true);
    expect(css.includes("[data-asset-appearance='source-page']")).toBe(true);
  });
});
