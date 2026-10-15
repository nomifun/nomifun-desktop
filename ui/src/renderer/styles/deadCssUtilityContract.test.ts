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
 *  - documentation recommending `border-b-base` / `border-b-light` as base
 *    border utilities instead of explaining the direction trap,
 *  - colors.ts recommending doubled-prefix classes (`bg-bg-0` and friends), and
 *  - a global `* { border-width: 0; border-style: solid }` reset sneaking into
 *    uno.config.ts's preflights. That reset would make form 7 (border width +
 *    colour with no border-style) impossible to write — and would also strip the
 *    default borders off native form controls. The maintained styling guide
 *    records that constraint, and this test keeps the reset out.
 */
import { describe, expect, test } from 'bun:test';
import { readFileSync } from 'node:fs';
import { dirname, join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

const stylesDir = dirname(fileURLToPath(import.meta.url));
const uiRoot = resolve(stylesDir, '../../..');

const unoConfig = readFileSync(join(uiRoot, 'uno.config.ts'), 'utf8');
const stylingGuide = readFileSync(
  resolve(uiRoot, '../docs/contributing/styling.md'),
  'utf8',
);
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

  test('styling guide documents the direction trap and valid border color', () => {
    expect(stylingGuide.includes('border-[var(--border-base)]')).toBe(true);
    expect(stylingGuide.includes('direction trap')).toBe(true);
    expect(stylingGuide.includes('check:dead-css')).toBe(true);
  });

  test('styling guide documents the slash-alpha ramp trap and its replacement', () => {
    expect(stylingGuide.includes('text-[rgb(var(--danger-6))]')).toBe(true);
    expect(stylingGuide.includes('text-danger-6')).toBe(true);
    // The explicit-alpha form is legal and must stay called out as such.
    expect(stylingGuide.includes('rgba(var(--primary-6),0.12)')).toBe(true);
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

  test('styling guide names both doubled-prefix and missing background keys', () => {
    expect(stylingGuide.includes('bg-bg-N')).toBe(true);
    expect(stylingGuide.includes('`bg-0`')).toBe(true);
  });
});

describe('no global border reset', () => {
  test('uno.config.ts preflights do not reset border-width or border-style', () => {
    // A `* { border-width: 0; border-style: solid }` preflight is the Tailwind cure
    // for form 7, and it is deliberately absent — see styling.md for the reason.
    // 只看 preflights 块本身：头注里提到这两个属性名是讲解，不是重置。
    const preflightBlock = unoConfig.slice(unoConfig.indexOf('preflights:'), unoConfig.indexOf('shortcuts:'));
    expect(preflightBlock.length > 0).toBe(true);
    const offenders = ['border-width', 'border-style', '@unocss/reset'].filter(
      (needle) => preflightBlock.includes(needle) || unoConfig.includes(`import '${needle}`),
    );
    expect(offenders).toEqual([]);
  });

  test('styling guide records the native-control reason to avoid a global reset', () => {
    expect(stylingGuide.includes('border-width: 0; border-style: solid;')).toBe(true);
    expect(stylingGuide.includes('native control borders')).toBe(true);
  });

  test('styling guide spells out that directional widths need directional styles', () => {
    expect(stylingGuide.includes('border-b border-b-solid')).toBe(true);
    expect(stylingGuide.includes('border-width: medium')).toBe(true);
  });
});
