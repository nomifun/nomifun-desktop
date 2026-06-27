/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { readFileSync } from 'node:fs';
import { describe, expect, test } from 'bun:test';

const readSource = (url: URL) => readFileSync(url, 'utf8');

describe('Guid resource cards placement', () => {
  test('renders resource cards directly below the Guid input card', () => {
    const source = readSource(new URL('../GuidPage.tsx', import.meta.url));

    const inputIndex = source.indexOf('<GuidInputCard');
    const resourceIndex = source.indexOf('<GuidResourceCards', inputIndex);
    const editorHostIndex = source.indexOf('<GuidAssistantEditorHost', inputIndex);

    expect(inputIndex).toBeGreaterThan(-1);
    expect(resourceIndex).toBeGreaterThan(inputIndex);
    expect(editorHostIndex).toBeGreaterThan(resourceIndex);
    expect(source.includes('onFillPrompt')).toBe(false);
  });

  test('contains docs, promo video, and contact feedback cards without recent prompt data access', () => {
    const source = readSource(new URL('./GuidResourceCards.tsx', import.meta.url));

    expect(source.includes('https://www.nomifun.com/docs')).toBe(true);
    expect(source.includes('https://youtu.be/gEDo5H0H0Pg')).toBe(true);
    expect(source.includes('https://www.nomifun.com/contact')).toBe(true);
    expect(source.includes('https://github.com/nomifun/nomifun-tauri/issues')).toBe(false);
    expect(source.includes('RECENT_PROMPT_LIMIT')).toBe(false);
    expect(source.includes('getConversationMessages')).toBe(false);
    expect(source.includes('useConversationHistoryContext')).toBe(false);
    expect(source.includes('useSWR')).toBe(false);
    expect(source.includes('onFillPrompt')).toBe(false);
  });
});
