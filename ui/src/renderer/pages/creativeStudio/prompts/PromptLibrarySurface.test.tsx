/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { describe, expect, test } from 'bun:test';
import React from 'react';
import { renderToStaticMarkup } from 'react-dom/server';

import { PromptLibrarySurface } from './PromptLibrarySurface';
import type { PromptLibraryItem } from './types';

const ITEM: PromptLibraryItem = {
  id: 'prompt-one',
  source: 'preset',
  title: '场景拆解',
  description: '将创作目标拆成可执行画面',
  prompt: '提取主体、环境、光线和镜头关系。',
  category: '视觉创作',
  tags: ['构图', '场景'],
  knowledgeBaseIds: ['knowledge-one'],
  updatedAt: null,
};

const render = (props: Partial<React.ComponentProps<typeof PromptLibrarySurface>> = {}) =>
  renderToStaticMarkup(
    <PromptLibrarySurface variant='page' items={[ITEM]} {...props} />
  );

describe('PromptLibrarySurface', () => {
  test('renders a validated page card, facets and insertion affordance', () => {
    const html = render({ onInsert: () => undefined });
    expect(html.includes('data-prompt-library="page"')).toBe(true);
    expect(html.includes('data-prompt-library-item="prompt-one"')).toBe(true);
    expect(html.includes('场景拆解')).toBe(true);
    expect(html.includes('视觉创作')).toBe(true);
    expect(html.includes('构图')).toBe(true);
    expect(html.includes('插入提示词：场景拆解')).toBe(true);
    expect(html.includes('关联 1 个知识库')).toBe(true);
  });

  test('renders compact sidebar chrome from the same controlled data', () => {
    const html = render({ variant: 'sidebar', title: '灵感提示' });
    expect(html.includes('data-prompt-library="sidebar"')).toBe(true);
    expect(html.includes('灵感提示')).toBe(true);
  });

  test('renders loading, empty and error states explicitly', () => {
    expect(render({ items: [], loading: true }).includes('data-prompt-library-state="loading"')).toBe(
      true
    );
    expect(render({ items: [] }).includes('data-prompt-library-state="empty"')).toBe(true);
    const error = render({ items: [], error: new Error('服务暂不可用'), onRetry: () => undefined });
    expect(error.includes('data-prompt-library-state="error"')).toBe(true);
    expect(error.includes('服务暂不可用')).toBe(true);
    expect(error.includes('重新加载')).toBe(true);
  });

  test('reports rejected records without rendering their content', () => {
    const html = render({ invalidCount: 2 });
    expect(html.includes('已忽略 2 条不符合数据契约的记录')).toBe(true);
    expect(html.includes('external prompt marketplace')).toBe(false);
  });
});
