/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { readFileSync } from 'node:fs';
import { describe, expect, test } from 'bun:test';

const source = readFileSync(new URL('./useRequirements.ts', import.meta.url), 'utf8');

describe('requirements reconnect recovery', () => {
  test('both hooks reload their durable snapshot after websocket reconnect', () => {
    // WebSocket delivery has no replay: requirements events lost during a gap
    // must be recovered by refetching. One subscription per hook (list + tags).
    const matches = source.match(/ipcBridge\.conversation\.reconnected\.on\(\(\) => void refresh\(\)\)/g) ?? [];
    expect(matches.length).toBe(2);
  });
});
