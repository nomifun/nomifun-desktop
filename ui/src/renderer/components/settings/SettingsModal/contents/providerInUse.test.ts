/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { describe, expect, test } from 'bun:test';
import { featureRoute, groupUsagesByFeature, type ProviderUsage } from './providerInUse';

describe('providerInUse helpers', () => {
  test('featureRoute maps each feature', () => {
    expect(featureRoute('desktopCompanion')).toBe('/companion');
    expect(featureRoute('publicCompanion', 'pa_1')).toBe('/public-companions/pa_1');
    expect(featureRoute('publicCompanion')).toBe('/public-companions');
    expect(featureRoute('smartDecision')).toBe('/nomi');
    expect(featureRoute('orchestrator')).toBe('/guid');
  });

  test('groupUsagesByFeature groups labels', () => {
    const usages: ProviderUsage[] = [
      { feature: 'desktopCompanion', label: '甲', targetId: 'c1' },
      { feature: 'desktopCompanion', label: '乙', targetId: 'c2' },
      { feature: 'orchestrator', label: '舰队', targetId: 'f1' },
    ];
    const groups = groupUsagesByFeature(usages);
    expect(groups.find((g) => g.feature === 'desktopCompanion')?.labels).toEqual(['甲', '乙']);
    expect(groups.find((g) => g.feature === 'orchestrator')?.targetId).toBe('f1');
  });
});
