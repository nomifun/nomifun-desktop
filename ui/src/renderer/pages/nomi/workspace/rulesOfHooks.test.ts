/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

/**
 * Guards the Rules of Hooks across the companion workspace.
 *
 * This exists because of a real bug: `index.tsx` had `if (loading) return <Spin/>`
 * and then declared another `useCallback` below it. The first render (loading)
 * called 18 hooks, the second (loaded) called 19, and React threw "Rendered more
 * hooks than during the previous render" — which then cascaded into unrelated
 * pages failing with bogus context errors. tsc, the linters and every unit test
 * were all green; only opening the page revealed it.
 *
 * A static check is the right tool here: the failure needs a two-render sequence
 * to reproduce, so unit tests miss it, and it is trivially detectable in source.
 */

import { describe, expect, test } from 'bun:test';
import { readFileSync, readdirSync, statSync } from 'node:fs';
import { fileURLToPath } from 'node:url';
import { dirname, join, relative } from 'node:path';

const nomiRoot = join(dirname(fileURLToPath(import.meta.url)), '..');
/** ContentAside was introduced by this redesign, so it is in scope too. */
const asideRoot = join(nomiRoot, '..', '..', 'components', 'layout', 'ContentAside');

const sourceFiles = (): string[] => {
  const out: string[] = [];
  const walk = (dir: string) => {
    for (const entry of readdirSync(dir)) {
      const full = join(dir, entry);
      if (statSync(full).isDirectory()) walk(full);
      else if (/\.tsx?$/.test(entry) && !/\.test\.tsx?$/.test(entry)) out.push(full);
    }
  };
  walk(nomiRoot);
  walk(asideRoot);
  return out;
};

/**
 * Mask comments and string/template literals line by line, so the returned array
 * indexes 1:1 with the file on disk and reported line numbers are exact.
 */
const maskedLines = (src: string): string[] => {
  let inBlockComment = false;
  return src.split('\n').map((raw) => {
    let line = raw;
    if (inBlockComment) {
      const end = line.indexOf('*/');
      if (end === -1) return '';
      line = line.slice(end + 2);
      inBlockComment = false;
    }
    // Drop any complete /* ... */ on this line, then open a block if one starts.
    line = line.replace(/\/\*.*?\*\//g, ' ');
    const open = line.indexOf('/*');
    if (open !== -1) {
      inBlockComment = true;
      line = line.slice(0, open);
    }
    const lineComment = line.indexOf('//');
    if (lineComment !== -1) line = line.slice(0, lineComment);
    return line
      .replace(/`(?:[^`\\]|\\.)*`/g, '``')
      .replace(/'(?:[^'\\]|\\.)*'/g, "''")
      .replace(/"(?:[^"\\]|\\.)*"/g, '""');
  });
};

const HOOK_CALL = /(?:^|[^.\w])(use[A-Z]\w*)\s*(?:<[^;()]*>)?\s*\(/;

/**
 * Report hook calls that sit below a component-level early `return`.
 *
 * Only conditional returns at the component's own statement level count. A
 * `return` nested inside a callback (`useMemo(() => { return 1; }, [])`) is
 * irrelevant and must not be flagged, or every component with a memo would trip
 * the check — so the scan tracks whether the block it is inside was opened by an
 * `if`/`else`/`switch` at the component's top level, not by a function body.
 */
const hooksAfterEarlyReturn = (src: string): string[] => {
  const lines = maskedLines(src);
  const findings: string[] = [];
  let depth = 0;
  let inComponent = false;
  let bodyDepth = 0;
  let earlyReturnLine = 0;
  /** Depth at which a top-level `if`/`else`/`switch` block was opened, or 0. */
  let conditionalDepth = 0;

  lines.forEach((line, index) => {
    if (!inComponent && /^(?:export\s+)?const\s+(?:[A-Z]\w*|use[A-Z]\w*)\b.*=>\s*\{\s*$/.test(line)) {
      inComponent = true;
      bodyDepth = depth + 1;
      earlyReturnLine = 0;
      conditionalDepth = 0;
    }

    if (inComponent && earlyReturnLine > 0 && depth === bodyDepth && HOOK_CALL.test(line)) {
      findings.push(`line ${index + 1}: ${HOOK_CALL.exec(line)?.[1]} after the early return on line ${earlyReturnLine}`);
    }

    if (inComponent) {
      // Single-line guard: `if (!x) return null;`
      if (depth === bodyDepth && /^\s*if\s*\(.*\)\s*return\b/.test(line) && earlyReturnLine === 0) {
        earlyReturnLine = index + 1;
      }
      // A top-level conditional opening a block.
      if (depth === bodyDepth && /^\s*(?:\}\s*else\b|if\s*\(|switch\s*\()/.test(line) && /\{\s*$/.test(line)) {
        conditionalDepth = depth + 1;
      }
      // A `return` inside that conditional block.
      if (conditionalDepth > 0 && depth >= conditionalDepth && /^\s*return[\s(;]/.test(line) && earlyReturnLine === 0) {
        earlyReturnLine = index + 1;
      }
    }

    depth += (line.match(/\{/g) || []).length - (line.match(/\}/g) || []).length;
    if (inComponent && conditionalDepth > 0 && depth < conditionalDepth) conditionalDepth = 0;
    if (inComponent && depth < bodyDepth) inComponent = false;
  });

  return findings;
};

describe('Rules of Hooks', () => {
  test('no hook is declared below a component-level early return', () => {
    const offenders = sourceFiles()
      .map((f) => ({ name: relative(nomiRoot, f), findings: hooksAfterEarlyReturn(readFileSync(f, 'utf8')) }))
      .filter(({ findings }) => findings.length > 0)
      .map(({ name, findings }) => `${name} — ${findings.join('; ')}`);
    expect(offenders).toEqual([]);
  });

  test('the detector actually detects (guards against a vacuous check)', () => {
    const bad = [
      'const Thing: React.FC = () => {',
      '  const a = useState(0);',
      '  if (!a) {',
      '    return null;',
      '  }',
      '  const b = useCallback(() => {}, []);',
      '  return null;',
      '};',
    ].join('\n');
    expect(hooksAfterEarlyReturn(bad).length).toBe(1);

    const good = [
      'const Thing: React.FC = () => {',
      '  const a = useState(0);',
      '  const b = useCallback(() => {}, []);',
      '  if (!a) {',
      '    return null;',
      '  }',
      '  return null;',
      '};',
    ].join('\n');
    expect(hooksAfterEarlyReturn(good)).toEqual([]);
  });
});
