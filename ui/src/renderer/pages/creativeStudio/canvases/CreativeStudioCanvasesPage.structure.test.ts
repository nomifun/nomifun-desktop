/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { describe, expect, test } from 'bun:test';
import { readFileSync } from 'node:fs';

const page = readFileSync(
  new URL('./CreativeStudioCanvasesPage.tsx', import.meta.url),
  'utf8'
);
const card = readFileSync(
  new URL('./CreativeStudioCanvasCard.tsx', import.meta.url),
  'utf8'
);
const copy = readFileSync(new URL('./copy.ts', import.meta.url), 'utf8');
const service = readFileSync(
  new URL('./canvasServiceAdapter.ts', import.meta.url),
  'utf8'
);
const css = readFileSync(
  new URL('./CreativeStudioCanvasesPage.module.css', import.meta.url),
  'utf8'
);

describe('Creative Studio Canvas library product contract', () => {
  test('uses Canvas names throughout the product-facing implementation', () => {
    for (const source of [page, card, copy, service]) {
      expect(/\bprojects?\b/i.test(source)).toBe(false);
    }
    expect(page.includes('data-creative-studio-canvases')).toBe(true);
    expect(page.includes('service.createCanvas')).toBe(true);
    expect(page.includes('service.renameCanvas')).toBe(true);
    expect(page.includes('service.deleteCanvases')).toBe(true);
    expect(page.includes('service.exportCanvases')).toBe(true);
    expect(card.includes('data-canvas-id')).toBe(true);
  });

  test('keeps the established responsive card-grid measurements', () => {
    expect(css.includes('max-width: 1152px')).toBe(true);
    expect(
      css.includes('grid-template-columns: repeat(3, minmax(0, 1fr))')
    ).toBe(true);
    expect(css.includes('@media (max-width: 1279px)')).toBe(true);
    expect(
      css.includes('grid-template-columns: repeat(2, minmax(0, 1fr))')
    ).toBe(true);
    expect(css.includes('@media (max-width: 639px)')).toBe(true);
    expect(css.includes('grid-template-columns: minmax(0, 1fr)')).toBe(true);
  });

  test('uses shared theme tokens instead of fixed light-only controls', () => {
    expect(css.includes('background: #171717')).toBe(false);
    expect(css.includes('background: #141414')).toBe(false);
    expect(css.includes('background: #d8d8d8')).toBe(false);
    expect(css.includes('rgb(0 0 0 / 88%)')).toBe(false);
    expect(css.includes('var(--color-bg-1')).toBe(true);
    expect(css.includes('var(--color-bg-2')).toBe(true);
    expect(css.includes('var(--color-text-1')).toBe(true);
    expect(css.includes(":global([data-theme='dark']) .card")).toBe(true);
  });
});
