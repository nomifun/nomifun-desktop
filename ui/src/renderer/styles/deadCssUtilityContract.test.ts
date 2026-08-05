/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

/**
 * Guards the root causes of the dead CSS utility classes that
 * `bun run check:dead-css` ratchets. The check script stops new *usages*; these
 * tests stop the *sources* of those usages from coming back:
 *
 *  - the unreachable `borderColors` theme block in uno.config.ts (UnoCSS eats
 *    `-b-` as the bottom direction before consulting the theme, so its keys
 *    could never be reached), and
 *  - MIGRATION.md recommending `border-b-base` / `border-b-light` as the "base
 *    border" utilities, which is how the existing occurrences got written.
 */
import { describe, expect, test } from 'bun:test';
import { readFileSync } from 'node:fs';
import { dirname, join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

const stylesDir = dirname(fileURLToPath(import.meta.url));
const uiRoot = resolve(stylesDir, '../../..');

const unoConfig = readFileSync(join(uiRoot, 'uno.config.ts'), 'utf8');
const migrationGuide = readFileSync(join(stylesDir, 'MIGRATION.md'), 'utf8');

describe('dead border utility root causes', () => {
  test('uno.config.ts carries no unreachable border color keys', () => {
    for (const key of ["'b-base'", "'b-light'", "'b-1'", "'b-2'", "'b-3'"]) {
      expect(unoConfig.includes(`${key}:`)).toBe(false);
    }
    expect(unoConfig.includes('...borderColors')).toBe(false);
  });

  test('uno.config.ts keeps the ramp rule that emits a parseable rgb()', () => {
    expect(unoConfig.includes('rgb(var(--${color}-${d}))')).toBe(true);
  });

  test('MIGRATION.md documents the -b- direction trap instead of recommending it', () => {
    expect(migrationGuide.includes('border-[var(--border-base)]')).toBe(true);
    expect(migrationGuide.includes('-b- 方向陷阱')).toBe(true);
    expect(migrationGuide.includes('check:dead-css')).toBe(true);
  });

  test('MIGRATION.md quick reference no longer offers the dead border utilities', () => {
    const quickRefBorderLine =
      migrationGuide.split('\n').find((line) => line.startsWith('- **边框**:')) ?? '';

    expect(quickRefBorderLine.length > 0).toBe(true);
    expect(quickRefBorderLine.includes('border-[var(--border-base)]')).toBe(true);
    expect(quickRefBorderLine.includes('`border-b-base`')).toBe(false);
    expect(quickRefBorderLine.includes('`border-b-light`')).toBe(false);
  });

  test('MIGRATION.md documents the slash-alpha ramp trap and its replacement', () => {
    expect(migrationGuide.includes('text-[rgb(var(--danger-6))]')).toBe(true);
    expect(migrationGuide.includes('text-danger-6')).toBe(true);
    // The explicit-alpha form is legal and must stay called out as such.
    expect(migrationGuide.includes('rgba(var(--primary-6),0.12)')).toBe(true);
  });
});
