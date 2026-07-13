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
  test('uses a matte black and white selected tab/card/checkbox treatment without glow', () => {
    for (const className of ['drawerSegmentActive', 'drawerCardSelected', 'drawerCardStatusSelected']) {
      const block = classBlock(className);
      expect(block.includes('var(--skill-selection-bg)')).toBe(true);
      expect(block.includes('var(--skill-selection-fg)')).toBe(true);
      expect(block.includes('box-shadow: none')).toBe(true);
    }
  });

  test('keeps the selected skills apply button aligned with the same black and white treatment', () => {
    const start = css.lastIndexOf('.drawerPrimaryButton {');
    const block = css.slice(start, css.indexOf('\n}', start));
    expect(block.includes('var(--skill-selection-bg)')).toBe(true);
    expect(block.includes('var(--skill-selection-fg)')).toBe(true);
  });

  test('does not add a redundant left selection rail to selected Skill cards', () => {
    expect(css.includes('.drawerCardSelected::before')).toBe(false);
  });
});
