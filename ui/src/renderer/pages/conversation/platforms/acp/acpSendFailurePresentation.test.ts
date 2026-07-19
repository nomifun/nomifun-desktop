/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { describe, expect, test } from 'bun:test';
import { readFileSync } from 'node:fs';
import { fileURLToPath } from 'node:url';

const readSibling = (name: string): string =>
  readFileSync(fileURLToPath(new URL(name, import.meta.url)), 'utf8');

describe('ACP send failure presentation ownership', () => {
  test('does not manufacture transcript error rows in the send box', () => {
    const source = readSibling('./AcpSendBox.tsx');

    expect(source.includes("type: 'tips'")).toBe(false);
    expect(source.includes('Message.error')).toBe(true);
  });

  test('does not manufacture transcript error rows for the initial send', () => {
    const source = readSibling('./useAcpInitialMessage.ts');

    expect(source.includes("type: 'tips'")).toBe(false);
    expect(source.includes('Message.error')).toBe(true);
  });
});
