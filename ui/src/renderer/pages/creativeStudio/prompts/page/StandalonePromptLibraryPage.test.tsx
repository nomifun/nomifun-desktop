/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { describe, expect, test } from 'bun:test';
import { readFileSync } from 'node:fs';
import React from 'react';
import { renderToStaticMarkup } from 'react-dom/server';

import type { PromptLibraryItem } from '../types';
import { StandalonePromptLibraryAppearance } from './StandalonePromptLibraryPage';

const ITEM: PromptLibraryItem = {
  id: 'prompt-1',
  source: 'preset',
  title: '镜头规划',
  description: '整理画面节奏',
  prompt: '按故事目标规划镜头。',
  category: '视频创作',
  tags: ['分镜', '节奏'],
  knowledgeBaseIds: [],
  updatedAt: null,
};

const render = (
  props: Partial<React.ComponentProps<typeof StandalonePromptLibraryAppearance>> = {}
) =>
  renderToStaticMarkup(
    <StandalonePromptLibraryAppearance items={[]} {...props} />
  );

describe('standalone prompt-library source-parity appearance', () => {
  test('centers the real count header and keeps loading state minimal', () => {
    const html = render({ loading: true, onRetry: () => undefined });
    expect(html.includes('data-standalone-prompt-library="true"')).toBe(true);
    expect(html.includes('提示词中心')).toBe(true);
    expect(html.includes('共 0 条提示词，按标题、标签与分类快速查找灵感。')).toBe(true);
    expect(html.includes('data-prompt-page-state="loading"')).toBe(true);
    expect(html.includes('data-prompt-page-toolbar')).toBe(false);
    expect(html.includes('还没有可用的提示词')).toBe(false);
  });

  test('renders a simple centered empty state without the old descriptive card', () => {
    const html = render();
    expect(html.includes('data-prompt-page-state="empty"')).toBe(true);
    expect(html.includes('暂无提示词')).toBe(true);
    expect(html.includes('可用的 NomiFun 预设和文本素材会显示在这里')).toBe(false);
    expect(html.includes('data-prompt-page-toolbar')).toBe(false);
  });

  test('reveals the flat search and real facets only when legal items exist', () => {
    const html = render({ items: [ITEM], selectedId: ITEM.id });
    expect(html.includes('共 1 条提示词')).toBe(true);
    expect(html.includes('data-prompt-page-toolbar="flat"')).toBe(true);
    expect(html.includes('data-prompt-library-item="prompt-1"')).toBe(true);
    expect(html.includes('镜头规划')).toBe(true);
    expect(html.includes('视频创作')).toBe(true);
    expect(html.includes('分镜')).toBe(true);
    expect(html.includes('aria-pressed="true"')).toBe(true);
  });

  test('keeps initial errors centered and retryable', () => {
    const html = render({ error: new Error('offline'), onRetry: () => undefined });
    expect(html.includes('data-prompt-page-state="error"')).toBe(true);
    expect(html.includes('提示词加载失败')).toBe(true);
    expect(html.includes('重新加载')).toBe(true);
    expect(html.includes('offline')).toBe(false);
  });

  test('locks the 1440px visual measurements in page-local CSS', () => {
    const css = readFileSync(
      new URL('./StandalonePromptLibraryPage.module.css', import.meta.url),
      'utf8'
    );
    expect(css.includes('width: min(1120px, calc(100% - 48px))')).toBe(true);
    expect(css.includes('padding: 31px 0 64px')).toBe(true);
    expect(css.includes('font-size: 36px')).toBe(true);
    expect(css.includes('text-align: center')).toBe(true);
    expect(css.includes('min-height: 263px')).toBe(true);
    expect(css.includes('background-size: 18px 18px')).toBe(true);

    const toolbarRule = css.slice(css.indexOf('.toolbar {'), css.indexOf('.searchField {'));
    expect(toolbarRule.includes('background:')).toBe(false);
    expect(toolbarRule.includes('border:')).toBe(false);
  });
});
