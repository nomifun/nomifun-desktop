/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { readFileSync } from 'node:fs';
import { describe, expect, test } from 'bun:test';

const readSource = (url: URL) => readFileSync(url, 'utf8');

describe('ACP initial message ownership', () => {
  test('starts the initial turn from AcpChat after the ACP turn-state hook is installed', () => {
    const chatSource = readSource(new URL('./AcpChat.tsx', import.meta.url));
    const useAcpMessageIndex = chatSource.indexOf('useAcpMessage(');
    const useAcpInitialMessageIndex = chatSource.indexOf('useAcpInitialMessage(');

    expect(useAcpMessageIndex).toBeGreaterThan(-1);
    expect(useAcpInitialMessageIndex).toBeGreaterThan(useAcpMessageIndex);
  });

  test('keeps initial-message ownership out of the sendbox UI component', () => {
    const sendBoxSource = readSource(new URL('./AcpSendBox.tsx', import.meta.url));

    expect(sendBoxSource.includes('useAcpInitialMessage')).toBe(false);
  });
});
