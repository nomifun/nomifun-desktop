/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { describe, expect, test } from 'bun:test';
import { createInstance } from 'i18next';
import React from 'react';
import { renderToStaticMarkup } from 'react-dom/server';
import { I18nextProvider, initReactI18next } from 'react-i18next';

import { PromptLibrarySurface } from './PromptLibrarySurface';
import type { PromptLibraryItem } from './types';

const testI18n = createInstance();
await testI18n.use(initReactI18next).init({
  lng: 'en-US',
  fallbackLng: 'en-US',
  resources: { 'en-US': { translation: {} } },
  interpolation: { escapeValue: false },
});

const ITEM: PromptLibraryItem = {
  id: 'prompt-one',
  source: 'preset',
  title: '场景拆解',
  description: '将创作目标拆成可执行画面',
  prompt: '提取主体、环境、光线和镜头关系。',
  category: '视觉创作',
  tags: ['构图', '场景'],
  knowledgeBaseIds: ['knowledge-one'],
  coverUrl: null,
  preview: null,
  sourceUrl: null,
  license: null,
  licenseUrl: null,
  createdAt: null,
  updatedAt: null,
};

const render = (props: Partial<React.ComponentProps<typeof PromptLibrarySurface>> = {}) =>
  renderToStaticMarkup(
    <I18nextProvider i18n={testI18n}>
      <PromptLibrarySurface variant='page' items={[ITEM]} {...props} />
    </I18nextProvider>
  );

describe('PromptLibrarySurface', () => {
  test('renders a validated page card, facets and copy affordance', () => {
    const html = render({ onCopy: () => undefined });
    expect(html.includes('data-prompt-library="page"')).toBe(true);
    expect(html.includes('data-prompt-library-item="prompt-one"')).toBe(true);
    expect(html.includes('场景拆解')).toBe(true);
    expect(html.includes('视觉创作')).toBe(true);
    expect(html.includes('构图')).toBe(true);
    expect(html.includes('Copy prompt: 场景拆解')).toBe(true);
    expect(html.includes('data-prompt-library-action="copy"')).toBe(true);
    expect(html.includes('>Copy</button>')).toBe(true);
    expect(html.includes('Insert prompt')).toBe(false);
    expect(html.includes('1 linked knowledge bases')).toBe(true);
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
    expect(error.includes('Reload')).toBe(true);
  });

  test('reports rejected records without rendering their content', () => {
    const html = render({ invalidCount: 2 });
    expect(
      html.includes(
        '2 records were ignored because they did not match the data contract.'
      )
    ).toBe(true);
    expect(html.includes('external prompt marketplace')).toBe(false);
  });
});
