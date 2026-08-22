/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { describe, expect, test } from 'bun:test';

import type { PromptLibraryItem } from '../types';
import { copyStandalonePrompt } from './standaloneSelection';

const item: PromptLibraryItem = {
  id: 'prompt-real-1',
  source: 'asset',
  title: '产品静物提示词',
  description: null,
  prompt: '在柔和侧光下拍摄真实产品静物。',
  category: '商业摄影',
  tags: ['静物', '产品'],
  knowledgeBaseIds: ['knowledge-1'],
  coverUrl: null,
  preview: null,
  sourceUrl: null,
  license: null,
  licenseUrl: null,
  createdAt: 1_760_000_000,
  updatedAt: 1_770_000_000,
};

describe('standalone prompt selection', () => {
  test('copies the validated prompt verbatim and returns the shared selection contract', async () => {
    const written: string[] = [];
    const selection = await copyStandalonePrompt(item, async (text) => {
      written.push(text);
    });

    expect(written).toEqual(['在柔和侧光下拍摄真实产品静物。']);
    expect(selection).toEqual({
      id: 'prompt-real-1',
      source: 'asset',
      title: '产品静物提示词',
      prompt: '在柔和侧光下拍摄真实产品静物。',
      category: '商业摄影',
      tags: ['静物', '产品'],
      knowledgeBaseIds: ['knowledge-1'],
      coverUrl: null,
      sourceUrl: null,
      license: null,
      licenseUrl: null,
    });
  });

  test('does not report a selection when the clipboard write fails', async () => {
    const failure = new Error('clipboard denied');
    let caught: unknown;
    try {
      await copyStandalonePrompt(item, async () => {
        throw failure;
      });
    } catch (reason) {
      caught = reason;
    }
    expect(caught).toBe(failure);
  });
});
