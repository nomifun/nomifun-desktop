/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { readFileSync } from 'node:fs';
import { describe, expect, test } from 'bun:test';
import { completionPercent, parseSeeds } from './CrawlPage';
import type { ICrawlJob } from '@/common/adapter/ipcBridge';

const pageSource = readFileSync(new URL('./CrawlPage/index.tsx', import.meta.url), 'utf8');

const job = (progress: ICrawlJob['progress']): ICrawlJob =>
  ({ progress }) as ICrawlJob;

describe('completionPercent', () => {
  test('is zero for an empty queue rather than NaN', () => {
    expect(
      completionPercent(job({ pending: 0, in_progress: 0, done: 0, failed: 0, skipped: 0 }))
    ).toBe(0);
  });

  test('counts failed and skipped as settled, not as outstanding work', () => {
    expect(
      completionPercent(job({ pending: 0, in_progress: 0, done: 1, failed: 1, skipped: 2 }))
    ).toBe(100);
  });

  test('excludes in-flight tasks from the settled share', () => {
    expect(
      completionPercent(job({ pending: 1, in_progress: 1, done: 2, failed: 0, skipped: 0 }))
    ).toBe(50);
  });
});

describe('parseSeeds', () => {
  test('splits lines and trims surrounding space', () => {
    expect(parseSeeds('  https://a.com \n https://b.com  ')).toEqual([
      'https://a.com',
      'https://b.com',
    ]);
  });

  test('drops blank lines instead of submitting empty seeds', () => {
    expect(parseSeeds('https://a.com\n\n   \nhttps://b.com\n')).toEqual([
      'https://a.com',
      'https://b.com',
    ]);
  });

  test('returns an empty list for whitespace-only input', () => {
    expect(parseSeeds('   \n  \n')).toEqual([]);
  });
});

describe('crawl page structure', () => {
  /// The id is never shown anywhere in the UI, so a free-text field made the
  /// sink unfillable without opening DevTools.
  test('picks the sink knowledge base from the catalog, not a typed id', () => {
    expect(pageSource.includes('useKnowledgeBaseOptions')).toBe(true);
    expect(pageSource.includes("field='knowledge_base_id'")).toBe(true);
    expect(/field='knowledge_base_id'[\s\S]{0,200}<Input/.test(pageSource)).toBe(false);
  });
});
