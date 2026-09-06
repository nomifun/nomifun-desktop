/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { readFileSync } from 'node:fs';
import { describe, expect, test } from 'bun:test';
import type { AgentPresetSummary } from '@/common/types/agentPlatform';
import {
  getAgentPresetKey,
  isExecutableAgentPreset,
} from './agentSelectionUtils';

const readSource = (url: URL): string => readFileSync(url, 'utf8');

const stablePreset = {
  preset_id: '0190f5fe-7c00-7a00-8000-000000000101',
  source: 'user',
  display_name: 'Release reviewer',
  bound_target_count: 0,
  current_stable_revision: {
    preset_id: '0190f5fe-7c00-7a00-8000-000000000101',
    revision: 3,
    revision_digest: 'a'.repeat(64),
  },
} as AgentPresetSummary;

describe('Guid AgentPreset selection contract', () => {
  test('only treats user presets with a stable revision as executable', () => {
    const draftOnly = {
      ...stablePreset,
      preset_id: '0190f5fe-7c00-7a00-8000-000000000102',
      current_stable_revision: undefined,
    } as AgentPresetSummary;

    expect(isExecutableAgentPreset(stablePreset)).toBe(true);
    expect(isExecutableAgentPreset(draftOnly)).toBe(false);
  });

  test('uses preset_id as the selection key', () => {
    expect(getAgentPresetKey(stablePreset)).toBe(stablePreset.preset_id);
  });

  test('loads only the user preset library and then filters draft-only rows', () => {
    const library = readSource(
      new URL('../../../hooks/agent/useAgentPresets.ts', import.meta.url)
    );
    const selection = readSource(new URL('./useGuidAgentSelection.ts', import.meta.url));

    expect(library.includes('presets: data?.user_presets ?? []')).toBe(true);
    expect(library.includes('presets: data?.official_templates')).toBe(false);
    expect(selection.includes('useAgentPresets()')).toBe(true);
    expect(selection.includes('savedPresets.filter(isExecutableAgentPreset)')).toBe(true);
  });

  test('persists only preset_id through the AgentPreset preference API', () => {
    const selection = readSource(new URL('./useGuidAgentSelection.ts', import.meta.url));
    const configKeys = readSource(
      new URL('../../../../common/config/configKeys.ts', import.meta.url)
    );

    expect(selection.includes('selectedAgentPresetId')).toBe(true);
    expect(selection.includes('preset.preset_id === selectedPresetId')).toBe(true);
    expect(selection.includes('modelList')).toBe(false);
    expect(selection.includes('localeKey')).toBe(false);
    expect(selection.includes('selectedAgentKey')).toBe(false);
    expect(selection.includes('availableAgents')).toBe(false);
    expect(selection.includes("configService.get('guid.lastSelectedAgentPreset')")).toBe(
      true
    );
    expect(selection.includes(".set('guid.lastSelectedAgentPreset'")).toBe(true);
    expect(selection.includes('useAgents')).toBe(false);
    expect(configKeys.includes("'guid.lastSelectedAgentPreset'")).toBe(true);
    const legacyPreferenceKey = ['guid.lastSelected', 'Agent'].join('');
    expect(configKeys.includes(`'${legacyPreferenceKey}'`)).toBe(false);
  });

  test('keeps the pill and mention surfaces preset-native', () => {
    const pillBar = readSource(
      new URL('../components/AgentPillBar.tsx', import.meta.url)
    );
    const mention = readSource(new URL('./useGuidMention.ts', import.meta.url));

    expect(pillBar.includes('{preset.display_name}')).toBe(true);
    expect(pillBar.includes('<Robot')).toBe(true);
    expect(pillBar.includes("navigate('/agent')")).toBe(true);
    expect(pillBar.includes('onSelectPreset(presetId)')).toBe(true);
    expect(pillBar.includes('selectedPresetId === presetId')).toBe(true);
    expect(pillBar.includes('resolveAgentLogo')).toBe(false);
    expect(pillBar.includes('AgentMetadata')).toBe(false);
    expect(mention.includes('preset.display_name')).toBe(true);
    expect(mention.includes('preset.preset_id')).toBe(true);
    expect(mention.includes('resolveAgentLogo')).toBe(false);
  });
});
