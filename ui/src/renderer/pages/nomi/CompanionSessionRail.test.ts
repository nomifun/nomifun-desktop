/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { readFileSync } from 'node:fs';
import { describe, expect, test } from 'bun:test';

const source = readFileSync(new URL('./CompanionSessionRail.tsx', import.meta.url), 'utf8');

describe('CompanionSessionRail layout', () => {
  test('places the create companion entry above the companion roster', () => {
    const createEntry = source.indexOf('onClick={openCreate}');
    const roster = source.indexOf('{companions.map((p) => {');

    expect(createEntry).toBeGreaterThan(-1);
    expect(roster).toBeGreaterThan(-1);
    expect(createEntry).toBeLessThan(roster);
  });

  test('renders the create companion entry as the selected card-style design', () => {
    const createEntry = source.slice(source.indexOf('onClick={openCreate}'), source.indexOf('<div className=\'flex-1'));

    expect(createEntry.includes('w-30px h-30px')).toBe(true);
    expect(createEntry.includes('shadow-[0_5px_12px_rgba(var(--primary-rgb),0.22)]')).toBe(true);
    expect(createEntry.includes("t('nomi.companions.create')")).toBe(true);
    expect(createEntry.includes("t('nomi.companions.createHint')")).toBe(true);
  });
});
