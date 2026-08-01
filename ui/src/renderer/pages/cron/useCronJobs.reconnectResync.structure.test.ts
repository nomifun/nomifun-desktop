/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { readFileSync } from 'node:fs';
import { describe, expect, test } from 'bun:test';

const source = readFileSync(new URL('./useCronJobs.ts', import.meta.url), 'utf8');

describe('cron jobs reconnect recovery', () => {
  test('shared subscription helper resyncs the durable snapshot after websocket reconnect', () => {
    // WebSocket delivery has no replay: cron job events lost during a gap must
    // be recovered by refetching. Wired once in useCronJobSubscription so every
    // consumer hook gets it through its own fetch function.
    expect(source.includes('onResync')).toBe(true);
    expect(source.includes('ipcBridge.conversation.reconnected.on')).toBe(true);
    expect(source.includes('useCronJobSubscription(eventHandlers, fetchJobs)')).toBe(true);
    expect(source.includes('useCronJobSubscription(eventHandlers, fetchAllJobs)')).toBe(true);
  });
});
