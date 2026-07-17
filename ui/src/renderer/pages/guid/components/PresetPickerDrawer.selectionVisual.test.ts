/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { describe, expect, test } from 'bun:test';
import { readFileSync } from 'node:fs';

const css = readFileSync(new URL('../index.module.css', import.meta.url), 'utf8');

const classBlock = (className: string) => {
  const start = css.indexOf(`.${className} {`);
  expect(start).toBeGreaterThan(-1);
  const end = css.indexOf('\n}', start);
  return css.slice(start, end);
};

describe('PresetPickerDrawer selection visual language', () => {
  test('uses a restrained theme-aware selection treatment instead of inverted black cards', () => {
    const surface = classBlock('drawerSurface');
    const tab = classBlock('drawerSegmentActive');
    const card = classBlock('drawerCardSelected');
    const status = classBlock('drawerCardStatusSelected');

    expect(surface.includes('--drawer-selection-bg')).toBe(true);
    expect(surface.includes('--drawer-selection-border')).toBe(true);
    expect(surface.includes('--skill-selection-bg')).toBe(false);
    expect(tab.includes('var(--drawer-selection-fg)')).toBe(true);
    expect(card.includes('var(--drawer-selection-bg)')).toBe(true);
    expect(card.includes('var(--drawer-selection-border)')).toBe(true);
    expect(status.includes('var(--drawer-selection-fg)')).toBe(true);
    expect(card.includes('#151515')).toBe(false);
  });

  test('reserves the solid accent for the compact apply action', () => {
    const start = css.lastIndexOf('.drawerPrimaryButton {');
    const block = css.slice(start, css.indexOf('\n}', start));
    expect(block.includes('rgb(var(--primary-6))')).toBe(true);
    expect(block.includes('var(--drawer-selection-bg)')).toBe(false);
  });

  test('does not add a redundant left selection rail to selected Skill cards', () => {
    expect(css.includes('.drawerCardSelected::before')).toBe(false);
  });
});
