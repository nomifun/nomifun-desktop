/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

/**
 * Cross-locale key parity, plural-aware — the single copy of the rule shared by
 * the `check:i18n` gate (`scripts/generate-i18n-types.mjs`) and the per-namespace
 * locale tests.
 *
 * 两边过去各自实现同一条规则，答案却不一样：gate 会通过 `Intl.PluralRules` 解析
 * i18next 的 `_<category>` 复数后缀，而 locale 测试要求逐键字面相等。于是 zh-CN
 * 里那些它永远用不到的复数变体（中文只有 `other` 一个类别，`foo_one` 永远不会被
 * i18next 选中）删不掉——删了测试红，留着 gate 每次构建都刷 warning。规则只留一份，
 * 就是为了让这两个答案不再分叉。
 *
 * Deliberately dependency-free and DOM-free: the `check:i18n` build script imports
 * this file straight from the renderer tree (under bun) rather than keeping a second
 * copy that can drift.
 */

/**
 * Every CLDR plural category. A trailing `_<category>` on a key is i18next's
 * plural suffix (JSON v4, which is what i18next >= 21 uses without
 * `compatibilityJSON`), not part of the key name.
 */
export const PLURAL_CATEGORIES = ['zero', 'one', 'two', 'few', 'many', 'other'] as const;

export type PluralCategory = (typeof PLURAL_CATEGORIES)[number];

const PLURAL_SUFFIX_RE = new RegExp(`^(.*)_(${PLURAL_CATEGORIES.join('|')})$`);

/**
 * Split `foo.bar_one` into `{ base: 'foo.bar', category: 'one' }`; `null` when the
 * key carries no plural suffix. Anchored on the category list, so a key that merely
 * ends in an unrelated word (`the_others`) is not mistaken for a plural variant.
 */
export function splitPluralSuffix(
  key: string
): { base: string; category: PluralCategory } | null {
  const match = PLURAL_SUFFIX_RE.exec(key);
  if (!match) return null;
  return { base: match[1], category: match[2] as PluralCategory };
}

/**
 * The plural categories i18next will actually look up for a locale — the same
 * `Intl.PluralRules` set it resolves `count` through. en-US has two (`one`,
 * `other`); zh-CN has exactly one (`other`), because Chinese does not inflect for
 * number. So `foo_one` in en-US has NO counterpart to demand in zh-CN, and a set
 * comparison that demanded one would fail on correct translations.
 */
export function pluralCategories(locale: string): Set<string> {
  return new Set(
    new Intl.PluralRules(locale, { type: 'cardinal' }).resolvedOptions().pluralCategories
  );
}

/** One offending key, attributed to the locale that should have (or should drop) it. */
export interface LocaleKeyIssue {
  locale: string;
  key: string;
  reason: string;
}

export interface LocaleKeyParity {
  /** Real drift: a key one locale can resolve and another cannot. */
  errors: LocaleKeyIssue[];
  /** Dead weight: a variant i18next will never select for that locale. */
  warnings: LocaleKeyIssue[];
}

/**
 * Compare the flattened key sets of every shipped locale.
 *
 * `keysByLocale` maps a BCP 47 locale tag (the tag is fed to `Intl.PluralRules`,
 * so it must be the real one, not a nickname) to that locale's flattened keys.
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
 * weight rather than drift: i18next will never resolve it, so nothing breaks, but
 * nothing maintains it either. Reported as a warning so callers can choose — the
 * gate prints it, the locale tests refuse it.
 */
export function diffLocaleKeys(
  keysByLocale: Record<string, readonly string[]>
): LocaleKeyParity {
  const locales = Object.keys(keysByLocale);
  const errors: LocaleKeyIssue[] = [];
  const warnings: LocaleKeyIssue[] = [];

  const plainByLocale = new Map<string, Set<string>>();
  const variantsByBase = new Map<string, Map<string, Set<string>>>();
  for (const locale of locales) {
    const plain = new Set<string>();
    plainByLocale.set(locale, plain);
    for (const key of keysByLocale[locale]) {
      const plural = splitPluralSuffix(key);
      if (!plural) {
        plain.add(key);
        continue;
      }
      if (!variantsByBase.has(plural.base)) variantsByBase.set(plural.base, new Map());
      const perLocale = variantsByBase.get(plural.base)!;
      if (!perLocale.has(locale)) perLocale.set(locale, new Set());
      perLocale.get(locale)!.add(plural.category);
    }
  }

  const allPlain = new Set(locales.flatMap((locale) => [...plainByLocale.get(locale)!]));
  for (const locale of locales) {
    for (const key of allPlain) {
      if (!plainByLocale.get(locale)!.has(key)) {
        const present = locales.filter((other) => plainByLocale.get(other)!.has(key));
        errors.push({ locale, key, reason: `present in ${present.join(', ')}` });
      }
    }
  }

  const categoriesByLocale = new Map(locales.map((locale) => [locale, pluralCategories(locale)]));
  for (const [base, perLocale] of variantsByBase) {
    const provided = new Set([...perLocale.values()].flatMap((set) => [...set]));
    for (const locale of locales) {
      const categories = categoriesByLocale.get(locale)!;
      const present = perLocale.get(locale) ?? new Set<string>();
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

  const order = (entry: LocaleKeyIssue): string => `${entry.locale} ${entry.key}`;
  const byLocaleThenKey = (a: LocaleKeyIssue, b: LocaleKeyIssue): number =>
    order(a) < order(b) ? -1 : order(a) > order(b) ? 1 : 0;
  return { errors: errors.sort(byLocaleThenKey), warnings: warnings.sort(byLocaleThenKey) };
}
