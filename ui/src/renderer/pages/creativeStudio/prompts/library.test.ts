/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { describe, expect, test } from 'bun:test';

import {
  filterPromptLibraryItems,
  normalizePromptLibrary,
  promptLibraryFacets,
  sortPromptLibraryItemsByUpdatedAt,
  toPromptLibrarySelection,
} from './library';
import type { PromptLibraryItem } from './types';

const FIRST: PromptLibraryItem = {
  id: 'prompt-one',
  source: 'preset',
  title: '分镜构思',
  description: '整理镜头节奏',
  prompt: '请根据故事目标整理分镜结构。',
  category: '视频创作',
  tags: ['分镜', '叙事'],
  knowledgeBaseIds: ['knowledge-one'],
  coverUrl: null,
  preview: null,
  sourceUrl: null,
  license: null,
  licenseUrl: null,
  createdAt: null,
  updatedAt: null,
  savedToAssets: false,
};

const SECOND: PromptLibraryItem = {
  id: 'prompt-two',
  source: 'asset',
  title: '画面说明',
  description: null,
  prompt: '描述画面的主体、光线与构图。',
  category: null,
  tags: ['构图'],
  knowledgeBaseIds: [],
  coverUrl: null,
  preview: null,
  sourceUrl: null,
  license: null,
  licenseUrl: null,
  createdAt: 10,
  updatedAt: 20,
  savedToAssets: true,
};

describe('prompt library validation and filtering', () => {
  test('keeps only valid, unique records from an untrusted port response', () => {
    const result = normalizePromptLibrary([
      FIRST,
      { ...FIRST },
      { ...SECOND, prompt: '   ' },
      SECOND,
      { ...SECOND, id: 'bad\u0000id' },
    ]);

    expect(result.items).toEqual([FIRST, SECOND]);
    expect(result.invalidCount).toBe(3);
  });

  test('defaults saved membership for older injected prompt ports', () => {
    const { savedToAssets: _savedToAssets, ...legacyItem } = FIRST;
    expect(normalizePromptLibrary([legacyItem]).items[0]?.savedToAssets).toBe(false);
  });

  test('keeps equal raw IDs from different source namespaces', () => {
    const catalog = { ...FIRST, source: 'catalog' as const };
    expect(normalizePromptLibrary([FIRST, catalog]).items).toEqual([FIRST, catalog]);
  });

  test('filters by text, exact category and intersected tags', () => {
    const items = [FIRST, SECOND];
    expect(filterPromptLibraryItems(items, { query: '镜头 节奏' })).toEqual([FIRST]);
    expect(filterPromptLibraryItems(items, { category: null })).toEqual([SECOND]);
    expect(filterPromptLibraryItems(items, { tags: ['分镜', '叙事'] })).toEqual([FIRST]);
    expect(filterPromptLibraryItems(items, { tags: ['分镜', '构图'] })).toEqual([]);
  });

  test('derives legal facets and produces a detached insertion snapshot', () => {
    expect(promptLibraryFacets([FIRST, SECOND])).toEqual({
      categories: ['视频创作'],
      tags: ['分镜', '叙事', '构图'],
      hasUncategorized: true,
    });
    const selection = toPromptLibrarySelection(FIRST);
    selection.tags.push('new');
    selection.knowledgeBaseIds.push('knowledge-two');
    expect(FIRST.tags).toEqual(['分镜', '叙事']);
    expect(FIRST.knowledgeBaseIds).toEqual(['knowledge-one']);
  });

  test('sorts cards newest-first and preserves null-date source order', () => {
    const older = { ...FIRST, id: 'older', updatedAt: 100 };
    const newer = { ...FIRST, id: 'newer', updatedAt: 200 };
    const undatedOne = { ...FIRST, id: 'undated-one', createdAt: null, updatedAt: null };
    const undatedTwo = { ...FIRST, id: 'undated-two', createdAt: null, updatedAt: null };
    expect(
      sortPromptLibraryItemsByUpdatedAt([undatedOne, older, undatedTwo, newer]).map(
        (item) => item.id
      )
    ).toEqual(['newer', 'older', 'undated-one', 'undated-two']);
  });

  test('rejects a non-array response instead of guessing a wire shape', () => {
    let error: unknown;
    try {
      normalizePromptLibrary({ items: [FIRST] });
    } catch (reason) {
      error = reason;
    }
    expect(error instanceof TypeError).toBe(true);
    expect((error as Error).message).toBe('Prompt library response must be an array');
  });
});
