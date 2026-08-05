/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

/**
 * Guards the root causes of the dead CSS utility classes that
 * `bun run check:dead-css` bans. The check script stops new *usages*; these
 * tests stop the *sources* of those usages from coming back:
 *
 *  - the unreachable `borderColors` theme block in uno.config.ts (UnoCSS eats
 *    `-b-` as the bottom direction before consulting the theme, so its keys
 *    could never be reached),
 *  - MIGRATION.md recommending `border-b-base` / `border-b-light` as the "base
 *    border" utilities, which is how the existing occurrences got written,
 *  - colors.ts recommending the doubled-prefix classes (`bg-bg-0` and friends),
 *    which is where all 87 `bg-bg-N` usages came from, and
 *  - a global `* { border-width: 0; border-style: solid }` reset sneaking into
 *    uno.config.ts's preflights. That reset would make form 7 (border width +
 *    colour with no border-style) impossible to write — and would also strip the
 *    default borders off every native form control in the app. It was evaluated
 *    and rejected; MIGRATION.md records why, and this test keeps it out.
 */
import { describe, expect, test } from 'bun:test';
import { readFileSync } from 'node:fs';
import { dirname, join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

const stylesDir = dirname(fileURLToPath(import.meta.url));
const uiRoot = resolve(stylesDir, '../../..');

const unoConfig = readFileSync(join(uiRoot, 'uno.config.ts'), 'utf8');
const migrationGuide = readFileSync(join(stylesDir, 'MIGRATION.md'), 'utf8');
const colorsModule = readFileSync(join(stylesDir, 'colors.ts'), 'utf8');

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

describe('doubled-prefix root cause', () => {
  test('colors.ts no longer recommends the doubled-prefix classes', () => {
    // The header used to hand out `bg-bg-0` / `text-text` / `border-border` as the
    // recommended atomic classes. It may keep naming them as counter-examples, but
    // never as advice, and it must say what to write instead.
    expect(colorsModule.includes('bg-1..bg-6')).toBe(true);
    expect(colorsModule.includes('border-arco-2')).toBe(true);
    expect(colorsModule.includes('ZERO CSS')).toBe(true);
  });

  test('MIGRATION.md documents that bg-bg-N and bg-0 both emit nothing', () => {
    expect(migrationGuide.includes('bg-bg-N')).toBe(true);
    expect(migrationGuide.includes('`bg-0` **也是死的**')).toBe(true);
  });
});

describe('no global border reset', () => {
  test('uno.config.ts preflights do not reset border-width or border-style', () => {
    // A `* { border-width: 0; border-style: solid }` preflight is the Tailwind cure
    // for form 7, and it is deliberately absent — see MIGRATION.md for the reason.
    // 只看 preflights 块本身：头注里提到这两个属性名是讲解，不是重置。
    const preflightBlock = unoConfig.slice(unoConfig.indexOf('preflights:'), unoConfig.indexOf('shortcuts:'));
    expect(preflightBlock.length > 0).toBe(true);
    const offenders = ['border-width', 'border-style', '@unocss/reset'].filter(
      (needle) => preflightBlock.includes(needle) || unoConfig.includes(`import '${needle}`),
    );
    expect(offenders).toEqual([]);
  });

  test('MIGRATION.md records that the global reset was considered and rejected', () => {
    expect(migrationGuide.includes('为什么不加全局 border reset')).toBe(true);
    expect(migrationGuide.includes('border-width: 0; border-style: solid;')).toBe(true);
    expect(migrationGuide.includes('1000+')).toBe(true);
  });

  test('MIGRATION.md spells out that directional widths need directional styles', () => {
    expect(migrationGuide.includes('border-b border-b-solid')).toBe(true);
    expect(migrationGuide.includes('border-width: medium')).toBe(true);
  });
});
