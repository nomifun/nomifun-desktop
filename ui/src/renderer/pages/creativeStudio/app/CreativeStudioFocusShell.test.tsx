/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { describe, expect, test } from 'bun:test';
import { readFileSync } from 'node:fs';
import React from 'react';
import { renderToStaticMarkup } from 'react-dom/server';
import { MemoryRouter, Route, Routes } from 'react-router-dom';

import CreativeStudioFocusShell from './CreativeStudioFocusShell';
import {
  CREATIVE_STUDIO_CANVASES_PATH,
  CREATIVE_STUDIO_ROOT_PATH,
  CREATIVE_STUDIO_VIDEO_PATH,
} from './routes';

const renderFocusShell = (path = CREATIVE_STUDIO_CANVASES_PATH) =>
  renderToStaticMarkup(
    <MemoryRouter initialEntries={[path]}>
      <Routes>
        <Route path={CREATIVE_STUDIO_ROOT_PATH} element={<CreativeStudioFocusShell />}>
          <Route path='canvases' element={<div data-test-route='canvases'>canvases route</div>} />
          <Route path='video' element={<div data-test-route='video'>video route</div>} />
        </Route>
      </Routes>
    </MemoryRouter>
  );

describe('Creative Studio route shell', () => {
  test('keeps product overlays on the active application theme', () => {
    const css = readFileSync(
      new URL('./CreativeStudioFocusShell.module.css', import.meta.url),
      'utf8'
    );

    expect(css.includes('.portalRoot')).toBe(true);
    expect(css.includes('--color-bg-1: #f4f2ed')).toBe(false);
    expect(css.includes('--color-bg-popup: #fbfaf7')).toBe(false);
    expect(css.includes('--primary-6: 87, 83, 78')).toBe(false);
    expect(css.includes(":global([data-theme='light']) .shell")).toBe(true);
    expect(css.includes(":global([data-theme='dark']) .shell")).toBe(true);
    expect(css.includes('color-scheme: inherit')).toBe(true);
  });

  test('renders the product outlet without a duplicate titlebar', () => {
    const html = renderFocusShell();
    const source = readFileSync(new URL('./CreativeStudioFocusShell.tsx', import.meta.url), 'utf8');

    expect(html.includes('data-creative-studio-focus-shell="true"')).toBe(true);
    expect(html.includes('data-creative-studio-section="canvases"')).toBe(true);
    expect(html.includes('id="creative-studio-portal-root"')).toBe(true);
    expect(html.includes('data-test-route="canvases"')).toBe(true);
    expect(html.includes('data-creative-studio-top-bar')).toBe(false);
    expect(source.includes('CreativeStudioTopBar')).toBe(false);
    expect(source.includes('WindowControls')).toBe(false);
    expect(source.includes('useThemeContext')).toBe(false);
  });

  test('preserves deep-linked section ownership', () => {
    const html = renderFocusShell(CREATIVE_STUDIO_VIDEO_PATH);

    expect(html.includes('data-creative-studio-section="video"')).toBe(true);
    expect(html.includes('data-test-route="video"')).toBe(true);
  });
});
