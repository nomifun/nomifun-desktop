/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { describe, expect, test } from 'bun:test';
import { createInstance } from 'i18next';
import { renderToStaticMarkup } from 'react-dom/server';
import { I18nextProvider, initReactI18next } from 'react-i18next';

import CreativeStudioCanvasesPage, {
  type CreativeStudioCanvasesPageProps,
} from './CreativeStudioCanvasesPage';
import { CREATIVE_STUDIO_CANVAS_FIXTURES } from './testing';

const testI18n = createInstance();
await testI18n.use(initReactI18next).init({
  lng: 'en-US',
  fallbackLng: 'en-US',
  resources: { 'en-US': { translation: {} } },
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
    expect(html.includes('Infinite canvas')).toBe(true);
    expect(html.includes('New canvas')).toBe(true);
    expect(html.includes('Import canvas')).toBe(true);
    expect(html.includes('品牌短片概念')).toBe(true);
    expect(html.includes('18 nodes · 12 connections')).toBe(true);
  });

  test('renders loading, error, empty, and selected Canvas states', () => {
    const loading = renderPage({
      initialSnapshot: { status: 'loading', canvases: [] },
    });
    expect(loading.includes('data-canvases-loading="true"')).toBe(true);
    expect(loading.includes('Loading canvases...')).toBe(true);

    const error = renderPage({
      initialSnapshot: {
        status: 'error',
        canvases: [],
        error: 'database unavailable',
      },
    });
    expect(error.includes('data-canvases-error="true"')).toBe(true);
    expect(error.includes('Could not load canvases')).toBe(true);

    const empty = renderPage({
      initialSnapshot: { status: 'ready', canvases: [] },
    });
    expect(empty.includes('data-canvases-empty="library"')).toBe(true);
    expect(empty.includes('No canvases yet')).toBe(true);

    const selected = renderPage({
      initialSnapshot: {
        status: 'ready',
        canvases: CREATIVE_STUDIO_CANVAS_FIXTURES,
      },
      initialSelectedIds: ['canvas-product-stills'],
    });
    expect(selected.includes('data-canvas-selected="true"')).toBe(true);
    expect(selected.includes('Export selected')).toBe(true);
    expect(selected.includes('Delete selected')).toBe(true);
  });
});
