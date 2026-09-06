/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { readFileSync } from 'node:fs';
import { describe, expect, test } from 'bun:test';

const readSource = (url: URL): string => readFileSync(url, 'utf8');

const extractSessionCreatePayload = (source: string): string => {
  const marker = 'agentPlatform.sessions.create.invoke({';
  const start = source.indexOf(marker);
  expect(start).toBeGreaterThan(-1);
  const payloadStart = start + marker.length;
  const payloadEnd = source.indexOf('});', payloadStart);
  expect(payloadEnd).toBeGreaterThan(payloadStart);
  return source.slice(payloadStart, payloadEnd);
};

describe('Guid AgentPreset-only launch wiring', () => {
  test('wires navigation, mentions, pills, and send through preset_id', () => {
    const page = readSource(new URL('./GuidPage.tsx', import.meta.url));

    expect(page.includes('selectedAgentPresetId?: string')).toBe(true);
    expect(/\w+\?\.selectedAgentPresetId/.test(page)).toBe(true);
    expect(
      /useGuidAgentSelection\(\{[\s\S]*?selectedAgentPresetId:\s*\w+/.test(page)
    ).toBe(true);
    expect(
      /<AgentPillBar[\s\S]*?presets=\{[^}]+\.presets\}[\s\S]*?selectedPresetId=\{[^}]+\.selectedPresetId\}[\s\S]*?onSelectPreset=\{[^}]+\}/.test(
        page
      )
    ).toBe(true);
    expect(/\.setSelectedPresetId\(\w+\)/.test(page)).toBe(true);
    expect(
      /useGuidSend\(\{[\s\S]*?selectedPreset:\s*\w+\.selectedPreset/.test(page)
    ).toBe(true);
    expect(
      /useGuidMention\(\{[\s\S]*?presets:\s*\w+\.presets[\s\S]*?selectedPresetId:\s*\w+\.selectedPresetId[\s\S]*?setSelectedPresetId:\s*\w+\.setSelectedPresetId/.test(
        page
      )
    ).toBe(true);

    expect(page.includes('selectedAgentKey')).toBe(false);
    expect(page.includes('availableAgents')).toBe(false);
    expect(page.includes('selectedAgentInfo')).toBe(false);
    expect(page.includes('findAgentByKey')).toBe(false);
    expect(page.includes('getEffectiveAgentType')).toBe(false);
  });

  test('does not expose preset-owned capability overrides', () => {
    const page = readSource(new URL('./GuidPage.tsx', import.meta.url));
    const actionRow = readSource(
      new URL('./components/GuidActionRow.tsx', import.meta.url)
    );
    const entryStrip = readSource(
      new URL('./components/ComposerEntryStrip.tsx', import.meta.url)
    );

    for (const forbidden of [
      'GuidModelSelector',
      'GuidSkillsDrawer',
      'KnowledgeControl',
      'GuidCollaboratorSelector',
      'CollaborationPolicyControl',
      'ensureBackendMcpCatalog',
      'modelSelectorNode',
      'collaboratorSelectorNode',
      'selectedMcpServerIds',
      'executionModelPool',
      'selectedCollaborationTemplate',
    ]) {
      expect(page.includes(forbidden)).toBe(false);
    }

    for (const forbidden of [
      'modelSelectorNode',
      'collaboratorSelectorNode',
      'mcpServers',
      'onToggleMcpServer',
    ]) {
      expect(actionRow.includes(forbidden)).toBe(false);
    }

    expect(entryStrip.includes('activeSkills')).toBe(false);
    expect(entryStrip.includes('onAdjustSkills')).toBe(false);
    expect(entryStrip.includes('collaborationPolicyNode')).toBe(false);
  });

  test('uses the loading state for the skeleton so an empty library still renders the workbench CTA', () => {
    const page = readSource(new URL('./GuidPage.tsx', import.meta.url));

    expect(
      /\{\w+\.isLoading\s*\?\s*\(\s*<AgentPillBarSkeleton/.test(page)
    ).toBe(true);
    expect(/\.presets\.length\s*===\s*0/.test(page)).toBe(false);
    expect(page.includes('availableAgents.length === 0')).toBe(false);
  });

  test('submits only preset_id and title to the high-level session API', () => {
    const send = readSource(new URL('./hooks/useGuidSend.ts', import.meta.url));
    const payload = extractSessionCreatePayload(send);
    const keys = [...payload.matchAll(/^\s+([a-z_][a-zA-Z0-9_]*)\s*:/gm)].map(
      (match) => match[1]
    );

    expect(keys).toEqual(['preset_id', 'title']);
    expect(payload.includes('selectedPreset.preset_id')).toBe(true);
    expect(payload.includes('entryPlan.conversationName')).toBe(true);

    for (const forbidden of [
      'agent_binding',
      'AgentBindingValue',
      'resolved_snapshot',
      'snapshot_digest',
      'revision_digest',
      'current_model',
      'model_id',
      'provider_id',
      'skill_bindings',
      'mcp',
    ]) {
      expect(send.includes(forbidden)).toBe(false);
    }
  });

  test('requires an executable selected preset for both send paths', () => {
    const send = readSource(new URL('./hooks/useGuidSend.ts', import.meta.url));
    const page = readSource(new URL('./GuidPage.tsx', import.meta.url));

    expect(send.match(/!selectedPreset\?\.current_stable_revision/g)).toHaveLength(
      2
    );
    expect(
      /Boolean\(\s*\w+\.selectedPreset\?\.current_stable_revision\s*\)/.test(
        page
      )
    ).toBe(true);
    expect(/!\w+\s*\|\|\s*autoWorkStartDisabled\(/.test(page)).toBe(true);
    expect(
      /isButtonDisabled=\{[\s\S]*?isAutoWorkMode\s*\?\s*autoWorkButtonDisabled\s*:\s*send\.isButtonDisabled/.test(
        page
      )
    ).toBe(true);
    const handlerStart = send.indexOf('const sendMessageHandler');
    const presetGuard = send.indexOf(
      'if (!selectedPreset?.current_stable_revision)',
      handlerStart
    );
    const loadingStart = send.indexOf('setLoading(true)', handlerStart);
    expect(presetGuard).toBeGreaterThan(handlerStart);
    expect(loadingStart).toBeGreaterThan(presetGuard);
  });
});
