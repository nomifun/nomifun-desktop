/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { describe, expect, test } from 'bun:test';
import { creativeAssetDisplayTitle } from './presentation';

const prompt = '纯白色纸张破洞视角，洞边缘有撕纸纤维质感，蓝色头发的小女孩从洞中探出。'.repeat(3);
const asset = { id: 'asset-1', title: '', origin: { prompt }, textContent: null };

describe('creative asset display titles', () => {
  test('shortens empty and historical generated titles while retaining the complete prompt', () => {
    const expected = '纯白色纸张破洞视角，洞边缘有撕';
    expect(creativeAssetDisplayTitle(asset)).toBe(expected);
    const historical = { ...asset, title: Array.from(prompt).slice(0, 60).join('') };
    expect(creativeAssetDisplayTitle(historical)).toBe(expected);
    expect(historical.origin.prompt).toBe(prompt);
    expect(Array.from(historical.title)).toHaveLength(60);
  });

  test('preserves custom names, even when they begin with the prompt', () => {
    for (const title of ['手动命名的作品标题不受十五字默认长度限制', prompt.slice(0, 20), `${prompt.slice(0, 60)} 版本二`]) {
      expect(creativeAssetDisplayTitle({ ...asset, title })).toBe(title);
    }
  });

  test('keeps catalog names and falls back to text content without splitting emoji', () => {
    const title = prompt.slice(0, 60);
    expect(creativeAssetDisplayTitle({ ...asset, title, origin: { prompt, promptCatalogId: 'catalog-1' } })).toBe(title);
    expect(creativeAssetDisplayTitle({ ...asset, title: '  ', origin: null, textContent: '👩🏽‍🎨'.repeat(16) })).toBe('👩🏽‍🎨'.repeat(15));
    expect(creativeAssetDisplayTitle({ ...asset, origin: null, textContent: '短描述' })).toBe('短描述');
    expect(creativeAssetDisplayTitle({ ...asset, origin: null })).toBe(asset.id);
  });
});
