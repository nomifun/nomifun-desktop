/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { describe, expect, test } from 'bun:test';
import { readFileSync } from 'node:fs';

import enCreativeStudio from './locales/en-US/creativeStudio.json';
import zhCreativeStudio from './locales/zh-CN/creativeStudio.json';

type LocaleTree = { readonly [key: string]: string | LocaleTree };

const flattenKeys = (tree: LocaleTree, prefix = ''): string[] =>
  Object.entries(tree).flatMap(([key, value]) => {
    const path = prefix ? `${prefix}.${key}` : key;
    return typeof value === 'string' ? [path] : flattenKeys(value, path);
  });

const expectedKeys = [
  'focus.backToWorkbench',
  'navigation.assets',
  'navigation.canvases',
  'navigation.image',
  'navigation.label',
  'navigation.prompts',
  'navigation.video',
  'title',
];

const referencedCreativeStudioKeys = (): string[] => {
  const sources = [
    new URL('../../components/layout/Sider/SiderNav/SiderCreativeStudioEntry.tsx', import.meta.url),
    new URL('../../components/layout/Sider/index.tsx', import.meta.url),
    new URL('../../pages/creativeStudio/app/CreativeStudioSider.tsx', import.meta.url),
  ].map((url) => readFileSync(url, 'utf8'));
  const matches = sources.flatMap((source) =>
    [...source.matchAll(/t\(['"]creativeStudio\.([A-Za-z0-9_.-]+)['"]/g)].map(
      (match) => match[1]
    )
  );
  return [...new Set(matches)].sort();
};

describe('Creative Studio locale contract', () => {
  test('keeps Chinese and English on the same product navigation surface', () => {
    expect(flattenKeys(zhCreativeStudio).sort()).toEqual(expectedKeys);
    expect(flattenKeys(enCreativeStudio).sort()).toEqual(expectedKeys);
  });

  test('covers every currently referenced Creative Studio translation key', () => {
    expect(referencedCreativeStudioKeys()).toEqual(expectedKeys);
  });

  test('ships real navigation and workbench-return copy without the retired home pitch', () => {
    expect(zhCreativeStudio).toEqual({
      title: '创意工坊',
      focus: { backToWorkbench: '返回工作台' },
      navigation: {
        label: '创意工坊',
        canvases: '我的画布',
        image: '生图工作台',
        video: '视频创作台',
        prompts: '提示词库',
        assets: '我的素材',
      },
    });
    expect(enCreativeStudio).toEqual({
      title: 'Creative Studio',
      focus: { backToWorkbench: 'Back to workbench' },
      navigation: {
        label: 'Creative Studio',
        canvases: 'My canvases',
        image: 'Image studio',
        video: 'Video studio',
        prompts: 'Prompt library',
        assets: 'My assets',
      },
    });
    expect(flattenKeys(zhCreativeStudio).some((key) => key.startsWith('home.'))).toBe(false);
  });
});
