import { existsSync, readFileSync } from 'node:fs';
import { describe, expect, test } from 'bun:test';

const source = (name: string) => readFileSync(new URL(`./${name}`, import.meta.url), 'utf8');

describe('single Nomi Runtime authoring boundary', () => {
  test('physically removes the Agent Runtime selector and its compatibility path', () => {
    expect(existsSync(new URL('./AgentRuntimeEngineSelector.tsx', import.meta.url))).toBe(false);
    expect(existsSync(new URL('./AgentRuntimeEngineSelector.test.tsx', import.meta.url))).toBe(false);
    const editor = source('AgentPresetEditor.tsx');
    const template = source('OfficialTemplateOverview.tsx');
    const controller = source('useAgentSettingsController.ts');
    expect(editor).not.toMatch(/RuntimeEngineSelector|runtime_engine|runtimeEngines/);
    expect(template).not.toMatch(/RuntimeEngineSelector|runtime_engine|runtimeEngines/);
    expect(controller).not.toMatch(/withoutRuntimeSelection|runtime_engine|runtimeEngines/);
    const contracts = readFileSync(new URL('../../../common/types/agentPlatform/contracts.ts', import.meta.url), 'utf8');
    expect(contracts).not.toContain('RuntimeEngineSelection');
  });

  test('keeps the server save/compile path authoritative', () => {
    const controller = source('useAgentSettingsController.ts');
    expect(controller.includes('agentPlatform.saveRevision.invoke')).toBe(true);
    expect(controller).not.toMatch(/compileSnapshot|compileLocally|resolved_snapshot_ref\s*:/);
    expect(source('AgentPresetEditor.tsx').includes('previewCompileHint')).toBe(true);
  });
});
