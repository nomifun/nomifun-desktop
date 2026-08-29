import { describe, expect, test } from 'bun:test';
import { readFileSync } from 'node:fs';

const read = (name: string) => readFileSync(new URL(name, import.meta.url), 'utf8');

describe('Agent Settings single-page contract', () => {
  test('keeps every progressive control and primary action on one page', () => {
    const page = read('./AgentSettingsPage.tsx');
    const editor = read('./AgentPresetEditor.tsx');

    expect(page.includes('<AgentPresetLibrary')).toBe(true);
    expect(page.includes('<AgentPresetEditor')).toBe(true);
    expect(editor.includes("id='agent-settings-basic'")).toBe(true);
    expect(editor.includes("id='agent-settings-capabilities'")).toBe(true);
    expect(editor.includes("id='agent-settings-skills-mcp'")).toBe(true);
    expect(editor.includes("id='agent-settings-resources'")).toBe(true);
    expect(editor.includes("id='agent-settings-preview'")).toBe(true);
    expect(editor.includes("id='agent-settings-test'")).toBe(true);
    expect(editor.includes('agentSettings.actions.preview')).toBe(true);
    expect(editor.includes('agentSettings.actions.saveRevision')).toBe(true);
    expect(editor.includes('agentSettings.actions.test')).toBe(true);
  });

  test('uses canonical APIs and has no alternate editor execution endpoint', () => {
    const bridge = read('../../../common/adapter/ipcBridge.ts');
    const forbiddenTestPath = '/api/' + 'test';

    expect(bridge.includes("'/api/agent-preset-templates?source=official'")).toBe(true);
    expect(bridge.includes("'/api/agent-presets'")).toBe(true);
    expect(bridge.includes("'/api/agent-sessions'")).toBe(true);
    expect(bridge.includes(forbiddenTestPath)).toBe(false);
    expect(bridge.includes('/test-' + 'sessions')).toBe(false);
  });

  test('keeps Test as a static real-effect warning without approval state', () => {
    const editor = read('./AgentPresetEditor.tsx');
    const locale = read('../../services/i18n/locales/en-US/agentSettings.json');

    expect(editor.includes('agentSettings.test.realEffectWarning')).toBe(true);
    expect(locale.includes('FullAuto')).toBe(true);
    expect(locale.includes('not simulated')).toBe(true);
    expect(editor.includes('awaiting_' + 'approval')).toBe(false);
    expect(editor.includes('require_' + 'approval')).toBe(false);
  });
});
