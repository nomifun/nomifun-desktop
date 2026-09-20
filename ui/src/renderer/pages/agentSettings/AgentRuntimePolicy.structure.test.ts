/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { readFileSync } from 'node:fs';
import { describe, expect, test } from 'bun:test';

const source = (name: string) => readFileSync(new URL(name, import.meta.url), 'utf8');

describe('Agent runtime policy workbench', () => {
  test('presents IDMM as revisioned runtime policy rather than a capability module', () => {
    const editor = source('./AgentPresetEditor.tsx');
    const template = source('./OfficialTemplateOverview.tsx');
    const panel = source('./AgentRuntimePolicyPanel.tsx');
    const contracts = source('../../../common/types/agentPlatform/contracts.ts');

    expect(editor.includes("key: 'runtime'")).toBe(true);
    expect(template.includes("key: 'runtime'")).toBe(true);
    expect(panel.includes("presentation='embedded'")).toBe(true);
    expect(panel.includes('document.runtime_policy')).toBe(true);
    expect(contracts.includes('runtime_policy:')).toBe(true);
    expect(panel.includes("capability: { id: 'idmm'")).toBe(false);
    expect(panel.includes("'idmm.observe'")).toBe(false);
    expect(panel.includes("'idmm.intervene'")).toBe(false);
  });

  test('explains the Agent to Session freeze and override boundary', () => {
    const editor = source('./AgentRuntimePolicyPanel.tsx');
    const styles = source('./AgentSettingsPage.module.css');

    expect(editor.includes("['agent', 'session', 'override']")).toBe(true);
    expect(styles.includes('.runtimePolicyFlow')).toBe(true);
    expect(styles.includes('@container agent-editor (max-width:720px)')).toBe(true);
  });
});
