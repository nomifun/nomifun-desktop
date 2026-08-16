/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { describe, expect, test } from 'bun:test';
import type { ICronJob } from '@/common/adapter/ipcBridge';
import { parsePresetReference } from '@/common/types/agent/presetTypes';
import { parseAgentId } from '@/common/types/ids';
import type { AgentMetadata } from '@/renderer/utils/model/agentTypes';
import {
  findCronSelectedAgent,
  getCronAgentOptionValue,
  getCronAgentSelectionFromJob,
  hasCronAgentConfigurationChanged,
  resolveCronAgentDisplayName,
} from './cronAgentSelection';

const firstId = parseAgentId('0190f5fe-7c00-7a00-8000-000000000021');
const secondId = parseAgentId('0190f5fe-7c00-7a00-8000-000000000022');

const agent = (agent_id: typeof firstId, name: string, backend?: string): AgentMetadata => ({
  agent_id,
  name,
  backend,
  agent_type: 'nomi',
  agent_source: 'custom',
  enabled: true,
  available: true,
});

const first = agent(firstId, 'Reviewer A');
const second = agent(secondId, 'Reviewer B');

// `ICronJob.metadata.agent_type` is a raw persisted string; historical rows
// still carry the retired 'acp' discriminant, which is exactly what the legacy
// fallback below has to keep working against.
const job = (config: ICronJob['metadata']['agent_config'], agent_type = 'acp'): ICronJob =>
  ({ metadata: { agent_type, agent_config: config } }) as ICronJob;

describe('scheduled task Agent selection identity', () => {
  test('keeps custom Agents distinct even when both lack a backend', () => {
    expect(getCronAgentOptionValue(firstId)).not.toBe(getCronAgentOptionValue(secondId));
    expect(findCronSelectedAgent(getCronAgentOptionValue(firstId), [first, second])).toBe(first);
    expect(findCronSelectedAgent(getCronAgentOptionValue(secondId), [first, second])).toBe(second);
  });

  test('restores the persisted custom Agent ID before any backend fallback', () => {
    const configured = job({ name: 'Reviewer A', custom_agent_id: firstId });
    expect(getCronAgentSelectionFromJob(configured, [first, second])).toBe(getCronAgentOptionValue(firstId));
  });

  test('uses a legacy backend when unique and preserves a frozen placeholder when ambiguous', () => {
    const backendRow = agent(firstId, 'Nomi', 'nomi');
    expect(getCronAgentSelectionFromJob(job({ name: 'Nomi', backend: 'nomi' }), [backendRow])).toBe(
      getCronAgentOptionValue(firstId)
    );
    // Historical rows persisted a retired engine discriminant. The frozen
    // legacy placeholder must survive so an unrelated edit does not rewrite it.
    expect(getCronAgentSelectionFromJob(job({ name: 'Old engine' }), [first, second])).toBe('legacy:acp');
    expect(
      hasCronAgentConfigurationChanged(job({ name: 'Old engine' }), [first, second], {
        selection: 'legacy:acp',
        clearContextEachRun: false,
      })
    ).toBe(false);
  });

  test('localizes the visible name but never substitutes the ID or icon for a configured name', () => {
    const localized = { ...first, name_i18n: { 'zh-CN': '审查员', 'en-US': 'Reviewer' }, icon: '🤖' };
    expect(resolveCronAgentDisplayName(localized, 'zh-Hans')).toBe('审查员');
    expect(resolveCronAgentDisplayName(localized, 'en-US')).toBe('Reviewer');
  });

  test('does not refresh a frozen Agent configuration during an unrelated task edit', () => {
    const configured = job({
      name: 'Reviewer A',
      custom_agent_id: firstId,
      model: 'review-model',
      workspace: '/project',
      clear_context_each_run: true,
    });
    const unchanged = {
      selection: getCronAgentOptionValue(firstId),
      model: 'review-model',
      workspace: '/project',
      clearContextEachRun: true,
    };
    expect(hasCronAgentConfigurationChanged(configured, [first], unchanged)).toBe(false);
    expect(hasCronAgentConfigurationChanged(configured, [first], { ...unchanged, workspace: '/other' })).toBe(true);
  });

  test('keeps a frozen preset unchanged even when its live catalog entry is unavailable', () => {
    const presetId = parsePresetReference('0190f5fe-7c00-7a00-8000-000000000023');
    const configured = job({
      name: 'Frozen reviewer preset',
      preset_id: presetId,
      model: 'frozen-model',
      workspace: '/project',
    });
    expect(
      hasCronAgentConfigurationChanged(configured, [], {
        selection: `preset:${presetId}`,
        model: 'frozen-model',
        workspace: '/project',
        clearContextEachRun: false,
      })
    ).toBe(false);
  });
});
