/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { readFileSync } from 'node:fs';
import { describe, expect, test } from 'bun:test';

const readSource = (url: URL) => readFileSync(url, 'utf8');

describe('GuidPage advanced controls', () => {
  test('keeps only the supported session-specific draft controls', () => {
    const source = readSource(new URL('./GuidPage.tsx', import.meta.url));

    expect(source.includes('<AutoWorkControl')).toBe(true);
    expect(source.includes('<IdmmControl')).toBe(true);
    expect(source.includes('<SummonDrawer')).toBe(true);
    expect(source.includes('<KnowledgeControl')).toBe(false);
    expect(source.includes("from '@/renderer/pages/conversation/components/KnowledgeControl'")).toBe(
      false
    );
  });

  test('keeps the remaining draft API focused on session behavior', () => {
    const source = readSource(new URL('./hooks/useGuidAdvancedConfig.ts', import.meta.url));

    expect(source.includes('autoWork: AutoWorkDraftValue')).toBe(true);
    expect(source.includes('idmm: IIdmmConfig')).toBe(true);
    expect(source.includes('summon: SummonDraft | null')).toBe(true);
  });
});
