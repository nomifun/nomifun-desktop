/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { readFileSync } from 'node:fs';
import { describe, expect, test } from 'bun:test';

const source = readFileSync(new URL('./browser.ts', import.meta.url), 'utf8');

describe('legacy bridge socket foreground recovery', () => {
  test('re-enables reconnection on foreground return after an auth-expired park', () => {
    // A 1008/auth-expired close parks the legacy socket (shouldReconnect =
    // false) until the login flow runs — but the session cookie may have been
    // renewed while the tab was hidden. The visibilitychange handler must
    // resume reconnecting off the login route; a genuinely dead session is
    // re-rejected at the next handshake.
    expect(source.includes("document.addEventListener('visibilitychange'")).toBe(true);
    expect(source.includes('shouldReconnect = true')).toBe(true);
  });
});
