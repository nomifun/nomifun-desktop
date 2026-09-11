/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { readFileSync } from 'node:fs';
import { describe, expect, test } from 'bun:test';
import type { AgentPresetSummary } from '@/common/types/agentPlatform';
import {
  DEFAULT_GUID_AGENT_SELECTION,
  getAgentPresetKey,
  isExecutableAgentPreset,
  normalizeGuidAgentSelection,
} from './agentSelectionUtils';

const readSource = (url: URL): string =>
  readFileSync(url, 'utf8').replace(/\r\n/g, '\n');

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

describe('Guid Agent selection contract', () => {
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

  test('normalizes legacy or invalid selections to the first official Agent', () => {
    expect(DEFAULT_GUID_AGENT_SELECTION).toEqual({
      kind: 'template',
      templateKey: 'chat.minimal',
    });
    expect(normalizeGuidAgentSelection({ kind: 'default' })).toEqual(
      DEFAULT_GUID_AGENT_SELECTION
    );
    expect(
      normalizeGuidAgentSelection({ kind: 'template', templateKey: 'removed.template' })
    ).toEqual(DEFAULT_GUID_AGENT_SELECTION);
    expect(
      normalizeGuidAgentSelection({ kind: 'template', templateKey: 'assistant.general' })
    ).toEqual({ kind: 'template', templateKey: 'assistant.general' });
  });

  test('loads saved user presets and filters draft-only rows', () => {
    const library = readSource(
      new URL('../../../hooks/agent/useAgentPresets.ts', import.meta.url)
    );
    const selection = readSource(new URL('./useGuidAgentSelection.ts', import.meta.url));

    expect(library.includes('presets: data?.user_presets ?? []')).toBe(true);
    expect(library.includes('presets: data?.official_templates')).toBe(false);
    expect(selection.includes('useAgentPresets()')).toBe(true);
    expect(/\.filter\(\s*isExecutableAgentPreset\s*\)/.test(selection)).toBe(true);
    expect(selection.includes('const isLoaded = library !== undefined;')).toBe(true);
    expect(selection.includes('!selectedAgentPresetId || isLoading || !isLoaded')).toBe(true);
  });

  test('exposes library errors while retaining cached rows and unresolved workbench preselection', () => {
    const library = readSource(
      new URL('../../../hooks/agent/useAgentPresets.ts', import.meta.url)
    );
    const selection = readSource(
      new URL('./useGuidAgentSelection.ts', import.meta.url)
    );
    const page = readSource(new URL('../GuidPage.tsx', import.meta.url));

    expect(library.includes('error: Error | undefined;')).toBe(true);
    expect(
      library.includes(
        'const { data, error, isLoading, mutate } = useSWR<'
      )
    ).toBe(true);
    expect(library.includes('presets: data?.user_presets ?? [],')).toBe(true);
    expect(library.includes('error,')).toBe(true);

    expect(selection.includes('loadError: Error | undefined;')).toBe(true);
    expect(selection.includes('error: loadError,')).toBe(true);
    expect(selection.includes('if (!preset && loadError) return;')).toBe(true);
    expect(selection.includes('loadError,')).toBe(true);

    expect(
      page.includes(
        'if (preselectedPresetId && agentSelection.loadError) return;'
      )
    ).toBe(true);
    expect(
      page.includes(
        'agentSelection.isLoading || !agentSelection.isLoaded'
      )
    ).toBe(true);
    expect(page.includes('agentSelection.loadError,')).toBe(true);
  });

  test('defaults to chat.minimal and persists only catalog-backed selections', () => {
    const selection = readSource(new URL('./useGuidAgentSelection.ts', import.meta.url));
    const selectionUtils = readSource(new URL('./agentSelectionUtils.ts', import.meta.url));
    const configKeys = readSource(
      new URL('../../../../common/config/configKeys.ts', import.meta.url)
    );

    expect(
      configKeys.includes(
        "| { kind: 'template'; templateKey: OfficialPresetKey }"
      )
    ).toBe(true);
    expect(
      selectionUtils.includes(
        "templateKey: 'chat.minimal',"
      )
    ).toBe(true);
    expect(configKeys.includes("| { kind: 'default' }")).toBe(false);
    expect(
      selection.includes("configService.get('guid.agentSelection')")
    ).toBe(true);
    expect(
      selection.includes(".set('guid.agentSelection', selection)")
    ).toBe(true);
    expect(
      selection.includes(
        "setSelection({ kind: 'preset', presetId: preset.preset_id });"
      )
    ).toBe(true);
    expect(selection.includes('presets[0]')).toBe(false);
  });

  test('keeps official-template and preset identities exact in the mention selector', () => {
    const mention = readSource(new URL('./useGuidMention.ts', import.meta.url));

    expect(mention.includes("kind: 'default'")).toBe(false);
    expect(mention.includes('guid-agent-default')).toBe(false);
    expect(mention.includes('guid-agent-template:${selection.templateKey}')).toBe(true);
    expect(
      mention.includes(
        "const presetSelection: GuidAgentSelection = {\n          kind: 'preset',\n          presetId: preset.preset_id,"
      )
    ).toBe(true);
  });
});
