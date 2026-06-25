#!/usr/bin/env node
/**
 * 预设 CSS 主题契约校验 / Preset CSS theme contract checker
 * 校验 ui/src/renderer/pages/settings/DisplaySettings/presets/*.css（default.css 除外）
 * 是否符合 presets/README.md 的主题契约：
 *  - 双块结构（:root,body 亮块 + [data-theme='dark'],[data-theme='dark'] body 暗块，暗块在后）
 *  - A 表 + B 表变量全量覆盖，且亮/暗两块变量集合对称
 *  - --primary-rgb / --primary-1..7 为 RGB 三元组
 *  - 无布局属性、无 @import / 外联 url、变量不进 @media、大括号配平
 *  - 内容弹层背景不过透明、消息排版外层不套主题背景
 *  - 预览缩略图取色键存在
 *
 * 用法 / Usage: node scripts/check-theme-contract.mjs  (或 bun)
 */
import { readdirSync, readFileSync } from 'node:fs';
import { join, dirname } from 'node:path';
import { fileURLToPath } from 'node:url';

const PRESETS_DIR = join(
  dirname(fileURLToPath(import.meta.url)),
  '..',
  'ui/src/renderer/pages/settings/DisplaySettings/presets'
);

const range = (prefix, from, to) => Array.from({ length: to - from + 1 }, (_, i) => `${prefix}${from + i}`);

/** 契约 A 表：App 变量 / Contract table A: app variables */
const APP_VARS = [
  'color-primary',
  'primary',
  ...range('color-primary-light-', 1, 3),
  'color-primary-dark-1',
  'primary-rgb',
  'brand',
  'brand-light',
  'brand-hover',
  'color-brand-fill',
  'color-brand-bg',
  ...range('aou-', 1, 10),
  'bg-base',
  ...range('bg-', 1, 6),
  'bg-8',
  'bg-9',
  'bg-10',
  'bg-hover',
  'bg-active',
  'fill',
  'color-fill',
  'fill-0',
  'fill-white-to-black',
  'dialog-fill-0',
  'inverse',
  'text-primary',
  'text-secondary',
  'text-disabled',
  'text-0',
  'text-white',
  'border-base',
  'border-light',
  'border-special',
  'success',
  'warning',
  'danger',
  'info',
  'message-user-bg',
  'message-tips-bg',
  'workspace-btn-bg',
  'color-guid-agent-bar',
  'terminal-surface-bg',
  'terminal-border',
];

/** 契约 B 表：Arco token（必须打在含 body 的选择器组） / Contract table B: Arco tokens */
const ARCO_VARS = [
  ...range('color-bg-', 1, 5),
  'color-bg-popup',
  'color-bg-white',
  ...range('color-text-', 1, 4),
  ...range('color-fill-', 1, 4),
  'color-border',
  ...range('color-border-', 1, 4),
  'color-primary-light-4',
  ...range('primary-', 1, 7),
  'color-secondary',
  'color-secondary-hover',
  'color-secondary-active',
  'color-secondary-disabled',
  'color-tooltip-bg',
  'color-mask-bg',
  'color-spin-layer-bg',
  'color-menu-light-bg',
  'color-menu-dark-bg',
];

const REQUIRED = [...APP_VARS, ...ARCO_VARS];
const TRIPLET_VARS = ['primary-rgb', ...range('primary-', 1, 7)];
const PREVIEW_KEYS = ['bg-1', 'bg-2', 'bg-3', 'color-primary', 'color-text-3', 'color-fill-2', 'color-primary-light-3'];
const LAYOUT_PROPS = new Set([
  'display',
  'position',
  'overflow',
  'overflow-x',
  'overflow-y',
  'z-index',
  'width',
  'height',
  'min-width',
  'max-width',
  'min-height',
  'max-height',
  'margin',
  'margin-top',
  'margin-right',
  'margin-bottom',
  'margin-left',
  'padding',
  'padding-top',
  'padding-right',
  'padding-bottom',
  'padding-left',
]);

const MESSAGE_ITEM_FORBIDDEN_PROPS = new Set([
  'background',
  'background-color',
  'background-image',
  'backdrop-filter',
  '-webkit-backdrop-filter',
]);
const CONTENT_POPOVER_SELECTORS = ['.arco-popover-content', '.arco-dropdown-menu', '.arco-select-popup'];
const MIN_CONTENT_SURFACE_ALPHA = 0.86;

const stripComments = (css) => css.replace(/\/\*[\s\S]*?\*\//g, '');

/** 顶层块扫描（@media/@keyframes 整体视作一个块） / Top-level block scan */
const topLevelBlocks = (css) => {
  const blocks = [];
  let i = 0;
  while (i < css.length) {
    const open = css.indexOf('{', i);
    if (open === -1) break;
    const prevClose = css.lastIndexOf('}', open);
    const selector = css
      .slice(prevClose === -1 ? 0 : prevClose + 1, open)
      .trim()
      .replace(/\s+/g, ' ');
    let depth = 1;
    let j = open + 1;
    while (j < css.length && depth > 0) {
      if (css[j] === '{') depth++;
      else if (css[j] === '}') depth--;
      j++;
    }
    blocks.push({ selector, body: css.slice(open + 1, j - 1), start: open, end: j });
    i = j;
  }
  return blocks;
};

const collectVars = (body) => {
  const map = new Map();
  const re = /--([a-zA-Z0-9-_]+)\s*:\s*([^;]+);/g;
  let m;
  while ((m = re.exec(body)) !== null) map.set(m[1], m[2].trim());
  return map;
};

const isTriplet = (value) => /^\d{1,3}\s*,\s*\d{1,3}\s*,\s*\d{1,3}$/.test(value.replace(/\s*!important\s*/i, ''));

const declarationEntries = (body) => {
  const entries = [];
  const re = /(?:^|[{;])\s*([-\w]+)\s*:\s*([^;{}]+);/g;
  let m;
  while ((m = re.exec(body)) !== null) entries.push({ prop: m[1], value: m[2].trim() });
  return entries;
};

const rgbaAlphaValues = (value) => {
  const alphas = [];
  const re = /rgba\(\s*[^,]+\s*,\s*[^,]+\s*,\s*[^,]+\s*,\s*([0-9.]+)\s*\)/gi;
  let m;
  while ((m = re.exec(value)) !== null) alphas.push(Number(m[1]));
  return alphas.filter((n) => Number.isFinite(n));
};

const checkTheme = (file, css) => {
  const problems = [];
  const cleaned = stripComments(css);

  // 大括号配平 / brace balance
  const opens = (cleaned.match(/\{/g) || []).length;
  const closes = (cleaned.match(/\}/g) || []).length;
  if (opens !== closes) problems.push(`大括号不配平: { x${opens} vs } x${closes}`);

  // 外联资源 / external resources
  if (/@import\b/.test(cleaned)) problems.push('包含 @import');
  if (/url\(\s*['"]?https?:/i.test(cleaned)) problems.push('包含外联 url(http...)');

  const blocks = topLevelBlocks(cleaned);
  const lightBlocks = blocks.filter((b) => /:root/.test(b.selector) && /(^|[,\s])body\b/.test(b.selector));
  const darkBlocks = blocks.filter(
    (b) => /\[data-theme=["']?dark["']?\]/.test(b.selector) && /\[data-theme=["']?dark["']?\]\s+body\b/.test(b.selector)
  );

  if (lightBlocks.length === 0) problems.push('缺少亮色块（选择器需同时含 :root 与 body）');
  if (darkBlocks.length === 0) problems.push("缺少暗色块（选择器需含 [data-theme='dark'] 与 [data-theme='dark'] body）");
  if (lightBlocks.length && darkBlocks.length && darkBlocks[0].start < lightBlocks[0].start) {
    problems.push('暗色块出现在亮色块之前');
  }

  const lightVars = new Map();
  for (const b of lightBlocks) for (const [k, v] of collectVars(b.body)) lightVars.set(k, v);
  const darkVars = new Map();
  for (const b of darkBlocks) for (const [k, v] of collectVars(b.body)) darkVars.set(k, v);

  for (const v of REQUIRED) {
    if (!lightVars.has(v)) problems.push(`亮色块缺变量 --${v}`);
    if (!darkVars.has(v)) problems.push(`暗色块缺变量 --${v}`);
  }
  // 对称性（除契约清单外的自定义变量也要求对称，--sider-section-title-color 例外）
  const symmetricExempt = new Set(['sider-section-title-color']);
  for (const k of lightVars.keys()) {
    if (!darkVars.has(k) && !symmetricExempt.has(k)) problems.push(`变量 --${k} 只在亮色块出现（不对称）`);
  }
  for (const k of darkVars.keys()) {
    if (!lightVars.has(k) && !symmetricExempt.has(k)) problems.push(`变量 --${k} 只在暗色块出现（不对称）`);
  }

  for (const v of TRIPLET_VARS) {
    for (const [mode, vars] of [
      ['亮', lightVars],
      ['暗', darkVars],
    ]) {
      const value = vars.get(v);
      if (value && !isTriplet(value)) problems.push(`${mode}色块 --${v} 不是 RGB 三元组: "${value}"`);
    }
  }

  for (const key of PREVIEW_KEYS) {
    if (!lightVars.has(key)) problems.push(`预览取色键 --${key} 在亮色块缺失`);
  }

  // 布局属性（@keyframes 块内豁免——其内是动画帧）/ layout props outside keyframes
  for (const b of blocks) {
    if (/@(?:-webkit-)?keyframes\b/.test(b.selector)) continue;
    for (const { prop, value } of declarationEntries(b.body)) {
      if (LAYOUT_PROPS.has(prop)) problems.push(`布局属性 "${prop}" 出现在选择器 "${b.selector.slice(0, 60)}"`);

      if (b.selector.includes('.message-item') && MESSAGE_ITEM_FORBIDDEN_PROPS.has(prop)) {
        problems.push(`禁止给 .message-item 设置 "${prop}"（消息排版外层不能套主题背景）`);
      }

      const isContentPopover = CONTENT_POPOVER_SELECTORS.some((selector) => b.selector.includes(selector));
      if (isContentPopover && (prop === 'background' || prop === 'background-color')) {
        const lowAlpha = rgbaAlphaValues(value).find((alpha) => alpha < MIN_CONTENT_SURFACE_ALPHA);
        if (lowAlpha != null) {
          problems.push(
            `内容弹层 "${b.selector.slice(0, 60)}" 的 ${prop} 透明度 ${lowAlpha} 过低（需 >= ${MIN_CONTENT_SURFACE_ALPHA}）`
          );
        }
      }
    }
    // 变量不进 @media / vars must not live inside @media
    if (/^@media\b/.test(b.selector) && /--[a-zA-Z0-9-_]+\s*:/.test(b.body)) {
      problems.push('@media 块内定义了 CSS 变量（预览解析不到）');
    }
  }

  return problems;
};

const files = readdirSync(PRESETS_DIR).filter((f) => f.endsWith('.css') && f !== 'default.css');
let failed = false;
for (const file of files) {
  const css = readFileSync(join(PRESETS_DIR, file), 'utf8');
  const problems = checkTheme(file, css);
  if (problems.length) {
    failed = true;
    console.log(`✗ ${file}`);
    for (const p of problems) console.log(`   - ${p}`);
  } else {
    console.log(`✓ ${file}`);
  }
}
if (!files.length) {
  console.log('（presets 目录下没有非 default 主题）');
}
process.exit(failed ? 1 : 0);
