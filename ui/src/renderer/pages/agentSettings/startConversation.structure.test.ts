/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { readFileSync } from 'node:fs';
import { describe, expect, test } from 'bun:test';
import en from '../../services/i18n/locales/en-US/agentSettings.json';
import zh from '../../services/i18n/locales/zh-CN/agentSettings.json';

const readSource = (url: URL) => readFileSync(url, 'utf8');

describe('Agent Workbench start conversation boundary', () => {
  test('enables the action only for a saved clean preset', () => {
    const editor = readSource(new URL('./AgentPresetEditor.tsx', import.meta.url));
    const page = readSource(new URL('./AgentSettingsPage.tsx', import.meta.url));

    expect(editor.includes('dirty: boolean')).toBe(true);
    expect(editor.includes('hasStableRevision={Boolean(editor.preset.current_stable_revision)}')).toBe(
      true
    );
    expect(editor.includes('disabled={busy || !hasStableRevision || dirty}')).toBe(true);
    expect(page.includes('dirty={controller.dirty}')).toBe(true);
  });

  test('navigates with only the selected preset identity', () => {
    const page = readSource(new URL('./AgentSettingsPage.tsx', import.meta.url));
    const actionStart = page.indexOf('const startConversation =');
    const actionEnd = page.indexOf('};', actionStart) + 2;
    const action = page.slice(actionStart, actionEnd);

    expect(action.includes("navigate('/guid'")).toBe(true);
    expect(action.includes('selectedAgentPresetId: preset.preset_id')).toBe(true);
    expect(page.includes('onStartConversation={startConversation}')).toBe(true);
    expect(editorSource().includes('onClick={() => onStartConversation(editor.preset)}')).toBe(
      true
    );
    expect(action.includes('session')).toBe(false);
    expect(action.includes('model')).toBe(false);
    expect(action.includes('binding')).toBe(false);
    expect(action.includes('snapshot')).toBe(false);
  });

  test('keeps the action copy localized', () => {
    expect(en.actions.startConversation).toBe('Start conversation');
    expect(zh.actions.startConversation).toBe('使用 Agent');
  });
});

function editorSource(): string {
  return readSource(new URL('./AgentPresetEditor.tsx', import.meta.url));
}
