/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { readFileSync } from 'node:fs';
import { describe, expect, test } from 'bun:test';

const readSource = (url: URL): string =>
  readFileSync(url, 'utf8').replace(/\r\n/g, '\n');

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

describe('Guid default Nomi and AgentPreset launch behavior', () => {
  test('uses the exact default-or-preset selection contract', () => {
    const configKeys = readSource(
      new URL('../../../common/config/configKeys.ts', import.meta.url)
    );
    const types = readSource(new URL('./types.ts', import.meta.url));
    const selection = readSource(
      new URL('./hooks/useGuidAgentSelection.ts', import.meta.url)
    );

    expect(configKeys.includes("| { kind: 'default' }")).toBe(true);
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
      selection.includes(
        "const DEFAULT_AGENT_SELECTION: GuidAgentSelection = { kind: 'default' };"
      )
    ).toBe(true);
    expect(selection.includes('presets[0]')).toBe(false);
  });

  test('exposes the model selector only in default Nomi mode', () => {
    const page = readSource(new URL('./GuidPage.tsx', import.meta.url));
    const actionRow = readSource(
      new URL('./components/GuidActionRow.tsx', import.meta.url)
    );

    expect(
      page.includes(
        "const isDefaultAgent = agentSelection.selection.kind === 'default';"
      )
    ).toBe(true);
    expect(
      page.includes('const modelSelectorNode = isDefaultAgent ? (')
    ).toBe(true);
    expect(page.includes('<GuidModelSelector')).toBe(true);
    expect(page.includes('modelSelectorNode={modelSelectorNode}')).toBe(true);
    expect(actionRow.includes('modelSelectorNode?: React.ReactNode;')).toBe(
      true
    );
    expect(actionRow.includes('{modelSelectorNode && (')).toBe(true);
  });

  test('default mode creates a Nomi conversation with the selected model and no preset dependency', () => {
    const send = readSource(new URL('./hooks/useGuidSend.ts', import.meta.url));
    const defaultBranchStart = send.indexOf(
      "if (selection.kind === 'default')"
    );
    const presetBranchStart = send.indexOf('    } else {', defaultBranchStart);
    expect(defaultBranchStart).toBeGreaterThan(-1);
    expect(presetBranchStart).toBeGreaterThan(defaultBranchStart);
    const defaultBranch = send.slice(defaultBranchStart, presetBranchStart);
    const payload = extractObjectArgument(
      defaultBranch,
      'ipcBridge.conversation.create.invoke'
    );

    expect(defaultBranch.includes('selectedPreset')).toBe(false);
    expect(defaultBranch.includes('agentPlatform.sessions.create')).toBe(false);
    expect(payload.includes("type: 'nomi'")).toBe(true);
    expect(payload.includes('model: current_model')).toBe(true);
    expect(payload.includes('preset_id')).toBe(false);
    expect(
      send.includes(
        "selection.kind === 'default'\n      ? Boolean(current_model)"
      )
    ).toBe(true);
  });

  test('preset mode submits only preset_id and title to the high-level session API', () => {
    const send = readSource(new URL('./hooks/useGuidSend.ts', import.meta.url));
    const payload = extractObjectArgument(
      send,
      'ipcBridge.agentPlatform.sessions.create.invoke'
    );

    expect(topLevelKeys(payload)).toEqual(['preset_id', 'title']);
    expect(payload.includes('preset_id: selectedPreset.preset_id')).toBe(true);
    expect(payload.includes('title: entryPlan.conversationName')).toBe(true);

    for (const forbidden of [
      'agent_binding',
      'AgentBindingValue',
      'snapshot',
      'revision',
      'current_model',
      'model',
      'provider',
      'skill',
      'mcp',
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
    expect(selection.includes('selectDefaultAgent();')).toBe(true);
  });
});
