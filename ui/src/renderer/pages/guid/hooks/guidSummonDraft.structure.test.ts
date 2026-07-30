/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

/**
 * Structure tests for the Guid landing page's summon-companion draft entry
 * (「使用设定」左侧的「召唤伙伴」入口) — same source-assertion style as
 * `SummonPanel.structure.test.ts`.
 */

import { describe, expect, test } from 'bun:test';
import { readFileSync } from 'node:fs';

const readSource = (url: URL) => readFileSync(url, 'utf8');

describe('Guid summon draft integration', () => {
  test('advanced-config drafts carry a summon pick and apply it via the bridge', () => {
    const source = readSource(new URL('./useGuidAdvancedConfig.ts', import.meta.url));
    expect(source.includes('summon')).toBe(true);
    expect(source.includes('ipcBridge.conversation.setSummon.invoke')).toBe(true);
  });

  test('GuidPage wires the strip entry only for nomi-typed launches', () => {
    const page = readSource(new URL('../GuidPage.tsx', import.meta.url));
    expect(page.includes('onSummonCompanion')).toBe(true);
    expect(page.includes("effectiveAgentType === 'nomi'")).toBe(true);
  });

  test('the reusable summon drawer is shared with the in-session control', () => {
    const page = readSource(new URL('../GuidPage.tsx', import.meta.url));
    expect(page.includes('SummonDrawer')).toBe(true);
    const panel = readSource(
      new URL('../../conversation/components/SummonPanel/index.tsx', import.meta.url)
    );
    expect(panel.includes('export const SummonDrawer')).toBe(true);
  });
});
