/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { describe, expect, test } from 'bun:test';
import { readdirSync, readFileSync } from 'node:fs';
import { extname } from 'node:path';

import enCreativeStudio from './locales/en-US/creativeStudio.json';
import zhCreativeStudio from './locales/zh-CN/creativeStudio.json';

type LocaleTree = { readonly [key: string]: string | LocaleTree };

const flattenKeys = (tree: LocaleTree, prefix = ''): string[] =>
  Object.entries(tree).flatMap(([key, value]) => {
    const path = prefix ? `${prefix}.${key}` : key;
    return typeof value === 'string' ? [path] : flattenKeys(value, path);
  });

const flattenLeaves = (tree: LocaleTree): string[] =>
  Object.values(tree).flatMap((value) =>
    typeof value === 'string' ? [value] : flattenLeaves(value)
  );

const CORE_KEYS = [
  'focus.backToWorkbench',
  'navigation.assets',
  'navigation.canvases',
  'navigation.image',
  'navigation.label',
  'navigation.prompts',
  'navigation.templates',
  'navigation.video',
  'siderTitle',
  'title',
] as const;

const productionSourcesIn = (directory: URL): URL[] =>
  readdirSync(directory, { withFileTypes: true }).flatMap((entry) => {
    const entryUrl = new URL(`${entry.name}${entry.isDirectory() ? '/' : ''}`, directory);
    if (entry.isDirectory()) return productionSourcesIn(entryUrl);
    if (!['.ts', '.tsx'].includes(extname(entry.name))) return [];
    if (entry.name.includes('.test.') || entry.name.includes('.structure.')) return [];
    return [entryUrl];
  });

const referencedCreativeStudioKeys = (): string[] => {
  const sources = [
    ...productionSourcesIn(new URL('../../pages/creativeStudio/', import.meta.url)),
    ...[
      '../../components/layout/Sider/SiderNav/SiderCreativeStudioEntry.tsx',
      '../../components/layout/Sider/SiderNav/SiderAssetLibraryEntry.tsx',
      '../../components/layout/Sider/index.tsx',
    ].map((path) => new URL(path, import.meta.url)),
  ].map((url) => readFileSync(url, 'utf8'));
  const matches = sources.flatMap((source) =>
    [...source.matchAll(/['"]creativeStudio\.([A-Za-z0-9_.-]+)['"]/g)].map(
      (match) => match[1]
    )
  );
  return [...new Set(matches)].sort();
};

describe('Creative Studio locale contract', () => {
  test('keeps Chinese and English on the same complete product surface', () => {
    expect(flattenKeys(zhCreativeStudio).sort()).toEqual(
      flattenKeys(enCreativeStudio).sort()
    );
  });

  test('covers every currently referenced Creative Studio translation key', () => {
    const localeKeys = new Set(flattenKeys(enCreativeStudio));
    expect(
      referencedCreativeStudioKeys().filter((key) => !localeKeys.has(key))
    ).toEqual([]);
  });

  test('ships real navigation and workbench-return copy without the retired home pitch', () => {
    expect(zhCreativeStudio.title).toBe('创意工坊');
    expect(zhCreativeStudio.focus.backToWorkbench).toBe('返回工作台');
    expect(enCreativeStudio.title).toBe('Creative Studio');
    expect(enCreativeStudio.focus.backToWorkbench).toBe('Back to workbench');
    const localeKeys = new Set(flattenKeys(enCreativeStudio));
    expect(CORE_KEYS.filter((key) => !localeKeys.has(key))).toEqual([]);
    expect(flattenKeys(zhCreativeStudio).some((key) => key.startsWith('home.'))).toBe(false);
  });

  test('keeps the retired workflow term out of user-facing Creative Studio copy', () => {
    for (const locale of [enCreativeStudio, zhCreativeStudio]) {
      const values = Object.values(flattenLeaves(locale));
      expect(values.some((value) => /workflow|工作流/i.test(value))).toBe(false);
    }
  });
});
