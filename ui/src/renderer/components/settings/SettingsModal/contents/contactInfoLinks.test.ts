/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { readFileSync } from 'node:fs';
import { describe, expect, test } from 'bun:test';

const readSource = (url: URL) => readFileSync(url, 'utf8');

describe('public contact links', () => {
  test('keeps About and Contact surfaces wired to current public channels', () => {
    const aboutSource = readSource(new URL('./AboutModalContent.tsx', import.meta.url));
    const contactSource = readSource(new URL('./FeedbackReportModal.tsx', import.meta.url));
    const combined = `${aboutSource}\n${contactSource}`;

    for (const target of [
      'https://www.nomifun.com',
      'https://www.nomifun.com/contact',
      'https://github.com/nomifun/nomifun-tauri/issues',
      'https://github.com/nomifun/nomifun-tauri/releases',
    ]) {
      expect(combined.includes(target)).toBe(true);
    }

    expect(combined.includes('mailto:')).toBe(false);
    expect(aboutSource.includes('NOMIFUN_PUBLIC_LINKS.email')).toBe(false);
    expect(aboutSource.includes('ABOUT_LINK_TARGET')).toBe(false);
  });

  test('keeps the Baidu manual installer link visible beside update checks', () => {
    const aboutSource = readSource(new URL('./AboutModalContent.tsx', import.meta.url));
    const contactSource = readSource(new URL('./FeedbackReportModal.tsx', import.meta.url));
    const updateModalSource = readSource(new URL('../../UpdateModal.tsx', import.meta.url));

    expect(contactSource.includes("baiduPan: 'https://pan.baidu.com/s/5GPonoJNrwJ7GciBSDgXLaA'")).toBe(true);
    expect(aboutSource.includes('NOMIFUN_PUBLIC_LINKS.baiduPan')).toBe(true);
    expect(aboutSource.includes('settings.baiduManualDownload')).toBe(true);
    expect(updateModalSource.includes('settings.baiduManualDownload')).toBe(true);
  });

  test('keeps the Contact modal visually quiet instead of rendering chunky cards', () => {
    const contactSource = readSource(new URL('./FeedbackReportModal.tsx', import.meta.url));

    expect(contactSource.includes("import CopyIconButton from '@/renderer/components/base/CopyIconButton'")).toBe(true);
    expect(contactSource.includes("<Info theme='outline' size='28' />")).toBe(false);
    expect(contactSource.includes("bg-fill-2 px-12px py-10px")).toBe(false);
    expect(contactSource.includes('>↗<')).toBe(false);
  });

  test('keeps the About page links vertically compact', () => {
    const aboutSource = readSource(new URL('./AboutModalContent.tsx', import.meta.url));

    expect(aboutSource.includes("items-center pb-12px")).toBe(true);
    expect(aboutSource.includes("<Divider className='my-8px' />")).toBe(true);
    expect(aboutSource.includes("flex flex-col gap-0")).toBe(true);
    expect(aboutSource.includes("px-12px py-8px rd-8px")).toBe(true);
    expect(aboutSource.includes("px-16px py-12px rd-8px")).toBe(false);
  });
});
