/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { describe, expect, test } from 'bun:test';
import React from 'react';
import { renderToStaticMarkup } from 'react-dom/server';
import { parseCompanionId } from '@/common/types/ids';
import MigrationSection from './MigrationSection';

const COMPANION_ID = parseCompanionId('0198f6b1-0ef0-7000-8000-000000000001');

/**
 * `isTauriRuntime()` reads `window` at render time, and the desktop branch is the
 * only one carrying the export-scope controls — a fake global window is what lets
 * this suite render it at all (there is no DOM here otherwise). Restored right
 * after so no other suite inherits a half-built window.
 */
const renderDesktop = (): string => {
  const global = globalThis as { window?: unknown };
  const had = 'window' in global;
  const previous = global.window;
  global.window = { isTauri: true };
  try {
    return renderToStaticMarkup(
      React.createElement(MigrationSection, { companionId: COMPANION_ID, companionName: '小南' })
    );
  } finally {
    if (had) global.window = previous;
    else delete global.window;
  }
};

const checkboxes = (markup: string): string[] => markup.match(/<input[^>]*type="checkbox"[^>]*>/g) ?? [];

describe('migration export scope', () => {
  test('memories and skills are choosable; 设定 is checked and disabled', () => {
    const boxes = checkboxes(renderDesktop());
    expect(boxes.length).toBe(3);
    // 设定: always in the bundle, so it is checked and cannot be unticked.
    expect(boxes[0]?.includes('disabled')).toBe(true);
    expect(boxes[0]?.includes('checked')).toBe(true);
    // 记忆 travels by default, 技能 is opt-in — both are the user's to choose.
    expect(boxes[1]?.includes('disabled')).toBe(false);
    expect(boxes[1]?.includes('checked')).toBe(true);
    expect(boxes[2]?.includes('disabled')).toBe(false);
    expect(boxes[2]?.includes('checked')).toBe(false);
  });

  test('the bundle description no longer claims memories and skills are excluded', () => {
    const markup = renderDesktop();
    expect(markup.includes('记忆和技能由你决定')).toBe(true);
    expect(markup.includes('记忆、技能、聊天记录与自定义形象图片都不在其中')).toBe(false);
  });
});
