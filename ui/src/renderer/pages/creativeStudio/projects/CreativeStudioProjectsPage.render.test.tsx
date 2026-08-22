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

import CreativeStudioProjectsPage, { type CreativeStudioProjectsPageProps } from './CreativeStudioProjectsPage';
import { CREATIVE_STUDIO_PROJECT_FIXTURES } from './testing';

const testI18n = createInstance();
await testI18n.use(initReactI18next).init({
  lng: 'zh-CN',
  fallbackLng: 'zh-CN',
  resources: { 'zh-CN': { translation: {} } },
  interpolation: { escapeValue: false },
});

const renderPage = (props: CreativeStudioProjectsPageProps = {}) =>
  renderToStaticMarkup(
    <I18nextProvider i18n={testI18n}>
      <CreativeStudioProjectsPage autoLoad={false} {...props} />
    </I18nextProvider>
  );

describe('Creative Studio project center presentation', () => {
  test('renders the source project library and canonical node/connection counts', () => {
    const html = renderPage({
      initialSnapshot: { status: 'ready', projects: CREATIVE_STUDIO_PROJECT_FIXTURES },
    });

    expect(html.includes('data-creative-studio-projects="true"')).toBe(true);
    expect(html.includes('data-projects-state="ready"')).toBe(true);
    expect(html.includes('data-projects-grid="true"')).toBe(true);
    expect(html.includes('画布库')).toBe(true);
    expect(html.includes('无限画布')).toBe(true);
    expect(html.includes('新建画布')).toBe(true);
    expect(html.includes('导入画布')).toBe(true);
    expect(html.includes('品牌短片概念')).toBe(true);
    expect(html.includes('秋季产品静物')).toBe(true);
    expect(html.includes('角色风格探索')).toBe(true);
    expect(html.includes('18 个节点 · 12 条连线')).toBe(true);
  });

  test('keeps checkbox selection on cards and moves only source batch actions into the header', () => {
    const html = renderPage({
      initialSnapshot: { status: 'ready', projects: CREATIVE_STUDIO_PROJECT_FIXTURES },
      initialSelectedIds: ['project-product-stills'],
    });

    expect(html.includes('data-selection-active="true"')).toBe(true);
    expect(html.includes('data-project-selected="true"')).toBe(true);
    expect(html.includes('导出选中')).toBe(true);
    expect(html.includes('删除选中')).toBe(true);
    expect(html.includes('删除全部')).toBe(true);
    expect(html.includes('品牌短片概念')).toBe(true);
    expect(html.includes('秋季产品静物')).toBe(true);
    expect(html.includes('角色风格探索')).toBe(true);
    expect(html.includes('已选 1 个项目')).toBe(false);
    expect(html.includes('取消选择')).toBe(false);
  });

  test('renders dedicated loading, error, and source-shaped empty-library states', () => {
    const loading = renderPage({ initialSnapshot: { status: 'loading', projects: [] } });
    expect(loading.includes('data-projects-loading="true"')).toBe(true);
    expect(loading.includes('正在加载画布')).toBe(true);

    const error = renderPage({
      initialSnapshot: { status: 'error', projects: [], error: 'database unavailable' },
    });
    expect(error.includes('data-projects-error="true"')).toBe(true);
    expect(error.includes('加载画布失败')).toBe(true);
    expect(error.includes('database unavailable')).toBe(true);
    expect(error.includes('重试')).toBe(true);

    const empty = renderPage({ initialSnapshot: { status: 'ready', projects: [] } });
    expect(empty.includes('data-projects-empty="library"')).toBe(true);
    expect(empty.includes('还没有画布')).toBe(true);
    expect(empty.includes('新建一个画布后，就可以独立保存节点、连线和画布外观。')).toBe(true);
    expect(empty.match(/新建画布/g)?.length).toBe(2);
  });

  test('does not reintroduce non-source search, sort, selection summary, or subtitle controls', () => {
    const html = renderPage({
      initialSnapshot: { status: 'ready', projects: CREATIVE_STUDIO_PROJECT_FIXTURES },
    });

    expect(html.includes('搜索画布')).toBe(false);
    expect(html.includes('排序方式')).toBe(false);
    expect(html.includes('全选当前结果')).toBe(false);
    expect(html.includes('共 3 个项目')).toBe(false);
    expect(html.includes('每个项目独立保存节点')).toBe(false);
  });
});
