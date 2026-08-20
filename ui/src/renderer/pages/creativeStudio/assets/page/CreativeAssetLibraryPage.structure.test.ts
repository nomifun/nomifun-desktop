/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { describe, expect, test } from 'bun:test';
import { readFileSync } from 'node:fs';

describe('CreativeAssetLibraryPage wiring', () => {
  test('composes the canonical asset client, hook and presentation without fake batch callbacks', () => {
    const source = readFileSync(new URL('./CreativeAssetLibraryPage.tsx', import.meta.url), 'utf8');
    expect(source.includes("client = creativeAssetClient")).toBe(true);
    expect(source.includes('useCreativeAssets({ client, query, pageSize: SOURCE_ASSET_PAGE_SIZE })')).toBe(true);
    expect(source.includes('<CreativeAssetLibrary')).toBe(true);
    expect(source.includes("appearance='source-page'")).toBe(true);
    expect(source.includes('selectable={false}')).toBe(true);
    expect(source.includes('pagination={{')).toBe(true);
    expect(source.includes('<CreateCreativeTextAssetModal')).toBe(true);
    expect(source.includes('asset.originalUrl')).toBe(true);
    expect(source.includes('onSetSelectedLibrary=')).toBe(false);
    expect(source.includes('onInsertSelected=')).toBe(false);
    expect(source.includes('onDownloadSelected=')).toBe(false);
    expect(source.includes('onRemoveSelected=')).toBe(false);
  });

  test('matches the measured source-page geometry without changing default component styles', () => {
    const css = readFileSync(new URL('../components/CreativeAssetLibrary.module.css', import.meta.url), 'utf8');
    expect(css.includes('width: min(1152px, calc(100% - 48px))')).toBe(true);
    expect(css.includes('width: 672px')).toBe(true);
    expect(css.includes('font-size: 36px')).toBe(true);
    expect(css.includes(".root[data-asset-appearance='source-page'] .statePanel")).toBe(true);
    expect(css.includes('.sourcePagination')).toBe(true);
  });

  test('wires cancellable progress uploads instead of optimistic completion', () => {
    const source = readFileSync(new URL('./useCreativeAssetUploadQueue.ts', import.meta.url), 'utf8');
    expect(source.includes('new AbortController()')).toBe(true);
    expect(source.includes('controller.signal')).toBe(true);
    expect(source.includes("dispatch({ type: 'progress'")).toBe(true);
    expect(source.includes(".then(() => {")).toBe(true);
    expect(source.includes("dispatch({ type: 'complete'")).toBe(true);
    expect(source.includes('controller.abort()')).toBe(true);
  });

  test('synchronizes every backend page before client slicing and flushes submitted search', () => {
    const source = readFileSync(new URL('./CreativeAssetLibraryPage.tsx', import.meta.url), 'utf8');
    expect(source.includes('creativeAssetCacheIsComplete(library.assets.length, library.total)')).toBe(true);
    expect(source.includes('library.error || library.loading || library.loadingMore || !library.hasMore')).toBe(true);
    expect(source.includes('void library.loadMore()')).toBe(true);
    expect(source.includes('creativeAssetPageSliceFromCompleteCache(')).toBe(true);
    expect(source.includes('setSubmittedSearch(value)')).toBe(true);
    expect(source.includes('setPendingPage')).toBe(false);
  });
});
