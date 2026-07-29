/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { readFileSync } from 'node:fs';
import { describe, expect, test } from 'bun:test';
import enBrowser from '../../services/i18n/locales/en-US/browser.json';
import zhBrowser from '../../services/i18n/locales/zh-CN/browser.json';

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
  new URL('./BrowserDisplayModeControl.tsx', import.meta.url),
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

    expect(Object.keys(zh).sort()).toEqual(Object.keys(en).sort());
    for (const key of Object.keys(en)) {
      expect(typeof en[key]).toBe('string');
      expect((en[key] as string).trim()).toBeTruthy();
      expect(typeof zh[key]).toBe('string');
      expect((zh[key] as string).trim()).toBeTruthy();
      expect(placeholders(zh[key] as string)).toEqual(placeholders(en[key] as string));
    }
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
