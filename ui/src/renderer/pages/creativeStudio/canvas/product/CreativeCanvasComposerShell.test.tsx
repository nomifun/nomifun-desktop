/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { describe, expect, test } from 'bun:test';
import { readFileSync } from 'node:fs';
import { renderToStaticMarkup } from 'react-dom/server';

import CreativeCanvasComposerShell from './CreativeCanvasComposerShell';

describe('CreativeCanvasComposerShell', () => {
  test('preserves every legacy composer boundary behind one shared shell', () => {
    for (const kind of ['image', 'video', 'audio'] as const) {
      const html = renderToStaticMarkup(
        <CreativeCanvasComposerShell kind={kind} nodeId={`node-${kind}`}>
          <span>{kind}</span>
        </CreativeCanvasComposerShell>
      );

      expect(html.includes('data-canvas-composer="true"')).toBe(true);
      expect(html.includes(`data-canvas-composer-kind="${kind}"`)).toBe(true);
      expect(html.includes(`data-canvas-${kind}-composer="true"`)).toBe(true);
      expect(html.includes(`data-canvas-${kind}-composer-anchor="true"`)).toBe(
        true
      );
      expect(html.includes(`data-node-id="node-${kind}"`)).toBe(true);
    }
  });

  test('owns the adaptive placement and compact visual contract once', () => {
    const source = readFileSync(
      new URL('./CreativeCanvasComposerShell.tsx', import.meta.url),
      'utf8'
    );
    const css = readFileSync(
      new URL('./CreativeCanvasComposerShell.module.css', import.meta.url),
      'utf8'
    );

    expect(source.includes("closest<HTMLElement>('[data-canvas-surface]')")).toBe(
      true
    );
    expect(source.includes("closest<HTMLElement>('[data-canvas-node-id]')")).toBe(
      true
    );
    expect(source.includes('new ResizeObserver(updatePlacement)')).toBe(true);
    expect(source.includes('observer?.disconnect()')).toBe(true);
    expect(source.includes('createPortal(content, document.body)')).toBe(true);
    expect(css.includes('width: 580px')).toBe(true);
    expect(css.includes('height: 104px')).toBe(true);
    expect(css.includes('font-size: 12px')).toBe(true);
    expect(css.includes('line-height: 18px')).toBe(true);
    expect(css.includes('height: 30px')).toBe(true);
    expect(css.includes('flex: 0 1 156px')).toBe(true);
    expect(css.includes('min-width: 48px')).toBe(true);
    expect(css.includes(".positioner[data-placement='above']")).toBe(true);
    expect(css.includes(".positioner[data-overlay='true']")).toBe(true);
    expect(css.includes('@media (prefers-reduced-motion: reduce)')).toBe(true);
  });
});
