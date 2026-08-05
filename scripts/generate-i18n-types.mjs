#!/usr/bin/env node
/**
 * generate-i18n-types.mjs — regenerate ui/src/renderer/services/i18n/i18n-keys.d.ts
 * from the en-US locale JSON files (source of truth), and enforce that every
 * shipped locale carries the same keys.
 *
 * Usage:
 *   node scripts/generate-i18n-types.mjs             # write the d.ts
 *   node scripts/generate-i18n-types.mjs --check     # no write; exit 1 if the
 *                                                    # committed d.ts drifts from
 *                                                    # the locale key set
 *   node scripts/generate-i18n-types.mjs --self-test # exercise the parity rule
 *                                                    # against fixtures, no repo read
 *
 * Either mode also fails on cross-language drift (see `diffLocaleKeys`): the d.ts is
 * generated from ONE locale, so without this a key added to zh-CN only, or dropped
 * from zh-CN only, is invisible — the type still typechecks and the runtime silently
 * falls back to English.
 *
 * No dependencies. Node >= 16.
 *
 * Rules (mirrors the historical generator output):
 * - Namespaces and their order come from locales/en-US/index.ts (runtime truth).
 * - Keys are the dot-flattened paths of every leaf value, prefixed with the
 *   namespace; arrays flatten to numeric indices (e.g. `a.list.0`).
 * - I18nKey union is sorted by UTF-16 code units; I18nModule keeps index.ts order.
 * - Output uses LF line endings (repo-wide `.gitattributes`: `* text=auto eol=lf`).
 */

import fs from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

const repoRoot = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..');
const i18nDir = path.join(repoRoot, 'ui', 'src', 'renderer', 'services', 'i18n');
const localesDir = path.join(i18nDir, 'locales');
const i18nConfigFile = path.join(repoRoot, 'ui', 'src', 'common', 'config', 'i18n-config.json');
const outFile = path.join(i18nDir, 'i18n-keys.d.ts');

const checkMode = process.argv.includes('--check');
const selfTestMode = process.argv.includes('--self-test');

/** Parse a locale's index.ts: namespace export order + json file per namespace. */
function readNamespaces(localeDir) {
  const src = fs.readFileSync(path.join(localeDir, 'index.ts'), 'utf8');

  const importMap = new Map(); // identifier -> json filename
  const importRe = /import\s+(\w+)\s+from\s+'\.\/([\w.-]+)\.json'/g;
  for (let m; (m = importRe.exec(src)); ) importMap.set(m[1], `${m[2]}.json`);

  const block = src.match(/export\s+default\s*\{([\s\S]*?)\}/);
  if (!block) throw new Error(`export default block not found in ${path.join(localeDir, 'index.ts')}`);
  const names = block[1]
    .split(',')
    .map((s) => s.trim())
    .filter(Boolean);

  const namespaces = names.map((name) => {
    // supports shorthand (`common`) and aliased (`starOffice: starOffice`) entries
    const [exportName, ident = exportName] = name.split(':').map((s) => s.trim());
    const file = importMap.get(ident);
    if (!file) throw new Error(`namespace '${exportName}' in index.ts has no matching JSON import`);
    return { name: exportName, file };
  });

  // Orphan JSON files (present on disk, not exported) are drift the runtime
  // cannot see — surface them loudly but do not include their keys.
  const referenced = new Set(namespaces.map((n) => n.file));
  const orphans = fs
    .readdirSync(localeDir)
    .filter((f) => f.endsWith('.json') && !referenced.has(f));
  for (const f of orphans) {
    process.stderr.write(
      `warning: ${f} exists in ${path.basename(localeDir)} but is not exported by index.ts (keys excluded)\n`,
    );
  }

  return namespaces;
}

/** Dot-flatten a JSON value into `out`; arrays become numeric segments. */
function flatten(value, prefix, out) {
  if (Array.isArray(value)) {
    value.forEach((v, i) => flatten(v, `${prefix}.${i}`, out));
  } else if (value !== null && typeof value === 'object') {
    for (const [k, v] of Object.entries(value)) flatten(v, `${prefix}.${k}`, out);
  } else {
    out.push(prefix);
  }
}

function collectKeys(namespaces, localeDir) {
  const keys = [];
  for (const { name, file } of namespaces) {
    const json = JSON.parse(fs.readFileSync(path.join(localeDir, file), 'utf8'));
    flatten(json, name, keys);
  }
  // Some locale files (e.g. settings.json) contain both a flat dotted key
  // ("assistant.botToken") and a nested object ("assistant": { "botToken" })
  // that flatten to the same path. The union type lists each key once, so we
  // dedupe — but surface the collisions as a lint warning.
  const seen = new Set();
  const dupes = new Set();
  for (const k of keys) (seen.has(k) ? dupes : seen).add(k);
  if (dupes.size) {
    process.stderr.write(
      `warning: ${dupes.size} flattened key collisions (flat dotted key + nested object), deduped:\n  ${[...dupes].join('\n  ')}\n`,
    );
  }
  return [...seen].sort(); // UTF-16 code unit order, matches historical output
}

// ── Cross-language parity ──────────────────────────────────────────────────────

/**
 * Every CLDR plural category. A trailing `_<category>` on a key is i18next's
 * plural suffix (JSON v4, which is what i18next ≥21 uses without
 * `compatibilityJSON`), not part of the key name.
 */
const PLURAL_CATEGORIES = ['zero', 'one', 'two', 'few', 'many', 'other'];
const PLURAL_SUFFIX_RE = new RegExp(`^(.*)_(${PLURAL_CATEGORIES.join('|')})$`);

/**
 * The plural categories i18next will actually look up for a locale — the same
 * `Intl.PluralRules` set it resolves `count` through. en-US has two (`one`,
 * `other`); zh-CN has exactly one (`other`), because Chinese does not inflect for
 * number. So `foo_one` in en-US has NO counterpart to demand in zh-CN, and a set
 * comparison that demanded one would fail on correct translations.
 */
function pluralCategories(locale) {
  return new Set(new Intl.PluralRules(locale, { type: 'cardinal' }).resolvedOptions().pluralCategories);
}

/**
 * Compare the flattened key sets of every shipped locale.
 *
 * `keysByLocale`: `{ [locale]: string[] }`. Returns `{ errors, warnings }`, both
 * arrays of `{ locale, key, reason }`.
 *
 * The rule, in two halves:
 *
 * 1. Keys with no plural suffix must exist in EVERY locale. This is the plain
 *    parity a naive set diff gives, and it is where real drift shows up (a key
 *    translated in one language and forgotten in the other).
 * 2. A plural variant `base_<category>` is required of a locale only when
 *    `category` is one of THAT locale's plural categories. So en-US must have
 *    `_one` + `_other` where zh-CN needs only `_other`; but any variant a sibling
 *    locale provides for a category this locale does have is mandatory — that is
 *    the hole `ssh.sessionsOnline_other` fell through (en-US had it, zh-CN did
 *    not, and `other` is the one category zh-CN has).
 *
 * A variant for a category the locale does NOT have (`foo_one` in zh-CN) is dead
 * weight rather than a bug: i18next will never resolve it. Reported as a warning,
 * not an error — some namespaces have a per-file locale test that still demands a
 * literal key-for-key match, so the two must be able to coexist.
 */
function diffLocaleKeys(keysByLocale) {
  const locales = Object.keys(keysByLocale);
  const errors = [];
  const warnings = [];

  const plainByLocale = new Map(); // locale -> Set(key)
  const variantsByBase = new Map(); // base -> Map(locale -> Set(category))
  for (const locale of locales) {
    const plain = new Set();
    plainByLocale.set(locale, plain);
    for (const key of keysByLocale[locale]) {
      const match = PLURAL_SUFFIX_RE.exec(key);
      if (!match) {
        plain.add(key);
        continue;
      }
      const [, base, category] = match;
      if (!variantsByBase.has(base)) variantsByBase.set(base, new Map());
      const perLocale = variantsByBase.get(base);
      if (!perLocale.has(locale)) perLocale.set(locale, new Set());
      perLocale.get(locale).add(category);
    }
  }

  const allPlain = new Set(locales.flatMap((locale) => [...plainByLocale.get(locale)]));
  for (const locale of locales) {
    for (const key of allPlain) {
      if (!plainByLocale.get(locale).has(key)) {
        const present = locales.filter((other) => plainByLocale.get(other).has(key));
        errors.push({ locale, key, reason: `present in ${present.join(', ')}` });
      }
    }
  }

  const categoriesByLocale = new Map(locales.map((locale) => [locale, pluralCategories(locale)]));
  for (const [base, perLocale] of variantsByBase) {
    const provided = new Set([...perLocale.values()].flatMap((set) => [...set]));
    for (const locale of locales) {
      const categories = categoriesByLocale.get(locale);
      const present = perLocale.get(locale) ?? new Set();
      for (const category of provided) {
        if (categories.has(category) && !present.has(category)) {
          errors.push({
            locale,
            key: `${base}_${category}`,
            reason: `plural form '${category}' exists for another locale and is one of ${locale}'s own plural categories`,
          });
        }
      }
      for (const category of present) {
        if (!categories.has(category)) {
          warnings.push({
            locale,
            key: `${base}_${category}`,
            reason: `'${category}' is not a plural category of ${locale}, so i18next never resolves this key`,
          });
        }
      }
    }
  }

  const order = (entry) => `${entry.locale} ${entry.key}`;
  const byLocaleThenKey = (a, b) => (order(a) < order(b) ? -1 : order(a) > order(b) ? 1 : 0);
  return { errors: errors.sort(byLocaleThenKey), warnings: warnings.sort(byLocaleThenKey) };
}

/** Read every shipped locale's flattened key set. Returns `{ keysByLocale, namespacesByLocale }`. */
function readAllLocales() {
  const { supportedLanguages, referenceLanguage } = JSON.parse(fs.readFileSync(i18nConfigFile, 'utf8'));
  if (!Array.isArray(supportedLanguages) || !supportedLanguages.includes(referenceLanguage)) {
    throw new Error(`${i18nConfigFile} must list referenceLanguage in supportedLanguages`);
  }
  // referenceLanguage first: its warnings read as the baseline, and the d.ts comes
  // from it. The rest keep the config's order for stable output.
  const locales = [referenceLanguage, ...supportedLanguages.filter((l) => l !== referenceLanguage)];
  const keysByLocale = {};
  const namespacesByLocale = {};
  for (const locale of locales) {
    const dir = path.join(localesDir, locale);
    if (!fs.existsSync(dir)) throw new Error(`supported language '${locale}' has no locales/${locale} directory`);
    const namespaces = readNamespaces(dir);
    namespacesByLocale[locale] = namespaces;
    keysByLocale[locale] = collectKeys(namespaces, dir);
  }
  return { locales, referenceLanguage, keysByLocale, namespacesByLocale };
}

/**
 * Report parity to stderr. Returns true when the locales agree.
 *
 * A namespace missing from one locale's index.ts would otherwise print as every
 * one of its keys, so it is reported on its own first.
 */
function reportParity({ locales, referenceLanguage, keysByLocale, namespacesByLocale }) {
  let ok = true;
  const reference = namespacesByLocale[referenceLanguage].map((n) => n.name);
  for (const locale of locales) {
    if (locale === referenceLanguage) continue;
    const names = namespacesByLocale[locale].map((n) => n.name);
    const missing = reference.filter((name) => !names.includes(name));
    const extra = names.filter((name) => !reference.includes(name));
    if (missing.length || extra.length) {
      ok = false;
      if (missing.length) console.error(`${locale}/index.ts does not export: ${missing.join(', ')}`);
      if (extra.length) console.error(`${locale}/index.ts exports namespaces ${referenceLanguage} does not: ${extra.join(', ')}`);
    }
  }

  const { errors, warnings } = diffLocaleKeys(keysByLocale);
  for (const { locale, key, reason } of warnings) {
    process.stderr.write(`warning: ${locale} '${key}' is unreachable — ${reason}\n`);
  }
  if (errors.length) {
    ok = false;
    for (const locale of locales) {
      const mine = errors.filter((error) => error.locale === locale);
      if (!mine.length) continue;
      console.error(`${locale} is missing ${mine.length} key${mine.length === 1 ? '' : 's'}:`);
      for (const { key, reason } of mine) console.error(`  ${key}  (${reason})`);
    }
    console.error(
      '\nlocale key sets must match: the d.ts is generated from one locale, so a one-sided key typechecks and then silently falls back at runtime.',
    );
  }
  return ok;
}

const quote = (s) => `'${s.replace(/\\/g, '\\\\').replace(/'/g, "\\'")}'`;
const union = (items) => items.map((k) => `  | ${quote(k)}`).join('\n');

function render(namespaces, keys) {
  return [
    '/* eslint-disable */',
    '/**',
    ' * AUTO-GENERATED FILE - DO NOT EDIT',
    ' * Generated by scripts/generate-i18n-types.mjs',
    ' */',
    '',
    'export type I18nKey =',
    `${union(keys)};`,
    '',
    'export type I18nModule =',
    `${union(namespaces.map((n) => n.name))};`,
    '',
  ].join('\n');
}

const normalize = (s) => s.replace(/\r\n/g, '\n');

/**
 * Prove the parity rule both bites and tolerates, on fixtures instead of the
 * repo: the two failure modes are "misses a real one-sided key" and "fails on a
 * correct translation", and only a self-test can show which one a change caused.
 */
function selfTest() {
  const en = 'en-US';
  const zh = 'zh-CN';
  const cases = [
    {
      name: 'a one-sided plain key fails, naming the locale that lacks it',
      keys: { [en]: ['a.kept', 'a.enOnly'], [zh]: ['a.kept'] },
      expectErrors: [`${zh}:a.enOnly`],
      expectWarnings: [],
    },
    {
      name: 'a one-sided plain key fails in the other direction too',
      keys: { [en]: ['a.kept'], [zh]: ['a.kept', 'a.zhOnly'] },
      expectErrors: [`${en}:a.zhOnly`],
      expectWarnings: [],
    },
    {
      name: "English's extra plural form is not demanded of Chinese",
      keys: { [en]: ['a.n_one', 'a.n_other'], [zh]: ['a.n_other'] },
      expectErrors: [],
      expectWarnings: [],
    },
    {
      name: 'the one plural form Chinese does have is demanded (the real bug)',
      keys: { [en]: ['a.n', 'a.n_other'], [zh]: ['a.n'] },
      expectErrors: [`${zh}:a.n_other`],
      expectWarnings: [],
    },
    {
      name: 'a plural form outside a locale’s categories warns instead of failing',
      keys: { [en]: ['a.n_one', 'a.n_other'], [zh]: ['a.n_one', 'a.n_other'] },
      expectErrors: [],
      expectWarnings: [`${zh}:a.n_one`],
    },
    {
      name: 'a key that merely ends in an unrelated word is not read as a plural',
      keys: { [en]: ['a.the_others'], [zh]: ['a.the_others'] },
      expectErrors: [],
      expectWarnings: [],
    },
  ];

  let failures = 0;
  for (const { name, keys, expectErrors, expectWarnings } of cases) {
    const { errors, warnings } = diffLocaleKeys(keys);
    const seen = (entries) => entries.map((e) => `${e.locale}:${e.key}`).sort();
    const expected = { errors: [...expectErrors].sort(), warnings: [...expectWarnings].sort() };
    const actual = { errors: seen(errors), warnings: seen(warnings) };
    const same =
      JSON.stringify(expected.errors) === JSON.stringify(actual.errors) &&
      JSON.stringify(expected.warnings) === JSON.stringify(actual.warnings);
    if (same) {
      console.log(`ok   ${name}`);
    } else {
      failures += 1;
      console.error(`FAIL ${name}`);
      console.error(`     expected errors ${JSON.stringify(expected.errors)}, got ${JSON.stringify(actual.errors)}`);
      console.error(`     expected warnings ${JSON.stringify(expected.warnings)}, got ${JSON.stringify(actual.warnings)}`);
    }
  }
  if (failures) {
    console.error(`\n${failures} of ${cases.length} parity self-tests failed`);
    process.exitCode = 1;
    return;
  }
  console.log(`all ${cases.length} parity self-tests passed`);
}

function main() {
  if (selfTestMode) {
    selfTest();
    return;
  }

  const locales = readAllLocales();
  const namespaces = locales.namespacesByLocale[locales.referenceLanguage];
  const keys = locales.keysByLocale[locales.referenceLanguage];
  const generated = render(namespaces, keys);

  const existing = fs.existsSync(outFile) ? normalize(fs.readFileSync(outFile, 'utf8')) : null;

  if (checkMode) {
    if (existing === generated) {
      console.log(`i18n-keys.d.ts is up to date (${keys.length} keys, ${namespaces.length} modules)`);
    } else {
      // Report drift at key granularity, then fall back to a text-level hint.
      const extractKeys = (text) => {
        const section = text.split('export type I18nModule')[0];
        return new Set([...section.matchAll(/\|\s+'((?:[^'\\]|\\.)*)'/g)].map((m) => m[1]));
      };
      const oldKeys = existing ? extractKeys(existing) : new Set();
      const newKeys = new Set(keys);
      const missing = keys.filter((k) => !oldKeys.has(k)); // in locales, not in d.ts
      const stale = [...oldKeys].filter((k) => !newKeys.has(k)); // in d.ts, not in locales
      if (missing.length) console.error(`missing from d.ts (${missing.length}):\n  ${missing.join('\n  ')}`);
      if (stale.length) console.error(`stale in d.ts (${stale.length}):\n  ${stale.join('\n  ')}`);
      if (!missing.length && !stale.length) console.error('key sets match but file text differs (ordering/header/EOL)');
      console.error('\ni18n-keys.d.ts is out of date — run: node scripts/generate-i18n-types.mjs');
      process.exitCode = 1;
    }
    if (!reportParity(locales)) process.exitCode = 1;
    return;
  }

  if (existing === generated) {
    console.log(`i18n-keys.d.ts already up to date (${keys.length} keys)`);
  } else {
    fs.writeFileSync(outFile, generated, 'utf8');
    console.log(`wrote ${path.relative(repoRoot, outFile)} (${keys.length} keys, ${namespaces.length} modules)`);
  }
  // Regenerating cannot fix cross-language drift — only editing the other locale
  // can — so this is reported (and failed on) after the write, not instead of it.
  if (!reportParity(locales)) process.exitCode = 1;
}

main();
