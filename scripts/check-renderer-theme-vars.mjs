#!/usr/bin/env node
/**
 * Reject renderer CSS variable references that have no declaration in the
 * renderer or Arco's shipped stylesheet. A missing var() without a fallback
 * silently discards the entire CSS declaration, which can hide controls when
 * light and dark backgrounds change. Also reject fixed white text on solid
 * theme accent fills, where one foreground cannot contrast in every mode.
 *
 * This checks names, not color contrast. Contrast and preset symmetry have
 * separate contracts in check-theme-contract.mjs and the renderer tests.
 */
import { existsSync, readFileSync, readdirSync } from 'node:fs';
import { dirname, join, relative, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

const ROOT = resolve(dirname(fileURLToPath(import.meta.url)), '..');
const RENDERER = join(ROOT, 'ui/src/renderer');
const ARCO_CSS = join(ROOT, 'ui/node_modules/@arco-design/web-react/dist/css/arco.css');
const SOURCE_EXTENSIONS = new Set(['.css', '.ts', '.tsx']);

// UnoCSS injects these variables into generated utilities rather than source.
const GENERATED_PREFIXES = ['--un-'];

// Preserve offsets so diagnostics still point at the original source line.
const blankComment = (comment) => comment.replace(/[^\n]/g, ' ');
const stripComments = (source) => source
  .replace(/\/\*[\s\S]*?\*\//g, blankComment)
  .replace(/(^|\s)\/\/[^\n]*/gm, blankComment);

const lineNumberAt = (source, index) => {
  let line = 1;
  for (let i = 0; i < index; i += 1) if (source[i] === '\n') line += 1;
  return line;
};

const entries = (source, expression) => {
  const matches = [];
  for (const match of source.matchAll(expression)) matches.push({ name: match[1], line: lineNumberAt(source, match.index) });
  return matches;
};

export const declaredVariables = (source) => {
  const cleaned = stripComments(source);
  const declared = new Set();
  // CSS declarations and quoted TS/JS style object keys.
  for (const { name } of entries(cleaned, /(?:^|[;{,\s])['"]?(--[\w-]+)['"]?\s*:/gm)) declared.add(name);
  // Imperative styles, including custom properties assigned after render.
  for (const { name } of entries(cleaned, /\.setProperty\(\s*['"`](--[\w-]+)['"`]/g)) declared.add(name);
  return declared;
};

export const referencedVariables = (source) => {
  const cleaned = stripComments(source);
  const references = [];
  for (const match of cleaned.matchAll(/var\(\s*(--[\w-]+)/g)) {
    // Optional component variables deliberately use local fallbacks. They are
    // safe even when no preset declares them, and should not need an allowlist.
    const next = cleaned.slice(match.index + match[0].length).trimStart()[0];
    if (next === ',') continue;
    references.push({ name: match[1], line: lineNumberAt(cleaned, match.index) });
  }
  return references;
};

export const findMissingVariables = (sources, externalDeclarations = '') => {
  const available = declaredVariables(externalDeclarations);
  for (const { source } of sources) for (const name of declaredVariables(source)) available.add(name);

  const missing = [];
  for (const { path, source } of sources) {
    for (const reference of referencedVariables(source)) {
      if (available.has(reference.name) || GENERATED_PREFIXES.some((prefix) => reference.name.startsWith(prefix))) continue;
      missing.push({ path, ...reference });
    }
  }
  return missing;
};

/**
 * Catch the most common contrast regression: a solid, theme-dependent accent
 * background with fixed white text. This intentionally does not inspect media
 * overlays, translucent surfaces, or the full CSS cascade.
 */
export const findFixedWhiteOnThemeFill = (sources) => {
  const problems = [];
  const accentClass = /(?:^|\s)!?bg-(?:primary(?:-6)?|brand|danger(?:-6)?)(?=\s|$)/;
  const whiteClass = /(?:^|\s)!?text-white(?=\s|$)/;
  const accentDeclaration = /(?:^|;)\s*background(?:-color)?\s*:\s*(?:var\(--(?:color-primary|primary|brand|danger)\)|rgb\(var\(--primary-6\)\))\s*(?:!important\s*)?;/;
  const whiteDeclaration = /(?:^|;)\s*color\s*:\s*(?:#fff(?:fff)?|white|var\(--(?:inverse|text-white)\))\s*(?:!important\s*)?;/i;

  for (const { path, source } of sources) {
    const cleaned = stripComments(source);
    if (path.endsWith('.css')) {
      for (const match of cleaned.matchAll(/\{([^{}]*)\}/g)) {
        if (!accentDeclaration.test(match[1]) || !whiteDeclaration.test(match[1])) continue;
        problems.push({ path, line: lineNumberAt(cleaned, match.index), reason: 'theme accent fill paired with fixed white text' });
      }
      continue;
    }
    // Uno classes often live in JSX/template strings. Limit the check to one
    // string literal, so independent branches and separate controls are safe.
    for (const match of cleaned.matchAll(/(['"`])((?:(?!\1)[\s\S])*?)\1/g)) {
      if (!accentClass.test(match[2]) || !whiteClass.test(match[2])) continue;
      problems.push({ path, line: lineNumberAt(cleaned, match.index), reason: 'theme accent class paired with text-white' });
    }
  }
  return problems;
};

const collectSources = (directory, files = []) => {
  for (const entry of readdirSync(directory, { withFileTypes: true })) {
    const path = join(directory, entry.name);
    if (entry.isDirectory()) {
      collectSources(path, files);
      continue;
    }
    if (![...SOURCE_EXTENSIONS].some((extension) => entry.name.endsWith(extension))) continue;
    if (/\.(?:test|spec|stories)\.[cm]?[jt]sx?$/.test(entry.name) || entry.name.endsWith('.d.ts')) continue;
    files.push({ path: relative(ROOT, path).replaceAll('\\', '/'), source: readFileSync(path, 'utf8') });
  }
  return files;
};

if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  if (!existsSync(ARCO_CSS)) {
    console.error(`Missing Arco stylesheet: ${relative(ROOT, ARCO_CSS)}. Install UI dependencies before running this check.`);
    process.exit(1);
  }
  const sources = collectSources(RENDERER);
  const missing = findMissingVariables(sources, readFileSync(ARCO_CSS, 'utf8'));
  const fixedWhite = findFixedWhiteOnThemeFill(sources);
  for (const { path, line, name } of missing) console.error(`${path}:${line}: undefined CSS variable ${name}`);
  for (const { path, line, reason } of fixedWhite) console.error(`${path}:${line}: ${reason}`);
  console.log(missing.length || fixedWhite.length
    ? `${missing.length} undefined renderer CSS variables; ${fixedWhite.length} fixed-white accent pairings`
    : 'Renderer theme color references resolve');
  process.exit(missing.length || fixedWhite.length ? 1 : 0);
}
