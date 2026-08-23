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

import CreativeStudioCanvasesPage, {
  type CreativeStudioCanvasesPageProps,
} from './CreativeStudioCanvasesPage';
import { CREATIVE_STUDIO_CANVAS_FIXTURES } from './testing';

const testI18n = createInstance();
await testI18n.use(initReactI18next).init({
  lng: 'zh-CN',
  fallbackLng: 'zh-CN',
  resources: { 'zh-CN': { translation: {} } },
  interpolation: { escapeValue: false },
});

const renderPage = (props: CreativeStudioCanvasesPageProps = {}) =>
  renderToStaticMarkup(
    <I18nextProvider i18n={testI18n}>
      <CreativeStudioCanvasesPage autoLoad={false} {...props} />
    </I18nextProvider>
  );

describe('Creative Studio Canvas library presentation', () => {
  test('renders canonical Canvas identity and node/connection counts', () => {
    const html = renderPage({
      initialSnapshot: {
        status: 'ready',
        canvases: CREATIVE_STUDIO_CANVAS_FIXTURES,
      },
    });

    expect(html.includes('data-creative-studio-canvases="true"')).toBe(true);
    expect(html.includes('data-canvases-state="ready"')).toBe(true);
    expect(html.includes('data-canvases-grid="true"')).toBe(true);
    expect(html.includes('data-canvas-id="canvas-brand-film"')).toBe(true);
    expect(html.includes('无限画布')).toBe(true);
    expect(html.includes('新建画布')).toBe(true);
    expect(html.includes('导入画布')).toBe(true);
    expect(html.includes('品牌短片概念')).toBe(true);
    expect(html.includes('18 个节点 · 12 条连线')).toBe(true);
  });

  test('renders loading, error, empty, and selected Canvas states', () => {
    const loading = renderPage({
      initialSnapshot: { status: 'loading', canvases: [] },
    });
    expect(loading.includes('data-canvases-loading="true"')).toBe(true);
    expect(loading.includes('正在加载画布')).toBe(true);

    const error = renderPage({
      initialSnapshot: {
        status: 'error',
        canvases: [],
        error: 'database unavailable',
      },
    });
    expect(error.includes('data-canvases-error="true"')).toBe(true);
    expect(error.includes('加载画布失败')).toBe(true);

    const empty = renderPage({
      initialSnapshot: { status: 'ready', canvases: [] },
    });
    expect(empty.includes('data-canvases-empty="library"')).toBe(true);
    expect(empty.includes('还没有画布')).toBe(true);

    const selected = renderPage({
      initialSnapshot: {
        status: 'ready',
        canvases: CREATIVE_STUDIO_CANVAS_FIXTURES,
      },
      initialSelectedIds: ['canvas-product-stills'],
    });
    expect(selected.includes('data-canvas-selected="true"')).toBe(true);
    expect(selected.includes('导出选中')).toBe(true);
    expect(selected.includes('删除选中')).toBe(true);
  });
});
