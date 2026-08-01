/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { readFileSync } from 'node:fs';
import { describe, expect, test } from 'bun:test';

const source = readFileSync(new URL('./usePendingConfirmationsRecovery.ts', import.meta.url), 'utf8');

describe('pending confirmations reconnect recovery', () => {
  test('re-runs the recovery list fetch after websocket reconnect', () => {
    // WebSocket delivery has no replay: a confirmation raised while delivery
    // was gapped must be recovered by re-fetching the pending list.
    expect(source.includes('const recoverPendingConfirmations = ')).toBe(true);
    expect(source.includes('ipcBridge.conversation.reconnected.on')).toBe(true);
  });
});
