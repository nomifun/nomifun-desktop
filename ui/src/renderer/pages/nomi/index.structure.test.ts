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
});
