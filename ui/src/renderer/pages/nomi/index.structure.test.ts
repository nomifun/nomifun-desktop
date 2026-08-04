/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { describe, expect, test } from 'bun:test';
import { readFileSync } from 'node:fs';

describe('Nomi companion tab order', () => {
  test('places remote connection immediately after overview', () => {
    const source = readFileSync(new URL('./index.tsx', import.meta.url), 'utf8');
    const registry = source.match(/const COMPANION_TABS = \[(.*?)\] as const;/s)?.[1] ?? '';

    expect(registry.indexOf("'overview'")).toBeLessThan(registry.indexOf("'remote'"));
    expect(registry.indexOf("'remote'")).toBeLessThan(registry.indexOf("'memories'"));

    const overviewRadio = source.indexOf("<Radio value='overview'>");
    const remoteRadio = source.indexOf("<Radio value='remote'>");
    const memoriesRadio = source.indexOf("<Radio value='memories'>");

    expect(overviewRadio).toBeGreaterThan(-1);
    expect(remoteRadio).toBeGreaterThan(overviewRadio);
    expect(memoriesRadio).toBeGreaterThan(remoteRadio);
  });

  test('does not expose retired credentials or overview diary UI', () => {
    const source = readFileSync(new URL('./index.tsx', import.meta.url), 'utf8');
    const overview = readFileSync(new URL('./tabs/OverviewTab.tsx', import.meta.url), 'utf8');

    expect(source.includes("'secrets'")).toBe(false);
    expect(source.includes('SecretsTab')).toBe(false);
    expect(overview.includes('listLearnRuns')).toBe(false);
    expect(overview.includes('nomi.overview.diary')).toBe(false);
  });

  test('removes learning history UI while preserving live learning and skill evolution', () => {
    const learn = readFileSync(new URL('./tabs/LearnTab.tsx', import.meta.url), 'utf8');
    const bridge = readFileSync(new URL('../../../common/adapter/ipcBridge.ts', import.meta.url), 'utf8');

    expect(learn.includes('listLearnRuns')).toBe(false);
    expect(learn.includes('ICompanionLearnRun')).toBe(false);
    expect(learn.includes('<Table')).toBe(false);
    expect(bridge.includes('/api/companion/learn/runs')).toBe(false);
    expect(bridge.includes('listLearnRuns')).toBe(false);

    expect(learn.includes('ipcBridge.companion.runLearn.invoke()')).toBe(true);
    expect(learn.includes('sharedConfig.evolve.enabled')).toBe(true);
    expect(bridge.includes('onLearnFinished')).toBe(true);
    expect(bridge.includes('ICompanionLearnResult')).toBe(true);
  });

  test('replaces raw event viewing and manual clearing with an explicit retention policy', () => {
    const collect = readFileSync(new URL('./tabs/CollectTab.tsx', import.meta.url), 'utf8');
    const bridge = readFileSync(new URL('../../../common/adapter/ipcBridge.ts', import.meta.url), 'utf8');

    expect(collect.includes('rawEvents')).toBe(false);
    expect(collect.includes('loadRawEvents')).toBe(false);
    expect(bridge.includes('/api/companion/events/recent')).toBe(false);
    expect(bridge.includes("httpDelete<void, void>('/api/companion/events')")).toBe(false);

    expect(bridge.includes("eventStorage: httpGet<ICompanionEventStorageStatus, void>('/api/companion/events/storage')"))
      .toBe(true);
    expect(collect.includes('event_retention_days: retentionDraft')).toBe(true);
    expect(collect.includes('event_max_storage_mb: capacityDraft')).toBe(true);
    expect(collect.includes('lowerPolicyConfirm')).toBe(true);
    expect(collect.includes('onOk={applyStoragePolicy}')).toBe(true);
    expect(collect.includes('min={7}')).toBe(true);
    expect(collect.includes('max={365}')).toBe(true);
    expect(collect.includes('min={16}')).toBe(true);
    expect(collect.includes('max={512}')).toBe(true);

    // Unknown/failed status must not be reported as an empty zero-byte store.
    expect(collect.includes('storage?.total_bytes ?? 0')).toBe(false);
    expect(collect.includes('storageError && !storageLoading ?')).toBe(true);
    expect(collect.includes('refreshStorage(true)')).toBe(true);
    expect(collect.includes('storageUnavailable')).toBe(true);
    expect(collect.includes('storageLoading')).toBe(true);

    const enNomi = JSON.parse(
      readFileSync(new URL('../../services/i18n/locales/en-US/nomi.json', import.meta.url), 'utf8')
    );
    const zhNomi = JSON.parse(
      readFileSync(new URL('../../services/i18n/locales/zh-CN/nomi.json', import.meta.url), 'utf8')
    );
    expect(enNomi.collect.storedRange_one.includes('{{count}} daily file)')).toBe(true);
    expect(enNomi.collect.storedRange_other.includes('{{count}} daily files)')).toBe(true);
    expect(Object.keys(enNomi.collect).sort()).toEqual(Object.keys(zhNomi.collect).sort());
  });
});
