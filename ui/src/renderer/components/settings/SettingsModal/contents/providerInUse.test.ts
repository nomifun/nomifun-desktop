/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { describe, expect, test } from 'bun:test';
import { featureRoute, groupUsagesByFeature, parseProviderInUseDetails, type ProviderUsage } from './providerInUse';

describe('providerInUse helpers', () => {
  test('featureRoute maps each feature', () => {
    expect(featureRoute('desktopCompanion')).toBe('/nomi');
    expect(featureRoute('publicCompanion', 'pa_1')).toBe('/public-companions/pa_1');
    expect(featureRoute('publicCompanion')).toBe('/public-companions');
    expect(featureRoute('smartDecision')).toBe('/models?section=global');
    expect(featureRoute('conversation', '42')).toBe('/conversation/42');
    expect(featureRoute('conversation')).toBe('/guid');
    expect(featureRoute('agentExecution')).toBe('/guid');
  });

  test('groupUsagesByFeature groups labels', () => {
    const usages: ProviderUsage[] = [
      { feature: 'desktopCompanion', label: '甲', targetId: 'c1' },
      { feature: 'desktopCompanion', label: '乙', targetId: 'c2' },
      { feature: 'conversation', label: '主会话', targetId: '42' },
      { feature: 'agentExecution', label: '协作任务', targetId: 'exec-1' },
    ];
    const groups = groupUsagesByFeature(usages);
    expect(groups.find((g) => g.feature === 'desktopCompanion')?.labels).toEqual(['甲', '乙']);
    expect(groups.find((g) => g.feature === 'conversation')?.targetId).toBe('42');
    expect(groups.find((g) => g.feature === 'agentExecution')?.targetId).toBe('exec-1');
  });

  test('parseProviderInUseDetails extracts usages and tolerates junk', () => {
    expect(
      parseProviderInUseDetails({ usages: [{ feature: 'agentExecution', label: '协作任务', targetId: 'exec-1' }] })
    ).toHaveLength(1);
    expect(parseProviderInUseDetails(undefined)).toEqual([]);
    expect(parseProviderInUseDetails({ nope: 1 })).toEqual([]);
    expect(parseProviderInUseDetails('string')).toEqual([]);
  });
});
