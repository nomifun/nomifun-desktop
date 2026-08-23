/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { describe, expect, test } from 'bun:test';
import { readFileSync } from 'node:fs';
import React from 'react';
import { renderToStaticMarkup } from 'react-dom/server';

import CanvasSurface from './CanvasSurface';
import { withCanvasTestI18n } from './canvasI18nTestUtils';

const renderSurface = (backgroundMode: 'dots' | 'lines' | 'blank' = 'dots') =>
  renderToStaticMarkup(
    withCanvasTestI18n(
      <CanvasSurface
        viewport={{ x: 120, y: -80, zoom: 1.25 }}
        backgroundMode={backgroundMode}
        selectionRect={{ x: 24, y: 30, width: -180, height: 96 }}
        edgeLayer={<span data-test-slot='edges' />}
        nodeLayer={<span data-test-slot='nodes' />}
        worldOverlay={<span data-test-slot='world-overlay' />}
        screenOverlay={<span data-test-slot='screen-overlay' />}
        topDock={<span data-test-slot='top' />}
        leftDock={<span data-test-slot='left' />}
        rightDock={<span data-test-slot='right' />}
        bottomDock={<span data-test-slot='bottom' />}
        miniMap={<span data-test-slot='minimap' />}
        isMiniMapOpen
        zoomControls={{
          onZoomChange: () => undefined,
          onResetView: () => undefined,
          onFitView: () => undefined,
          onToggleMiniMap: () => undefined,
        }}
      />
    )
  );

describe('Creative Studio canvas surface', () => {
  test('establishes controlled screen and world layers without owning canvas state', () => {
    const html = renderSurface();

    expect(html.includes('data-canvas-surface="true"')).toBe(true);
    expect(html.includes('data-canvas-background="dots"')).toBe(true);
    expect(html.includes('data-canvas-world="true"')).toBe(true);
    expect(html.includes('translate3d(120px, -80px, 0) scale(1.25)')).toBe(true);
    expect(html.includes('data-canvas-layer="edges"')).toBe(true);
    expect(html.includes('data-canvas-layer="nodes"')).toBe(true);
    expect(html.includes('data-canvas-layer="selection"')).toBe(true);
    expect(html.includes('left:-156px')).toBe(true);
    expect(html.includes('width:180px')).toBe(true);
  });

  test('renders every product chrome slot and controlled navigation surface', () => {
    const html = renderSurface();

    for (const slot of ['top', 'left', 'right', 'bottom', 'minimap']) {
      expect(html.includes(`data-test-slot="${slot}"`)).toBe(true);
    }
    expect(html.includes('data-canvas-zoom-controls="true"')).toBe(true);
    expect(html.includes('data-canvas-minimap="true"')).toBe(true);
    expect(html.includes('aria-pressed="true"')).toBe(true);
    expect(html.includes('125%')).toBe(true);
  });

  test('switches the visual background contract without replacing the world layer', () => {
    for (const mode of ['lines', 'blank'] as const) {
      const html = renderSurface(mode);
      expect(html.includes(`data-canvas-background="${mode}"`)).toBe(true);
      expect(html.includes('data-canvas-world="true"')).toBe(true);
    }
  });

  test('keeps the full-screen chrome responsive down to the compact focus shell', () => {
    const css = readFileSync(new URL('./CanvasSurface.module.css', import.meta.url), 'utf8');
    const zoomCss = readFileSync(new URL('./CanvasZoomControls.module.css', import.meta.url), 'utf8');

    expect(css.includes('@media (max-width: 1280px)')).toBe(true);
    expect(css.includes('@media (max-width: 1024px)')).toBe(true);
    expect(css.includes('@media (max-width: 640px)')).toBe(true);
    expect(css.includes('position: fixed')).toBe(false);
    expect(css.includes('bottom: 112px')).toBe(true);
    expect(zoomCss.includes('@media (max-width: 640px)')).toBe(true);
    expect(zoomCss.includes('.slider {\n    display: none;')).toBe(true);
  });
});
