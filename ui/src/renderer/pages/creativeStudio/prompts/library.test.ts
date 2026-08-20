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
  updatedAt: null,
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
  updatedAt: 20,
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
      tags: ['分镜', '构图', '叙事'],
      hasUncategorized: true,
    });
    const selection = toPromptLibrarySelection(FIRST);
    selection.tags.push('new');
    selection.knowledgeBaseIds.push('knowledge-two');
    expect(FIRST.tags).toEqual(['分镜', '叙事']);
    expect(FIRST.knowledgeBaseIds).toEqual(['knowledge-one']);
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
