/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { readFileSync } from 'node:fs';
import { describe, expect, test } from 'bun:test';

const source = readFileSync(
  new URL('./useConversationShortcuts.ts', import.meta.url),
  'utf8'
).replace(/\r\n/g, '\n');

describe('conversation keyboard shortcuts', () => {
  test('Ctrl/Cmd+T opens a new default Nomi conversation', () => {
    expect(
      source.includes(
        "return (event.metaKey || event.ctrlKey) && !event.altKey && !event.shiftKey && event.key.toLowerCase() === 't';"
      )
    ).toBe(true);
    expect(
      source.includes(
        "void navigate('/guid', { state: { resetAgentSelection: true } });"
      )
    ).toBe(true);
    expect(source.includes("void navigate('/guid');")).toBe(false);
  });
});
