/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { readFileSync } from 'node:fs';
import { describe, expect, test } from 'bun:test';
import enBrowser from '../../services/i18n/locales/en-US/browser.json';
import zhBrowser from '../../services/i18n/locales/zh-CN/browser.json';
import { diffLocaleKeys } from '../../services/i18n/localeKeyParity';

const readSource = (url: URL): string => readFileSync(url, 'utf8');

const flatten = (
  value: unknown,
  prefix = '',
  result: Record<string, unknown> = {}
): Record<string, unknown> => {
  if (value !== null && typeof value === 'object' && !Array.isArray(value)) {
    for (const [key, child] of Object.entries(value)) {
      flatten(child, prefix ? `${prefix}.${key}` : key, result);
    }
  } else {
    result[prefix] = value;
  }
  return result;
};

const componentSources = [
  new URL('./index.tsx', import.meta.url),
  new URL('./BrowserPageHeader.tsx', import.meta.url),
  new URL('./BrowserHostDiagnostics.tsx', import.meta.url),
  new URL('./BrowserInventoryTree.tsx', import.meta.url),
  new URL('./BrowserLaneDetails.tsx', import.meta.url),
  new URL(
    '../../components/layout/Sider/SiderNav/SiderBrowserEntry.tsx',
    import.meta.url
  ),
].map(readSource);

const placeholders = (value: string): string[] =>
  [...value.matchAll(/{{\s*([^{}\s]+)\s*}}/g)]
    .map((match) => match[1])
    .sort();

describe('Browser UI localization', () => {
  test('keeps English and Chinese Browser locale leaves aligned and populated', () => {
    const en = flatten(enBrowser);
    const zh = flatten(zhBrowser);

    // Plural-aware parity, borrowed from the `check:i18n` gate rather than
    // re-implemented: a literal key-for-key comparison here demanded that zh-CN carry
    // an `_one` variant for every English one, even though Chinese has a single plural
    // category (`other`) and i18next can therefore never select `_one`. Those dead
    // zh-CN keys existed only to satisfy this assertion, and the gate warned about
    // every one of them on every build.
    const { errors, warnings } = diffLocaleKeys({
      'en-US': Object.keys(en).map((key) => `browser.${key}`),
      'zh-CN': Object.keys(zh).map((key) => `browser.${key}`),
    });
    const oneSided = errors.map(({ locale, key, reason }) => `${locale} lacks ${key} (${reason})`);
    expect(oneSided).toEqual([]);
    // A variant for a category the locale does not have is dead weight: nothing
    // resolves it, nothing updates it when the sibling copy changes.
    const unreachable = warnings.map(({ locale, key }) => `${locale} ${key}`);
    expect(unreachable).toEqual([]);

    const blank: string[] = [];
    for (const [locale, leaves] of [
      ['en-US', en],
      ['zh-CN', zh],
    ] as const) {
      for (const key of Object.keys(leaves)) {
        const leaf = leaves[key];
        if (typeof leaf !== 'string' || !leaf.trim()) blank.push(`${locale} ${key}`);
      }
    }
    expect(blank).toEqual([]);

    // Placeholders are compared only where both locales define the key: a plural
    // variant the other locale legitimately does not have (English `_one`) has no
    // counterpart to compare against, and its base key is checked here anyway.
    const mismatchedPlaceholders: string[] = [];
    for (const key of Object.keys(en)) {
      if (!(key in zh)) continue;
      const enPlaceholders = placeholders(en[key] as string).join(',');
      const zhPlaceholders = placeholders(zh[key] as string).join(',');
      if (enPlaceholders !== zhPlaceholders) {
        mismatchedPlaceholders.push(`${key} (en-US {${enPlaceholders}} vs zh-CN {${zhPlaceholders}})`);
      }
    }
    expect(mismatchedPlaceholders).toEqual([]);
  });

  test('routes every Browser surface through the Browser translation namespace', () => {
    for (const source of componentSources) {
      expect(source.includes('useTranslation')).toBe(true);
      expect(source.includes("t('browser.")).toBe(true);
    }
  });

  test('defines close-all as draining and verifying every managed resource class', () => {
    const enCopy = `${enBrowser.page.closeAll} ${enBrowser.close.allTitle} ${enBrowser.close.allContent} ${enBrowser.close.allSuccess} ${enBrowser.close.drainUnconfirmed}`;
    for (const phrase of [
      'browser resources',
      'browser lane',
      'pending cleanup',
      'managed Browser Hosts/processes',
      'authoritative remaining counts',
      'all zero',
    ]) {
      expect(enCopy.includes(phrase)).toBe(true);
    }

    const zhCopy = `${zhBrowser.page.closeAll} ${zhBrowser.close.allTitle} ${zhBrowser.close.allContent} ${zhBrowser.close.allSuccess} ${zhBrowser.close.drainUnconfirmed}`;
    for (const phrase of [
      '浏览器资源',
      '浏览器通道',
      '待清理任务',
      '受管浏览器主机及进程',
      '权威剩余计数',
      '全部归零',
    ]) {
      expect(zhCopy.includes(phrase)).toBe(true);
    }
  });

  test('does not regress to hardcoded copy or malformed apostrophe text', () => {
    const sources = componentSources.join('\n');
    const hardcodedPhrases = [
      'Browser management',
      'Close a browser lane with active work?',
      'No active page',
      'Waiting for browser capacity',
      'Lane details',
      'You control this lane',
    ];

    for (const phrase of hardcodedPhrases) {
      expect(sources.includes(phrase)).toBe(false);
    }
    const malformedPunctuation = new RegExp(
      '\\u00e2\\u20ac\\u2122|[\\u2019\\u9225\\ufffd]',
      'u'
    );
    expect(
      malformedPunctuation.test(
        `${sources}\n${JSON.stringify(enBrowser)}\n${JSON.stringify(zhBrowser)}`
      )
    ).toBe(false);
  });
});
