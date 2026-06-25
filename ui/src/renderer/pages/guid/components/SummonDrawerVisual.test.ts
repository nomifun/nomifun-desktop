/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { readFileSync } from 'node:fs';
import { describe, expect, test } from 'bun:test';

const readSource = (url: URL) => readFileSync(url, 'utf8');

const classBlock = (css: string, className: string) => {
  const start = css.indexOf(`.${className} {`);
  expect(start).toBeGreaterThan(-1);
  const end = css.indexOf('\n}', start);
  return css.slice(start, end);
};

describe('Guid summon drawer visual system', () => {
  test('uses a scoped command-panel shell instead of old utility-class controls', () => {
    const source = readSource(new URL('./SummonDrawer.tsx', import.meta.url));
    const css = readSource(new URL('../index.module.css', import.meta.url));

    expect(source.includes('styles.drawerSurface')).toBe(true);
    expect(source.includes('styles.drawerTopbar')).toBe(true);
    expect(source.includes('styles.drawerSegmented')).toBe(true);
    expect(source.includes('styles.drawerCloseButton')).toBe(true);
    expect(source.includes('styles.drawerSearchInput')).toBe(true);
    expect(css.includes('.drawerSurface')).toBe(true);
    expect(css.includes('.drawerSegmented')).toBe(true);
    expect(source.includes('&times;')).toBe(false);
    expect(source.includes('bg-color-fill-1 border border-color-border-2')).toBe(false);
  });

  test('renders assistant and skill results with the modern drawer card classes', () => {
    const assistant = readSource(new URL('./DrawerAssistantCard.tsx', import.meta.url));
    const skill = readSource(new URL('./DrawerSkillCard.tsx', import.meta.url));
    const css = readSource(new URL('../index.module.css', import.meta.url));

    for (const source of [assistant, skill]) {
      expect(source.includes('styles.drawerCard')).toBe(true);
      expect(source.includes('styles.drawerCardSelected')).toBe(true);
      expect(source.includes('styles.drawerIconTile')).toBe(true);
      expect(source.includes('styles.drawerTagChip')).toBe(true);
      expect(source.includes('rounded-xl cursor-pointer border transition-all')).toBe(false);
    }

    expect(css.includes('.drawerCard')).toBe(true);
    expect(css.includes('.drawerCardSelected')).toBe(true);
    expect(css.includes('.drawerIconTile')).toBe(true);
    expect(css.includes('.drawerTagChip')).toBe(true);
  });

  test('lets the shared tag filter opt into the drawer skin', () => {
    const drawer = readSource(new URL('./SummonDrawer.tsx', import.meta.url));
    const filter = readSource(new URL('../../settings/AssistantSettings/AssistantTagFilterBar.tsx', import.meta.url));

    expect(drawer.includes("variant='drawer'")).toBe(true);
    expect(drawer.includes('styles.drawerFilterPanel')).toBe(true);
    expect(filter.includes("variant?: 'default' | 'drawer'")).toBe(true);
    expect(filter.includes('filterBarStyles.drawerFilterBar')).toBe(true);
  });

  test('keeps the drawer tag filter compact inside the narrow panel', () => {
    const css = readSource(new URL('../../settings/AssistantSettings/AssistantTagFilterBar.module.css', import.meta.url));
    const bar = classBlock(css, 'drawerFilterBar');
    const rows = classBlock(css, 'drawerFilterRows');
    const row = classBlock(css, 'drawerFilterRow');
    const label = classBlock(css, 'drawerFilterLabel');
    const chips = classBlock(css, 'drawerFilterChips');
    const chip = classBlock(css, 'drawerFilterChip');

    expect(bar.includes('border-radius: 12px')).toBe(true);
    expect(bar.includes('padding: 9px 10px')).toBe(true);
    expect(rows.includes('gap: 7px')).toBe(true);
    expect(row.includes('grid-template-columns: minmax(68px, max-content) minmax(0, 1fr)')).toBe(true);
    expect(row.includes('gap: 8px')).toBe(true);
    expect(label.includes('min-height: 26px')).toBe(true);
    expect(chips.includes('column-gap: 6px')).toBe(true);
    expect(chips.includes('row-gap: 5px')).toBe(true);
    expect(chip.includes('min-height: 26px')).toBe(true);
    expect(chip.includes('padding: 0 10px')).toBe(true);
    expect(chip.includes('font-size: 12px')).toBe(true);
  });

  test('prevents drawer cards from being squeezed flat inside the scroll list', () => {
    const css = readSource(new URL('../index.module.css', import.meta.url));
    const card = classBlock(css, 'drawerCard');

    expect(card.includes('flex: 0 0 auto')).toBe(true);
  });

  test('keeps the drawer search field compact instead of a nested white box', () => {
    const css = readSource(new URL('../index.module.css', import.meta.url));

    expect(css.includes('.drawerSearchInput:global(.arco-input-inner-wrapper)')).toBe(true);
    expect(css.includes('height: 42px')).toBe(true);
    expect(css.includes('min-height: 44px')).toBe(false);
    expect(css.includes('0 10px 24px rgba(26, 20, 51, 0.04)')).toBe(false);
    expect(css.includes('.drawerSearchInput :global(.arco-input-prefix)')).toBe(true);
  });
});
