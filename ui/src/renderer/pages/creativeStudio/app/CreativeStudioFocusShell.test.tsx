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
import { MemoryRouter, Route, Routes } from 'react-router-dom';

import zhCreativeStudio from '@renderer/services/i18n/locales/zh-CN/creativeStudio.json';
import CreativeStudioFocusShell from './CreativeStudioFocusShell';
import CreativeStudioHomePage from './CreativeStudioHomePage';
import { CREATIVE_STUDIO_ROOT_PATH } from './routes';

const testI18n = createInstance();
await testI18n.use(initReactI18next).init({
  lng: 'zh-CN',
  fallbackLng: 'zh-CN',
  resources: { 'zh-CN': { translation: { creativeStudio: zhCreativeStudio } } },
  interpolation: { escapeValue: false },
});

const renderFocusShell = () =>
  renderToStaticMarkup(
    <I18nextProvider i18n={testI18n}>
      <MemoryRouter initialEntries={[CREATIVE_STUDIO_ROOT_PATH]}>
        <Routes>
          <Route path={CREATIVE_STUDIO_ROOT_PATH} element={<CreativeStudioFocusShell />}>
            <Route index element={<CreativeStudioHomePage />} />
          </Route>
        </Routes>
      </MemoryRouter>
    </I18nextProvider>
  );

describe('Creative Studio focus shell', () => {
  test('renders product chrome, routed content, and the isolated portal root', () => {
    const html = renderFocusShell();

    expect(html.includes('data-creative-studio-focus-shell="true"')).toBe(true);
    expect(html.includes('data-creative-studio-top-bar="true"')).toBe(true);
    expect(html.includes('data-creative-studio-home="true"')).toBe(true);
    expect(html.includes('id="creative-studio-portal-root"')).toBe(true);
    expect(html.includes('返回工作台')).toBe(true);
    expect(html.includes('把灵感铺展成一张无限画布')).toBe(true);
  });

  test('does not render ordinary workbench chrome inside the product boundary', () => {
    const html = renderFocusShell();

    expect(html.includes('layout-sider')).toBe(false);
    expect(html.includes('PwaPullToRefresh')).toBe(false);
    expect(html.includes('app-titlebar__menu')).toBe(false);
  });
});
