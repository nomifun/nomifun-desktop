/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { describe, expect, test } from 'bun:test';
import { readFileSync } from 'node:fs';
import { createInstance } from 'i18next';
import React from 'react';
import { renderToStaticMarkup } from 'react-dom/server';
import { I18nextProvider, initReactI18next } from 'react-i18next';
import { MemoryRouter, Route, Routes } from 'react-router-dom';

import { ThemeProvider } from '@renderer/hooks/context/ThemeContext';
import zhCreativeStudio from '@renderer/services/i18n/locales/zh-CN/creativeStudio.json';
import CreativeStudioFocusShell from './CreativeStudioFocusShell';
import CreativeStudioHomePage from './CreativeStudioHomePage';
import { CREATIVE_STUDIO_ROOT_PATH, CREATIVE_STUDIO_VIDEO_PATH } from './routes';

const testI18n = createInstance();
await testI18n.use(initReactI18next).init({
  lng: 'zh-CN',
  fallbackLng: 'zh-CN',
  resources: { 'zh-CN': { translation: { creativeStudio: zhCreativeStudio } } },
  interpolation: { escapeValue: false },
});

const renderFocusShell = (path = CREATIVE_STUDIO_ROOT_PATH) =>
  renderToStaticMarkup(
    <I18nextProvider i18n={testI18n}>
      <ThemeProvider>
        <MemoryRouter initialEntries={[path]}>
          <Routes>
            <Route path={CREATIVE_STUDIO_ROOT_PATH} element={<CreativeStudioFocusShell />}>
              <Route index element={<CreativeStudioHomePage />} />
              <Route path='video' element={<div data-test-route='video'>video route</div>} />
            </Route>
          </Routes>
        </MemoryRouter>
      </ThemeProvider>
    </I18nextProvider>
  );

describe('Creative Studio focus shell', () => {
  test('coordinates shell navigation with the active canvas CAS gate', () => {
    const source = readFileSync(new URL('./CreativeStudioFocusShell.tsx', import.meta.url), 'utf8');

    expect(source.includes('requestCreativeCanvasProductBeforeLeave')).toBe(true);
    expect(source.includes('navigateAfterCanvasFlush')).toBe(true);
    expect(source.includes('onNavigate={navigateWithinStudio}')).toBe(true);
  });

  test('renders source product navigation and the isolated route boundary', () => {
    const html = renderFocusShell();

    expect(html.includes('data-creative-studio-focus-shell="true"')).toBe(true);
    expect(html.includes('data-creative-studio-top-bar="true"')).toBe(true);
    expect(html.includes('data-creative-studio-section="projects"')).toBe(true);
    expect(html.includes('id="creative-studio-portal-root"')).toBe(true);
    expect(html.includes('返回工作台')).toBe(true);
    expect(html.includes('aria-label="深色"')).toBe(true);
    expect(html.includes('我的画布')).toBe(true);
    expect(html.includes('生图工作台')).toBe(true);
    expect(html.includes('视频创作台')).toBe(true);
    expect(html.includes('提示词库')).toBe(true);
    expect(html.includes('我的素材')).toBe(true);
    expect(html.includes('data-creative-studio-navigation="audio"')).toBe(false);
  });

  test('keeps the index empty until the project list is wired', () => {
    const html = renderFocusShell();

    expect(html.includes('data-creative-studio-home')).toBe(false);
    expect(html.includes('把灵感铺展成一张无限画布')).toBe(false);
    expect(html.includes('creativeStudio.home')).toBe(false);
  });

  test('marks a deep-linked workbench destination active', () => {
    const html = renderFocusShell(CREATIVE_STUDIO_VIDEO_PATH);

    expect(html.includes('data-creative-studio-section="video"')).toBe(true);
    expect(html.includes('data-test-route="video"')).toBe(true);
    expect(
      /data-active="true" data-creative-studio-navigation="video" aria-current="page"/.test(html)
    ).toBe(true);
  });

  test('does not render ordinary workbench chrome inside the product boundary', () => {
    const html = renderFocusShell();

    expect(html.includes('layout-sider')).toBe(false);
    expect(html.includes('PwaPullToRefresh')).toBe(false);
    expect(html.includes('app-titlebar__menu')).toBe(false);
  });
});
