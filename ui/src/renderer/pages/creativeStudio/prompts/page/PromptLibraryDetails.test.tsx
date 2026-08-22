/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { describe, expect, test } from 'bun:test';
import React from 'react';
import { renderToStaticMarkup } from 'react-dom/server';

import type { PromptLibraryItem } from '../types';
import { PromptLibraryDetailsContent } from './PromptLibraryDetails';

const renderDetails = (item: PromptLibraryItem) =>
  renderToStaticMarkup(<PromptLibraryDetailsContent item={item} locale='zh-CN' />);

describe('prompt-library standalone details', () => {
  test('renders the full validated preset without inventing a preview asset', () => {
    const html = renderDetails({
      id: 'preset-1',
      source: 'preset',
      title: '品牌海报',
      description: '面向品牌视觉创作。',
      prompt: '生成一张具有明确视觉层级的品牌海报。',
      category: '品牌设计',
      tags: ['海报', '品牌'],
      knowledgeBaseIds: ['kb-1', 'kb-2'],
      coverUrl: null,
      preview: null,
      sourceUrl: null,
      license: null,
      licenseUrl: null,
      createdAt: null,
      updatedAt: null,
    });

    expect(html.includes('data-prompt-library-details="true"')).toBe(true);
    expect(html.includes('data-prompt-source="preset"')).toBe(true);
    expect(html.includes('NomiFun 预设')).toBe(true);
    expect(html.includes('完整提示词')).toBe(true);
    expect(html.includes('生成一张具有明确视觉层级的品牌海报。')).toBe(true);
    expect(html.includes('关联 2 个知识库')).toBe(true);
    expect(html.includes('<img')).toBe(false);
  });

  test('identifies user-owned text assets honestly', () => {
    const html = renderDetails({
      id: 'asset-1',
      source: 'asset',
      title: '我的提示词',
      description: null,
      prompt: '保留真实材质与自然阴影。',
      category: null,
      tags: [],
      knowledgeBaseIds: [],
      coverUrl: null,
      preview: null,
      sourceUrl: null,
      license: null,
      licenseUrl: null,
      createdAt: 1_760_000_000,
      updatedAt: 1_770_000_000,
    });

    expect(html.includes('我的文本素材')).toBe(true);
    expect(html.includes('未分类')).toBe(true);
    expect(html.includes('保留真实材质与自然阴影。')).toBe(true);
  });

  test('renders catalog media and auditable source attribution', () => {
    const html = renderDetails({
      id: 'catalog-1',
      source: 'catalog',
      title: '纸雕海报',
      description: null,
      prompt: 'Create a paper poster.',
      category: 'awesome-gpt-image',
      tags: ['poster'],
      knowledgeBaseIds: [],
      coverUrl: 'https://raw.githubusercontent.com/example/paper.jpg',
      preview: '![](https://raw.githubusercontent.com/example/paper.jpg)\n\n来源说明',
      sourceUrl: 'https://github.com/ZeroLu/awesome-gpt-image',
      license: 'MIT',
      licenseUrl: 'https://github.com/ZeroLu/awesome-gpt-image/blob/main/LICENSE',
      createdAt: null,
      updatedAt: null,
    });

    expect(html.includes('data-prompt-source="catalog"')).toBe(true);
    expect(html.includes('公共提示词目录')).toBe(true);
    expect(html.includes('<img')).toBe(true);
    expect(html.includes('查看来源')).toBe(true);
    expect(html.includes('MIT')).toBe(true);
    expect(html.includes('来源说明')).toBe(true);
  });
});
