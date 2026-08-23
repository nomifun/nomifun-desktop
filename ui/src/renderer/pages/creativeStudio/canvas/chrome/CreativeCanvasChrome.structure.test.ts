/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { describe, expect, test } from 'bun:test';
import { readFileSync } from 'node:fs';

const component = readFileSync(new URL('./CreativeCanvasChrome.tsx', import.meta.url), 'utf8');
const types = readFileSync(new URL('./types.ts', import.meta.url), 'utf8');
const css = readFileSync(new URL('./CreativeCanvasChrome.module.css', import.meta.url), 'utf8');

describe('CreativeCanvasChrome architecture boundaries', () => {
  test('uses canonical product types, IconPark, Arco, and injected slots', () => {
    expect(component.includes("from '@icon-park/react'")).toBe(true);
    expect(component.includes("from '@arco-design/web-react'")).toBe(true);
    expect(types.includes("CreativeCanvasNode['type']")).toBe(true);
    expect(types.includes('CreativeCanvasBackground')).toBe(true);
    expect(types.includes('CanvasInteractionTool')).toBe(true);
    expect(component.includes('props.slots?.canvas')).toBe(true);
    expect(component.includes('props.slots?.left')).toBe(true);
    expect(component.includes('props.slots?.right')).toBe(true);
    expect(component.includes('props.slots?.bottom')).toBe(true);
  });

  test('emits actions without persistence, API, model, or fake-asset logic', () => {
    for (const callback of [
      'onBackToCanvases',
      'onToolChange',
      'onAddNode',
      'onBackgroundChange',
      'onUndo',
      'onRedo',
      'onFitView',
      'onToggleMiniMap',
      'onLeftViewChange',
      'onRightViewChange',
      'onBottomViewChange',
    ]) {
      expect(types.includes(callback)).toBe(true);
    }
    for (const forbidden of [
      'fetch(',
      'localStorage',
      'useCreativeProject',
      'invokeModel',
      'generateImage',
      'fakeAsset',
      '<svg',
      'workshop',
    ]) {
      expect(component.includes(forbidden)).toBe(false);
    }
  });

  test('keeps the canonical background vocabulary without a legacy fourth mode', () => {
    expect(types.includes("'dots'" )).toBe(true);
    expect(types.includes("'lines'" )).toBe(true);
    expect(types.includes("'blank'" )).toBe(true);
    expect(types.includes("'grid'" )).toBe(false);
  });

  test('keeps source-order node creation directly on the toolbar', () => {
    expect(
      types.includes('CREATIVE_CANVAS_CHROME_TOOLBAR_NODE_KINDS')
    ).toBe(true);
    expect(
      component.includes('CREATIVE_CANVAS_CHROME_TOOLBAR_NODE_KINDS.map')
    ).toBe(true);
    expect(
      component.includes('onClick={() => props.onAddNode(kind)}')
    ).toBe(true);
    expect(component.includes('nodeMenuOpen')).toBe(false);
    expect(types.includes('nodeMenuOpen')).toBe(false);
  });

  test('preserves a squeezed three-column canvas and compact scrolling dock', () => {
    for (const token of [
      'grid-template-columns: 280px minmax(0, 1fr) 360px',
      'grid-column: 2',
      'overflow-x: auto',
      "data-compact='true'",
      '@media (max-width: 1180px)',
      '@media (max-width: 880px)',
      '@media (max-width: 640px)',
      '@media (prefers-reduced-motion: reduce)',
    ]) {
      expect(css.includes(token)).toBe(true);
    }
  });
});
