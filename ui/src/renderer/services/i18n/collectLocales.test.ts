/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

/**
 * Every `nomi.collect.*` key the 采集 sections ask for must exist in both
 * locales.
 *
 * `t()` is not typed against `I18nKey` at these call sites, and each one passes a
 * Chinese `defaultValue`, so a mistyped or un-migrated key renders that Chinese
 * fallback — in BOTH locales — with nothing failing. `check:i18n` cannot catch it
 * either: it guards cross-locale parity of the JSON, not whether the code asks for
 * keys that are actually there. These keys were moved wholesale out of
 * `settings.privacy.*` when the controls moved from 设置 › 数据采集 into the 进化
 * tab, which is exactly the kind of rename that leaves one call site behind.
 *
 * Keys are scraped from source rather than listed by hand so the list cannot
 * drift; the three template-literal keys are expanded over the same constants the
 * component iterates.
 */

import { describe, expect, test } from 'bun:test';
import { readFileSync } from 'node:fs';
import { fileURLToPath } from 'node:url';
import enNomi from './locales/en-US/nomi.json';
import zhNomi from './locales/zh-CN/nomi.json';
import {
  COLLECTION_SOURCE_KEYS,
  SOURCE_SENSITIVITY,
} from '../../pages/nomi/workspace/tabs/EvolutionTab/useCollectSettings';

type LocaleJson = Record<string, unknown>;

const SOURCE_FILES = [
  '../../pages/nomi/workspace/tabs/EvolutionTab/index.tsx',
  '../../pages/nomi/workspace/tabs/EvolutionTab/CollectionSourcesSection.tsx',
  '../../pages/nomi/workspace/tabs/EvolutionTab/RetentionSection.tsx',
  '../../pages/nomi/workspace/tabs/EvolutionTab/StopAllSection.tsx',
] as const;

/** Literal `nomi.collect.…` keys appearing anywhere in the collect sources. */
const scrapedKeys = (): string[] => {
  const found = new Set<string>();
  for (const rel of SOURCE_FILES) {
    const source = readFileSync(fileURLToPath(new URL(rel, import.meta.url)), 'utf8');
    for (const match of source.matchAll(/nomi\.collect\.([A-Za-z0-9_.]+)/g)) {
      // Trailing dot = the static prefix of a template literal (`…items.${key}`),
      // which the expansion below covers instead.
      if (!match[1].endsWith('.')) found.add(`collect.${match[1]}`);
    }
  }
  return [...found].sort();
};

/** The dynamically-keyed rows, expanded over the constants the section maps. */
const templateKeys = (): string[] => [
  ...COLLECTION_SOURCE_KEYS.flatMap((key) => [
    `collect.sources.items.${key}.name`,
    `collect.sources.items.${key}.desc`,
  ]),
  ...[...new Set(Object.values(SOURCE_SENSITIVITY))].map((level) => `collect.sources.sensitivity.${level}`),
];

function getLocaleValue(locale: LocaleJson, key: string): unknown {
  let cursor: unknown = locale;
  for (const segment of key.split('.')) {
    if (!cursor || typeof cursor !== 'object' || !Object.prototype.hasOwnProperty.call(cursor, segment)) {
      return undefined;
    }
    cursor = (cursor as LocaleJson)[segment];
  }
  return cursor;
}

describe('nomi.collect locales', () => {
  const keys = [...new Set([...scrapedKeys(), ...templateKeys()])].sort();

  test('the scrape found the sections it was pointed at', () => {
    // A refactor that renames or moves these files would otherwise turn this
    // whole suite into a vacuous pass over an empty key list.
    expect(keys.length > 20).toBe(true);
    expect(keys.includes('collect.sources.title')).toBe(true);
    expect(keys.includes('collect.retention.title')).toBe(true);
    expect(keys.includes('collect.stopAll.title')).toBe(true);
    expect(keys.includes('collect.loadFailed')).toBe(true);
  });

  for (const [localeName, locale] of [
    ['zh-CN', zhNomi as LocaleJson],
    ['en-US', enNomi as LocaleJson],
  ] as const) {
    test(`${localeName} defines every key the sections ask for`, () => {
      const missing = keys.filter((key) => typeof getLocaleValue(locale, key) !== 'string');
      expect(missing).toEqual([]);
    });
  }

  test('no key was left behind under settings.privacy.*', () => {
    // The retired namespace named a page that no longer exists.
    for (const rel of SOURCE_FILES) {
      const source = readFileSync(fileURLToPath(new URL(rel, import.meta.url)), 'utf8');
      expect(source.includes('settings.privacy.')).toBe(false);
    }
  });
});
