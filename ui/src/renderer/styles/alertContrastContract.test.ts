/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

/**
 * Alert 对比度契约 / Alert contrast contract.
 *
 * 这不是字符串断言：它把 arco-override.css 里 Alert 的 `color-mix()` 声明真的算出来，
 * 再按 WCAG 相对亮度公式求"前景 vs 底色"，逐主题、逐明暗、逐 Alert 类型校验。
 * 之所以要算：Arco 把 Alert 底色挂在 `--color-*-light-1`（暗色值只存在于
 * `body[arco-theme='dark']`），字色却被氛围预设按 `[data-theme='dark'] body` 重写，
 * 两个属性各管一半，历史上让暗色 warning Alert 掉到 1.02–1.07:1。
 * 改挂 data-theme 侧的语义 token 之后，唯一能悄悄退回去的方式就是改混色比例
 * 或改引用的变量 —— 这两种都会被下面的数值断言抓到。
 *
 * Real computation, not a string match: it evaluates the `color-mix()` values
 * declared in arco-override.css and checks the WCAG contrast of Alert body copy
 * against its own surface for every built-in theme, in both light and dark.
 */

import { describe, expect, test } from 'bun:test';
import { readFileSync } from 'node:fs';
import { PRESET_THEMES } from '@renderer/pages/settings/DisplaySettings/presets';

const overrideCss = readFileSync(new URL('./arco-override.css', import.meta.url), 'utf8');
const defaultSchemeCss = readFileSync(new URL('./themes/default-color-scheme.css', import.meta.url), 'utf8');

/** WCAG 2.x 正文最小对比度 / minimum contrast for body copy. */
const MIN_TEXT_RATIO = 4.5;
/** WCAG 1.4.11 非文本图形（状态图标）最小对比度 / minimum contrast for the status icon. */
const MIN_ICON_RATIO = 3;

const ALERT_TYPES = ['info', 'success', 'warning', 'error'] as const;
type AlertType = (typeof ALERT_TYPES)[number];

type Rgb = [number, number, number];

// ---------------------------------------------------------------------------
// Colour maths
// ---------------------------------------------------------------------------

const clamp255 = (value: number): number => Math.min(255, Math.max(0, value));

const parseHex = (raw: string): Rgb | null => {
  const hex = raw.slice(1);
  if (hex.length === 3) {
    return [
      Number.parseInt(hex[0] + hex[0], 16),
      Number.parseInt(hex[1] + hex[1], 16),
      Number.parseInt(hex[2] + hex[2], 16),
    ];
  }
  if (hex.length === 6 || hex.length === 8) {
    return [
      Number.parseInt(hex.slice(0, 2), 16),
      Number.parseInt(hex.slice(2, 4), 16),
      Number.parseInt(hex.slice(4, 6), 16),
    ];
  }
  return null;
};

const parseFunctionalRgb = (raw: string): Rgb | null => {
  const match = raw.match(/^rgba?\(([^)]+)\)$/i);
  if (!match) return null;
  const parts = match[1]
    .split(/[,/\s]+/)
    .filter(Boolean)
    .map(Number);
  if (parts.length < 3 || parts.some((part) => Number.isNaN(part))) return null;
  return [parts[0], parts[1], parts[2]];
};

/**
 * `color-mix(in srgb, A P%, B)` —— sRGB 空间里的逐通道线性插值。两个操作数都是不透明色时
 * 这就是完整语义，Alert 的底色正是这种情形。
 */
const parseColorMix = (raw: string, resolve: (value: string) => Rgb | null): Rgb | null => {
  const match = raw.match(/^color-mix\(\s*in\s+srgb\s*,\s*(.+)\)$/i);
  if (!match) return null;
  const args = splitTopLevel(match[1]);
  if (args.length !== 2) return null;

  const operands = args.map((arg) => {
    const percent = arg.match(/(-?[\d.]+)%\s*$/);
    const colour = percent ? arg.slice(0, arg.length - percent[0].length).trim() : arg.trim();
    return { colour, weight: percent ? Number(percent[1]) / 100 : null };
  });

  const first = resolve(operands[0].colour);
  const second = resolve(operands[1].colour);
  if (!first || !second) return null;

  const firstWeight = operands[0].weight ?? (operands[1].weight === null ? 0.5 : 1 - operands[1].weight);
  const secondWeight = operands[1].weight ?? 1 - firstWeight;
  const total = firstWeight + secondWeight;
  if (total <= 0) return null;

  return [0, 1, 2].map((channel) =>
    clamp255((first[channel] * firstWeight + second[channel] * secondWeight) / total)
  ) as Rgb;
};

/** 按顶层逗号切分函数实参，避免切开嵌套的 var()/color-mix()。 */
const splitTopLevel = (input: string): string[] => {
  const parts: string[] = [];
  let depth = 0;
  let current = '';
  for (const char of input) {
    if (char === '(') depth += 1;
    if (char === ')') depth -= 1;
    if (char === ',' && depth === 0) {
      parts.push(current.trim());
      current = '';
      continue;
    }
    current += char;
  }
  if (current.trim()) parts.push(current.trim());
  return parts;
};

const srgbToLinear = (channel: number): number => {
  const ratio = channel / 255;
  return ratio <= 0.03928 ? ratio / 12.92 : Math.pow((ratio + 0.055) / 1.055, 2.4);
};

const relativeLuminance = ([r, g, b]: Rgb): number =>
  0.2126 * srgbToLinear(r) + 0.7152 * srgbToLinear(g) + 0.0722 * srgbToLinear(b);

const contrastRatio = (a: Rgb, b: Rgb): number => {
  const first = relativeLuminance(a);
  const second = relativeLuminance(b);
  const lighter = Math.max(first, second);
  const darker = Math.min(first, second);
  return (lighter + 0.05) / (darker + 0.05);
};

const round = (value: number): number => Math.round(value * 100) / 100;

// ---------------------------------------------------------------------------
// Stylesheet parsing
// ---------------------------------------------------------------------------

/** 剥掉注释，避免注释里的示例声明被当成真声明。 */
const stripComments = (css: string): string => css.replace(/\/\*[\s\S]*?\*\//g, '');

type Block = { selector: string; body: string };

/** 抽出所有顶层 `selector { … }` 块（跳过 @ 规则，主题契约禁止把变量写进 @media）。 */
const topLevelBlocks = (css: string): Block[] => {
  const source = stripComments(css);
  const blocks: Block[] = [];
  let depth = 0;
  let selectorStart = 0;
  let bodyStart = 0;
  for (let index = 0; index < source.length; index += 1) {
    const char = source[index];
    if (char === '{') {
      if (depth === 0) bodyStart = index;
      depth += 1;
    } else if (char === '}') {
      depth -= 1;
      if (depth === 0) {
        const selector = source.slice(selectorStart, bodyStart).trim();
        if (!selector.startsWith('@')) {
          blocks.push({ selector, body: source.slice(bodyStart + 1, index) });
        }
        selectorStart = index + 1;
      }
    }
  }
  return blocks;
};

const declarationsOf = (body: string): Map<string, string> => {
  const declarations = new Map<string, string>();
  for (const raw of splitDeclarations(body)) {
    const separator = raw.indexOf(':');
    if (separator < 0) continue;
    const property = raw.slice(0, separator).trim();
    const value = raw
      .slice(separator + 1)
      .replace(/!\s*important\s*$/i, '')
      .trim();
    if (property && value) declarations.set(property, value);
  }
  return declarations;
};

/** 值里可能出现分号以外的括号内容，按顶层分号切。 */
const splitDeclarations = (body: string): string[] => {
  const parts: string[] = [];
  let depth = 0;
  let current = '';
  for (const char of body) {
    if (char === '(') depth += 1;
    if (char === ')') depth -= 1;
    if (char === ';' && depth === 0) {
      parts.push(current);
      current = '';
      continue;
    }
    current += char;
  }
  if (current.trim()) parts.push(current);
  return parts;
};

const DARK_MARKER = "data-theme='dark'";

/**
 * 调色板块的选择器形状：`:root` / `body` / `[data-…]` / `[data-…] body` / `[data-…][data-…]`。
 * 刻意排除 `body[arco-theme='dark'] .xxx` 这类点缀块 —— 它们不属于契约里的变量块，
 * 而且 arco-theme 正是本次要摆脱的那个属性。
 */
const PALETTE_SELECTOR = /^(?::root|body|\[[^\]]+\](?:\[[^\]]+\])?(?:\s+body)?)$/;

const isPaletteSelector = (selector: string): boolean =>
  selector
    .split(',')
    .map((part) => part.trim())
    .every((part) => PALETTE_SELECTOR.test(part));

/**
 * 按主题契约（presets/README.md）取出亮/暗两套变量：亮块是不含 dark 标记的根/body 块，
 * 暗块是带 `[data-theme='dark']` 的块。同一模式下的多个块按出现顺序合并，后者覆盖前者。
 */
const paletteOf = (css: string, mode: 'light' | 'dark'): Map<string, string> => {
  const palette = new Map<string, string>();
  for (const block of topLevelBlocks(css)) {
    if (!isPaletteSelector(block.selector)) continue;
    const isDark = block.selector.includes(DARK_MARKER);
    if (isDark !== (mode === 'dark')) continue;
    for (const [property, value] of declarationsOf(block.body)) {
      if (property.startsWith('--')) palette.set(property, value);
    }
  }
  return palette;
};

/** 解析一个 CSS 颜色值，递归展开 var() 回退链与 color-mix()。 */
const makeResolver = (palette: Map<string, string>) => {
  const resolve = (raw: string, depth = 0): Rgb | null => {
    if (depth > 12) return null;
    const value = raw.trim();
    if (value.startsWith('#')) return parseHex(value);
    if (/^rgba?\(/i.test(value)) return parseFunctionalRgb(value);
    if (/^color-mix\(/i.test(value)) return parseColorMix(value, (inner) => resolve(inner, depth + 1));
    const varMatch = value.match(/^var\(\s*(--[\w-]+)\s*(?:,\s*([\s\S]+))?\)$/);
    if (varMatch) {
      const declared = palette.get(varMatch[1]);
      if (declared) return resolve(declared, depth + 1);
      return varMatch[2] ? resolve(varMatch[2], depth + 1) : null;
    }
    return null;
  };
  return resolve;
};

// ---------------------------------------------------------------------------
// The Alert contract as declared in arco-override.css
// ---------------------------------------------------------------------------

const alertSurfaceValue = (type: AlertType): string | null => {
  const block = topLevelBlocks(overrideCss).find((candidate) => candidate.selector === `.arco-alert-${type}`);
  return block ? (declarationsOf(block.body).get('background-color') ?? null) : null;
};

const alertTextValue = (property: 'primary' | 'secondary'): string | null => {
  const needle =
    property === 'primary'
      ? '.arco-alert-warning .arco-alert-content'
      : '.arco-alert-warning.arco-alert-with-title .arco-alert-content';
  const block = topLevelBlocks(overrideCss).find((candidate) =>
    candidate.selector.split(',').some((part) => part.trim() === needle)
  );
  return block ? (declarationsOf(block.body).get('color') ?? null) : null;
};

const alertIconValue = (type: AlertType): string | null => {
  const block = topLevelBlocks(overrideCss).find(
    (candidate) => candidate.selector === `.arco-alert-${type} .arco-alert-icon-wrapper svg`
  );
  return block ? (declarationsOf(block.body).get('color') ?? null) : null;
};

type Palette = { name: string; mode: 'light' | 'dark'; palette: Map<string, string> };

const palettes = (): Palette[] => {
  const all: Palette[] = [];
  for (const mode of ['light', 'dark'] as const) {
    all.push({ name: 'default', mode, palette: paletteOf(defaultSchemeCss, mode) });
    for (const theme of PRESET_THEMES) {
      if (!theme.css) continue;
      all.push({ name: theme.id ?? theme.name, mode, palette: paletteOf(theme.css, mode) });
    }
  }
  return all;
};

describe('alert contrast contract', () => {
  test('declares a surface, a text colour and an icon colour for every Alert type', () => {
    const missing: string[] = [];
    for (const type of ALERT_TYPES) {
      if (!alertSurfaceValue(type)) missing.push(`.arco-alert-${type} background-color`);
      if (!alertIconValue(type)) missing.push(`.arco-alert-${type} icon color`);
    }
    if (!alertTextValue('primary')) missing.push('.arco-alert-* .arco-alert-content color');
    if (!alertTextValue('secondary')) missing.push('.arco-alert-*.arco-alert-with-title .arco-alert-content color');
    expect(missing).toEqual([]);
  });

  test('keys Alert colours off data-theme tokens, never off the arco-theme-only Arco tints', () => {
    const offenders: string[] = [];
    const forbidden = [
      '--color-warning-light-',
      '--color-primary-light-',
      '--color-success-light-',
      '--color-danger-light-',
      '--color-text-',
      '--primary-6',
    ];
    const values = [
      ...ALERT_TYPES.map((type) => alertSurfaceValue(type)),
      ...ALERT_TYPES.map((type) => alertIconValue(type)),
      alertTextValue('primary'),
      alertTextValue('secondary'),
    ];
    for (const value of values) {
      for (const token of forbidden) {
        if (value && value.includes(token)) offenders.push(`${value} references ${token}*`);
      }
    }
    expect(offenders).toEqual([]);
  });

  test('body copy clears 4.5:1 and the status icon 3:1 against the Alert surface, every theme and mode', () => {
    const offenders: string[] = [];
    for (const { name, mode, palette } of palettes()) {
      const resolve = makeResolver(palette);
      for (const type of ALERT_TYPES) {
        const surfaceValue = alertSurfaceValue(type);
        const surface = surfaceValue ? resolve(surfaceValue) : null;
        if (!surface) {
          offenders.push(`${name}/${mode}/${type}: cannot resolve surface "${surfaceValue}"`);
          continue;
        }
        const foregrounds = [
          ['content', alertTextValue('primary'), MIN_TEXT_RATIO],
          ['with-title content', alertTextValue('secondary'), MIN_TEXT_RATIO],
          ['icon', alertIconValue(type), MIN_ICON_RATIO],
        ] as const;
        for (const [label, declared, floor] of foregrounds) {
          const foreground = declared ? resolve(declared) : null;
          if (!foreground) {
            offenders.push(`${name}/${mode}/${type}: cannot resolve ${label} colour "${declared}"`);
            continue;
          }
          const ratio = contrastRatio(foreground, surface);
          if (ratio < floor) {
            offenders.push(`${name}/${mode}/${type} ${label}: ${round(ratio)}:1 < ${floor}:1`);
          }
        }
      }
    }
    expect(offenders).toEqual([]);
  });

  test('every built-in theme supplies the semantic and surface tokens the contract reads', () => {
    const offenders: string[] = [];
    for (const { name, mode, palette } of palettes()) {
      for (const token of [
        '--info',
        '--success',
        '--warning',
        '--danger',
        '--bg-1',
        '--text-primary',
        '--text-secondary',
      ]) {
        if (!palette.has(token)) offenders.push(`${name}/${mode} is missing ${token}`);
      }
    }
    expect(offenders).toEqual([]);
  });
});
