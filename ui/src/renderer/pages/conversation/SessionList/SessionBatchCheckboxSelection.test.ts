/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { describe, expect, test } from 'bun:test';
import { readFileSync } from 'node:fs';

const conversationRowSource = readFileSync(new URL('./ConversationRow.tsx', import.meta.url), 'utf8');
const terminalRowSource = readFileSync(new URL('./TerminalRow.tsx', import.meta.url), 'utf8');
const sessionKindSource = readFileSync(new URL('./SessionKindGroup.tsx', import.meta.url), 'utf8');
const workpathSource = readFileSync(new URL('./WorkpathDrawer.tsx', import.meta.url), 'utf8');
const controlCss = readFileSync(new URL('../../../styles/theme-control-contract.css', import.meta.url), 'utf8');

describe('session-list batch checkbox selection treatment', () => {
  test('uses the themed checkbox treatment for every batch-selection level', () => {
    for (const source of [conversationRowSource, terminalRowSource, sessionKindSource, workpathSource]) {
      expect(source.includes("className='session-batch-selection-checkbox'")).toBe(true);
    }
    expect(controlCss.includes('.arco-checkbox-mask {')).toBe(true);
    expect(controlCss.includes('.arco-checkbox-checked .arco-checkbox-mask')).toBe(true);
    expect(controlCss.includes('.arco-checkbox-mask-icon')).toBe(true);
  });
});
