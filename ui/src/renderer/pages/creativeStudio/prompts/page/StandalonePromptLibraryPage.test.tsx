/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { describe, expect, test } from 'bun:test';
import { createInstance } from 'i18next';
import { readFileSync } from 'node:fs';
import React from 'react';
import { renderToStaticMarkup } from 'react-dom/server';
import { I18nextProvider, initReactI18next } from 'react-i18next';

import type { PromptLibraryItem } from '../types';
import { StandalonePromptLibraryAppearance } from './StandalonePromptLibraryPage';

const testI18n = createInstance();
await testI18n.use(initReactI18next).init({
  lng: 'en-US',
  fallbackLng: 'en-US',
  resources: { 'en-US': { translation: {} } },
  interpolation: { escapeValue: false },
});

const ITEM: PromptLibraryItem = {
  id: 'prompt-1',
  source: 'preset',
  title: '镜头规划',
  description: '整理画面节奏',
  prompt: '按故事目标规划镜头。',
  category: '视频创作',
  tags: ['分镜', '节奏'],
  knowledgeBaseIds: [],
  coverUrl: null,
  preview: null,
  sourceUrl: null,
  license: null,
  licenseUrl: null,
  createdAt: null,
  updatedAt: null,
  savedToAssets: false,
};

const render = (
  props: Partial<React.ComponentProps<typeof StandalonePromptLibraryAppearance>> = {}
) =>
  renderToStaticMarkup(
    <I18nextProvider i18n={testI18n}>
      <StandalonePromptLibraryAppearance items={[]} {...props} />
    </I18nextProvider>
  );

describe('standalone prompt-library source-parity appearance', () => {
  test('centers the real count header and keeps loading state minimal', () => {
    const html = render({ loading: true, onRetry: () => undefined });
    expect(html.includes('data-standalone-prompt-library="true"')).toBe(true);
    expect(html.includes('Prompt center')).toBe(true);
    expect(
      html.includes(
        '0 prompts. Search by title, tags, and category for inspiration.'
      )
    ).toBe(true);
    expect(html.includes('data-prompt-page-state="loading"')).toBe(true);
    expect(html.includes('data-prompt-page-toolbar')).toBe(false);
    expect(html.includes('No prompts available yet')).toBe(false);
  });

  test('renders the loaded-empty toolbar and source-aligned inbox state without fake facets', () => {
    const html = render();
    expect(html.includes('data-prompt-page-state="empty"')).toBe(true);
    expect(html.includes('No matching prompts found')).toBe(true);
    expect(html.includes('NomiFun presets and text assets will appear here')).toBe(false);
    expect(html.includes('data-prompt-page-toolbar="flat"')).toBe(true);
    expect(html.split('>All<').length - 1).toBe(2);
    expect(html.includes('视频创作')).toBe(false);
  });

  test('reveals the flat search and real facets only when legal items exist', () => {
    const html = render({ items: [ITEM], selectedId: ITEM.id });
    expect(
      html.includes(
        '1 prompts. Search by title, tags, and category for inspiration.'
      )
    ).toBe(true);
    expect(html.includes('data-prompt-page-toolbar="flat"')).toBe(true);
    expect(html.includes('data-prompt-library-item="prompt-1"')).toBe(true);
    expect(html.includes('镜头规划')).toBe(true);
    expect(html.includes('视频创作')).toBe(true);
    expect(html.includes('分镜')).toBe(true);
    expect(html.includes('aria-pressed="true"')).toBe(true);
    expect(html.includes('Search by title, then press Enter')).toBe(true);
    expect(html.includes('You have reached the end')).toBe(true);
    expect(html.split('data-facet-chips-expanded="false"').length - 1).toBe(2);
  });

  test('collapses facet chips to two rows and reveals a measured overflow toggle', () => {
    const source = readFileSync(
      new URL('./StandalonePromptLibraryPage.tsx', import.meta.url),
      'utf8'
    );
    const css = readFileSync(
      new URL('./StandalonePromptLibraryPage.module.css', import.meta.url),
      'utf8'
    );

    expect(source.includes("creativeStudio.prompts.collapse")).toBe(true);
    expect(source.includes("creativeStudio.prompts.showMore")).toBe(true);
    expect(source.includes('chips.scrollHeight > collapsedHeight + 1')).toBe(true);
    expect(source.includes('new ResizeObserver(measureOverflow)')).toBe(true);
    expect(source.includes('aria-expanded={expanded}')).toBe(true);
    expect(css.includes('--facet-chips-collapsed-height: 54px')).toBe(true);
    expect(/\.chipsCollapsed\s*\{[\s\S]*?max-height:[\s\S]*?overflow:\s*hidden;/.test(css)).toBe(true);
  });

  test('renders a bounded first page instead of mounting the full catalog at once', () => {
    const items = Array.from({ length: 31 }, (_, index) => ({
      ...ITEM,
      id: `prompt-${index + 1}`,
      title: `提示词 ${index + 1}`,
    }));
    const html = render({ items });
    expect(html.split('data-prompt-library-item=').length - 1).toBe(30);
    expect(html.includes('Continue scrolling to load more')).toBe(true);
  });

  test('keeps initial errors centered and retryable', () => {
    const html = render({ error: new Error('offline'), onRetry: () => undefined });
    expect(html.includes('data-prompt-page-state="error"')).toBe(true);
    expect(html.includes('Prompt loading failed')).toBe(true);
    expect(html.includes('Reload')).toBe(true);
    expect(html.includes('offline')).toBe(false);
  });

  test('locks the 1440px visual measurements in page-local CSS', () => {
    const css = readFileSync(
      new URL('./StandalonePromptLibraryPage.module.css', import.meta.url),
      'utf8'
    );
    expect(css.includes('width: min(1280px, calc(100% - 48px))')).toBe(true);
    expect(css.includes('padding: 31px 0 64px')).toBe(true);
    expect(css.includes('font-size: 36px')).toBe(true);
    expect(css.includes('text-align: center')).toBe(true);
    expect(css.includes('min-height: 263px')).toBe(true);
    expect(css.includes('background-size: 18px 18px')).toBe(true);
    expect(css.includes('width: min(672px, 100%)')).toBe(true);
    expect(css.includes('max-width: 1152px')).toBe(true);
    expect(css.includes('overflow-wrap: anywhere')).toBe(true);
    expect(css.includes('padding-top: 126px')).toBe(true);
    expect(css.includes('aspect-ratio: 4 / 3')).toBe(true);
    expect(css.includes('gap: 20px')).toBe(true);

    const toolbarRule = css.slice(css.indexOf('.toolbar {'), css.indexOf('.searchField {'));
    expect(toolbarRule.includes('background:')).toBe(false);
    expect(toolbarRule.includes('border:')).toBe(false);
  });
});
