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
    expect(source.includes('useCreativeAssets({ client, query })')).toBe(true);
    expect(source.includes('<CreativeAssetLibrary')).toBe(true);
    expect(source.includes('<CreateCreativeTextAssetModal')).toBe(true);
    expect(source.includes('asset.originalUrl')).toBe(true);
    expect(source.includes('onSetSelectedLibrary=')).toBe(false);
    expect(source.includes('onInsertSelected=')).toBe(false);
    expect(source.includes('onDownloadSelected=')).toBe(false);
    expect(source.includes('onRemoveSelected=')).toBe(false);
  });

  test('keeps unsupported global scope and selection controls explicitly absent', () => {
    const css = readFileSync(new URL('./CreativeAssetLibraryPage.module.css', import.meta.url), 'utf8');
    expect(css.includes("[aria-label='creative-studio-global-scope-fixed']")).toBe(true);
    expect(css.includes('[data-asset-selection-bar]')).toBe(true);
    expect(css.includes('[data-asset-id] > label:first-child')).toBe(true);
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
});
