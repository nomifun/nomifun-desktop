/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

/**
 * Structure contract for the rebuilt 桌面伙伴 workspace.
 *
 * This replaces the former `index.structure.test.ts`, which asserted exact
 * `className` strings and per-tab `NomiSettingRow` counts and therefore had to be
 * rewritten by hand on every visual tweak. What is pinned here instead is the
 * stuff a regression would actually break: the tab registry, the URL contract,
 * and the invariants of the features this redesign deleted — because those are
 * strings and dynamic lookups that `tsc` cannot catch coming back.
 *
 * Assertions collect offending files into a list and assert it is empty, so a
 * failure names the file instead of just saying `true !== false`.
 */

import { describe, expect, test } from 'bun:test';
import { readFileSync, readdirSync, statSync } from 'node:fs';
import { fileURLToPath } from 'node:url';
import { dirname, join, relative } from 'node:path';
import { WORKSPACE_TABS, isWorkspaceTabKey } from './types';

const here = dirname(fileURLToPath(import.meta.url));
const nomiRoot = join(here, '..');
const read = (rel: string) => readFileSync(join(nomiRoot, rel), 'utf8');

/** Every .ts/.tsx under pages/nomi, excluding tests. */
const sourceFiles = (): string[] => {
  const out: string[] = [];
  const walk = (dir: string) => {
    for (const entry of readdirSync(dir)) {
      const full = join(dir, entry);
      if (statSync(full).isDirectory()) {
        walk(full);
        continue;
      }
      if (!/\.tsx?$/.test(entry) || /\.test\.tsx?$/.test(entry)) continue;
      out.push(full);
    }
  };
  walk(nomiRoot);
  return out;
};

/**
 * Source with comments stripped. The deletion invariants must judge *code*, not
 * prose: several files legitimately explain in a comment why a removed concept is
 * absent, and matching those would make the test unfixable except by deleting the
 * explanation.
 */
const codeOnly = (s: string): string => s.replace(/\/\*[\s\S]*?\*\//g, '').replace(/^\s*\/\/.*$/gm, '');

interface Source {
  name: string;
  code: string;
  raw: string;
}

const corpus: Source[] = sourceFiles().map((f) => {
  const raw = readFileSync(f, 'utf8');
  return { name: relative(nomiRoot, f), code: codeOnly(raw), raw };
});

/** Files where `predicate` holds — the failure message becomes the file list. */
const offenders = (predicate: (src: Source) => boolean): string[] =>
  corpus.filter(predicate).map((src) => src.name);

describe('workspace tab registry', () => {
  test('is the seven tabs from the redesign, in reading order', () => {
    expect([...WORKSPACE_TABS]).toEqual(['overview', 'memory', 'remote', 'evolution', 'skills', 'history', 'other']);
  });

  test('rejects the retired tab keys so a stale deep link falls back instead of blanking', () => {
    // These were real ?tab= values before the redesign. A bookmark or an old pet
    // menu entry can still carry them, and the shell must treat them as unknown
    // (→ default to 总览) rather than rendering an undefined component.
    const retired = ['suggestions', 'collect', 'learn', 'review', 'settings', 'memories', 'chat', 'figures'];
    expect(retired.filter((key) => isWorkspaceTabKey(key))).toEqual([]);
    expect(isWorkspaceTabKey(null)).toBe(false);
  });

  test('the shell maps exactly the declared tabs to components', () => {
    const shell = read('index.tsx');
    const start = shell.indexOf('TAB_COMPONENTS');
    const registry = shell.slice(start, shell.indexOf('};', start));
    const mapped = [...registry.matchAll(/^\s{2}(\w+):/gm)].map((m) => m[1]);
    // A missing entry is a type error (the Record is keyed by the union), but an
    // EXTRA hand-written key is not — and would render a tab the strip never offers.
    expect(mapped.sort()).toEqual([...WORKSPACE_TABS].sort());
  });
});

describe('URL contract', () => {
  test('selection, tab and the figure view all live in the query string', () => {
    const shell = read('index.tsx');
    expect(shell.includes("searchParams.get('companion')")).toBe(true);
    expect(shell.includes("searchParams.get('tab')")).toBe(true);
    expect(shell.includes("searchParams.get('view')")).toBe(true);
  });

  test('param writes replace rather than push, so Back leaves the page', () => {
    const shell = read('index.tsx');
    const setters = shell.match(/setSearchParams\(/g)?.length ?? 0;
    expect(setters).toBeGreaterThan(0);
    // Pushing would make Back walk through tab switches instead of leaving /nomi.
    expect(shell.match(/\{ replace: true \}/g)?.length ?? 0).toBe(setters);
  });
});

describe('deleted features stay deleted', () => {
  test('no 建议 / suggestion surface', () => {
    // The memory MERGE assistant is an unrelated live feature whose endpoint is
    // literally named merge-suggestions.
    expect(
      offenders(({ code }) =>
        code.split('\n').some((line) => /suggestion/i.test(line) && !/merge[-_]?[Ss]uggestion/i.test(line))
      )
    ).toEqual([]);
  });

  test('no 技能专精 framing', () => {
    expect(offenders(({ code }) => code.includes('专精'))).toEqual([]);
  });

  test('no 赠送 / cross-companion skill gift', () => {
    expect(offenders(({ code }) => code.includes('赠送') || /giftSkill/.test(code))).toEqual([]);
  });

  test('no cross-companion skill scope at all', () => {
    // `include_shared` is gone from the API, not merely defaulted to false: a
    // companion's skill list is its own rows, so the identifier itself must not
    // come back in any form (a request param, a prop, a local flag).
    expect(offenders(({ code }) => /include_shared/.test(code))).toEqual([]);
  });

  test('no shared-memory scope selector', () => {
    // What was deleted is the *control*: the 共享/私有 radio, the owner picker and the
    // 此伙伴可见/全部伙伴 view switch.
    expect(
      offenders(({ code }) =>
        /scopeShared|scopePrivateOf|scopePickCompanion|scopeFilterSelf|scopeFilterAll|scopeMode|scopeSelector/.test(code)
      )
    ).toEqual([]);
  });

  test('the 共享记忆 concept is gone from the workspace entirely', () => {
    // 共享记忆 was deleted as a product concept: memory is strictly per-companion.
    // `scope_kind` was the DB discriminator that used to encode it ('user' = shared);
    // the column is now physically gone (ownership is one nullable owner column) and
    // the backend never sends it, so NO workspace surface may read it — not to
    // filter, not to badge. Ownership questions are answered by the owner id alone,
    // which now travels under the column's own name, `companion_id`.
    expect(offenders(({ code }) => /scope_kind/.test(code))).toEqual([]);
    // The install-wide memory read-outs went with it (their i18n keys are deleted
    // too, so a leftover call would render the defaultValue and silently lie).
    expect(offenders(({ code }) => /nomi\.memory\.installWide/.test(code))).toEqual([]);
  });

  test('nothing in the workspace still calls learning or evolution install-wide', () => {
    // 学习 / 进化 moved off the shared config onto each companion's profile, so
    // every "applies to every companion" disclosure — and the `installWide` /
    // `ownsLearningOutput` flags that gated them — is gone. Their i18n keys are
    // deleted, so a leftover call site would render its defaultValue and lie to
    // the user with no test failing anywhere else.
    expect(
      offenders(({ code }) => /installWideNote|installWide|ownsLearningOutput|InstallWideNote/.test(code))
    ).toEqual([]);
    // The tab must write through the per-companion profile, never the shared config.
    expect(
      offenders(
        ({ name, code }) => name.includes('tabs/EvolutionTab/') && /patchSharedConfig|getSharedConfig/.test(code)
      )
    ).toEqual([]);
  });

  test('no cross-companion 共享 domain switch', () => {
    expect(offenders(({ code }) => /nomi\.domains\.|SHARED_TABS|useCompanionShared/.test(code))).toEqual([]);
  });
});

describe('house style', () => {
  test('no dead border utilities', () => {
    // Verified against the UnoCSS config: `border-border-N` matches no colour and
    // emits nothing, and `border-b-*` is parsed as border-BOTTOM-color off the --bg
    // ramp rather than the intended base border.
    expect(offenders(({ raw }) => /\bborder-border-\d/.test(raw))).toEqual([]);
    expect(offenders(({ raw }) => /\bborder-b-(?:base|light)\b/.test(raw))).toEqual([]);
  });

  test('colour ramps go through the project rule, not an arbitrary rgb()', () => {
    // Measured with the real generator: `text-[rgb(var(--danger-6))]` compiles to
    //   color: rgb(var(--danger-6) / var(--un-text-opacity))
    // because UnoCSS injects slash-alpha into arbitrary colour values. The ramp
    // variables are COMMA-separated triplets (`--red-6: 245,63,63`), so
    // `rgb(245,63,63 / 1)` is unparseable and the browser drops the declaration —
    // the element silently keeps its inherited colour. The project's own rule
    // (`text-danger-6`) emits a valid `color: rgb(var(--danger-6))`.
    //
    // Only the bare `rgb(var(--ramp-N))` form is affected. An explicit
    // `rgba(var(--ramp-N), 0.12)` carries its own alpha, so nothing is injected and
    // it stays valid — those are deliberately not matched here.
    expect(
      offenders(({ raw }) => /\b(?:text|bg|border)-\[rgb\(var\(--(?:primary|danger|success|warning)-[1-9]\)\)\]/.test(raw))
    ).toEqual([]);
  });

  test('icons come from @icon-park/react only', () => {
    expect(offenders(({ raw }) => raw.includes('@arco-design/web-react/icon'))).toEqual([]);
  });

  test('icon-park imports are never aliased', () => {
    // A build plugin rewrites `import { X }` into `X as _X`; a pre-existing alias
    // produces `X as Y as _X as Y` and 500s the module at runtime while tsc is happy.
    expect(
      offenders(({ raw }) =>
        [...raw.matchAll(/import\s*\{([^}]*)\}\s*from\s*'@icon-park\/react'/g)].some(([, names]) =>
          names.includes(' as ')
        )
      )
    ).toEqual([]);
  });

  test('deleting a companion is a danger-styled confirm', () => {
    const shell = read('index.tsx');
    expect(shell.includes('Modal.confirm')).toBe(true);
    expect(shell.includes("okButtonProps: { status: 'danger' }")).toBe(true);
  });

  test('every source file carries the license header', () => {
    expect(offenders(({ raw }) => !raw.startsWith('/**\n * @license'))).toEqual([]);
  });
});
