/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { readFileSync } from 'node:fs';
import { describe, expect, test } from 'bun:test';

const readSource = (url: URL): string =>
  readFileSync(url, 'utf8').replace(/\r\n/g, '\n');

const compactWhitespace = (source: string): string =>
  source.replace(/\s+/g, ' ').trim();

const extractObjectArgument = (source: string, call: string): string => {
  const callStart = source.indexOf(call);
  expect(callStart).toBeGreaterThan(-1);
  const objectStart = source.indexOf('{', callStart + call.length);
  expect(objectStart).toBeGreaterThan(callStart);

  let depth = 0;
  for (let index = objectStart; index < source.length; index += 1) {
    if (source[index] === '{') depth += 1;
    if (source[index] !== '}') continue;
    depth -= 1;
    if (depth === 0) return source.slice(objectStart + 1, index);
  }

  throw new Error(`Unclosed object argument for ${call}`);
};

const topLevelKeys = (objectBody: string): string[] => {
  const keys: string[] = [];
  let depth = 0;

  for (const line of objectBody.split(/\r?\n/)) {
    const trimmed = line.trim();
    if (depth === 0) {
      const match = trimmed.match(/^([a-z_][a-zA-Z0-9_]*)\s*:/);
      if (match) keys.push(match[1]);
    }
    depth += [...line].filter((character) => character === '{').length;
    depth -= [...line].filter((character) => character === '}').length;
  }

  return keys;
};

describe('Guid workbench Agent launch behavior', () => {
  test('uses only official-template or personal-preset selection identities', () => {
    const configKeys = readSource(
      new URL('../../../common/config/configKeys.ts', import.meta.url)
    );
    const types = readSource(new URL('./types.ts', import.meta.url));
    const selection = readSource(
      new URL('./hooks/useGuidAgentSelection.ts', import.meta.url)
    );
    const selectionUtils = readSource(
      new URL('./hooks/agentSelectionUtils.ts', import.meta.url)
    );
    const workbenchController = readSource(
      new URL('../agentSettings/useAgentSettingsController.ts', import.meta.url)
    );

    expect(configKeys.includes("| { kind: 'default' }")).toBe(false);
    expect(
      configKeys.includes("| { kind: 'preset'; presetId: AgentPresetId };")
    ).toBe(true);
    expect(
      configKeys.includes(
        "'guid.agentSelection': GuidAgentSelectionPreference | undefined;"
      )
    ).toBe(true);
    expect(
      types.includes(
        'export type GuidAgentSelection = GuidAgentSelectionPreference;'
      )
    ).toBe(true);
    expect(
      selectionUtils.includes(
        "templateKey: 'chat.minimal',"
      )
    ).toBe(true);
    expect(selectionUtils.includes("kind: 'template',")).toBe(true);
    expect(selection.includes('presets[0]')).toBe(false);
    expect(
      workbenchController.includes(
        'const firstTemplate = nextLibrary.official_templates[0];'
      )
    ).toBe(true);
  });

  test('keeps session model selection visible and independent from Agent identity', () => {
    const page = readSource(new URL('./GuidPage.tsx', import.meta.url));
    const actionRow = readSource(
      new URL('./components/GuidActionRow.tsx', import.meta.url)
    );

    expect(page.includes('isDefaultAgent')).toBe(false);
    expect(page.includes('const modelSelectorNode = (')).toBe(true);
    expect(page.includes('<GuidModelSelector')).toBe(true);
    expect(page.includes('modelSelectorNode={modelSelectorNode}')).toBe(true);
    expect(actionRow.includes('modelSelectorNode: React.ReactNode;')).toBe(true);
  });

  test('does not retain a hidden plain-Nomi launch branch', () => {
    const send = readSource(new URL('./hooks/useGuidSend.ts', import.meta.url));
    const officialLaunch = readSource(
      new URL('./hooks/officialAgentLaunch.ts', import.meta.url)
    );

    expect(send.includes("selection.kind === 'default'")).toBe(false);
    expect(send.includes('ipcBridge.conversation.create.invoke')).toBe(false);
    expect(send.includes('current_model')).toBe(true);
    expect(send.includes('provider_id: current_model.id')).toBe(true);
    expect(send.includes('model: current_model.use_model')).toBe(true);
    expect(
      send.match(/ipcBridge\.agentPlatform\.sessions\.create\.invoke\(/g)
    ).toHaveLength(1);
    expect(officialLaunch.includes('conversation.create')).toBe(false);

    // Official preparation now needs the selected model for validation, but
    // still prepares a stable preset for the same AgentSession launch path.
    expect(compactWhitespace(send)).toContain(
      'launchPreset = await prepareOfficialAgent( selectedTemplate, t(`agentSettings.template.${TEMPLATE_I18N_PATH[selection.templateKey]}.name`), current_model, );'
    );
    expect(officialLaunch).toContain(
      "model?: Pick<TProviderWithModel, 'id' | 'use_model'>,"
    );
    expect(officialLaunch).toContain(
      'createFromTemplate = agentPlatform.createFromTemplate.invoke,'
    );
    // Exact payload equality forbids forwarding provider configuration or
    // credentials, including through a spread of the full selected model.
    expect(
      compactWhitespace(extractObjectArgument(officialLaunch, 'await createFromTemplate('))
    ).toBe(compactWhitespace(`
      template_id: template.template_key,
      request: {
        display_name: displayName,
        model_route_refs: {},
        chat_route_records: {},
        reuse_existing: true,
        ...(model
          ? { model: { provider_id: model.id, model: model.use_model } }
          : {}),
      },
    `));
    expect(officialLaunch).toMatch(
      /if \(!editor\.preset\.current_stable_revision\) \{\s*throw new Error\('AGENT_PRESET_REQUIRED'\);\s*\}\s*return editor\.preset as ExecutableAgentPreset;/
    );
  });

  test('Agent launch submits Agent identity, title, model, and narrow product resource selections', () => {
    const send = readSource(new URL('./hooks/useGuidSend.ts', import.meta.url));
    const payload = extractObjectArgument(
      send,
      'ipcBridge.agentPlatform.sessions.create.invoke'
    );

    expect(topLevelKeys(payload)).toEqual(['preset_id', 'title', 'model']);
    expect(payload.includes('provider_id: current_model.id')).toBe(true);
    expect(payload.includes('model: current_model.use_model')).toBe(true);
    expect(payload.includes('preset_id: launchPreset.preset_id')).toBe(true);
    expect(payload.includes('title: entryPlan.conversationName')).toBe(true);
    expect(payload.includes('resource_selections: resourceSelections')).toBe(true);

    for (const forbidden of [
      'agent_binding',
      'AgentBindingValue',
      'snapshot',
      'revision',
      'credential',
      'base_url',
      'skill',
      'typed_resource_bindings',
    ]) {
      expect(payload.includes(forbidden)).toBe(false);
    }
  });

  test('workbench preselection switches selection to the requested preset mode', () => {
    const page = readSource(new URL('./GuidPage.tsx', import.meta.url));
    const selection = readSource(
      new URL('./hooks/useGuidAgentSelection.ts', import.meta.url)
    );

    expect(page.includes('selectedAgentPresetId?: string;')).toBe(true);
    expect(
      page.includes(
        'const preselectedPresetId = navigationState?.selectedAgentPresetId;'
      )
    ).toBe(true);
    expect(page.includes('selectedAgentPresetId: preselectedPresetId')).toBe(
      true
    );
    expect(
      selection.includes(
        '(candidate) => candidate.preset_id === selectedAgentPresetId'
      )
    ).toBe(true);
    expect(
      selection.includes(
        "setSelection({ kind: 'preset', presetId: preset.preset_id });"
      )
    ).toBe(true);
    expect(selection.includes('selectDefaultTemplate();')).toBe(true);
  });
});
