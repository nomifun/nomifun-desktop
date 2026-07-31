/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { readFileSync } from 'node:fs';
import { describe, expect, test } from 'bun:test';

const source = readFileSync(new URL('./ModelModalContent.tsx', import.meta.url), 'utf8');

describe('model health probe persistence', () => {
  test('heartbeat no longer PUTs the legacy model_health map back after probing', () => {
    // The server-side probe (`checkProviderHealth`) persists its result into
    // the provider_models row; the old fetch-latest-then-merge whole-map
    // `updateProvider({ model_health })` write after each heartbeat is gone.
    expect(source.includes('checkProviderHealth')).toBe(true);
    expect(source.includes('model_health[modelName]')).toBe(false);
    expect(source.includes('const latestData = await ipcBridge.mode.listProviders.invoke()')).toBe(false);
    expect(/updateProvider\.invoke\(\{ provider_id: platform\.id, model_health \}\)/.test(source)).toBe(false);
  });

  test('row health is the only per-model health source in the provider list', () => {
    // Rendered health comes from the authoritative row (`row.health`);
    // modelRowsFor backfills legacy-map health only for row-less providers.
    expect(source.includes('const model_health = row.health;')).toBe(true);
    expect(source.includes('row.health ?? platform.model_health')).toBe(false);
    // The "清除状态" bulk-clear button is gone: the server ignores client
    // model_health writes (P3 T1), so its `updateProvider({ model_health: {} })`
    // PUT became a no-op and the whole handler was removed.
    expect(source.includes('clearAllHealthData')).toBe(false);
    expect(source.includes('model_health: {} }')).toBe(false);
  });
});
