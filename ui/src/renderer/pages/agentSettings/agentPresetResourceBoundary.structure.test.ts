import { describe, expect, test } from 'bun:test';
import { readFileSync } from 'node:fs';

const sourceFile = (name: string) =>
  readFileSync(new URL(`./${name}`, import.meta.url), 'utf8');

describe('Agent Workbench capability and resource boundary', () => {
  test('keeps concrete resource bindings out of the saved preset authoring contract', () => {
    const sources = [
      sourceFile('AgentPresetEditor.tsx'),
      sourceFile('AgentSettingsPage.tsx'),
      sourceFile('useAgentSettingsController.ts'),
      sourceFile('model.ts'),
    ].join('\n');
    const forbidden = [
      'resource_bindings',
      'resource_binding_refs',
      'TemplateResourceSelection',
      'WorkspaceFolderSelect',
      'useKnowledgeBases',
      'hostWorkDir',
      'knowledgeBases',
      'connectors',
      'bindKnowledgeBaseResource',
      'bindWorkspaceResource',
      'resolveHostManagedResourceBindings',
    ];

    for (const token of forbidden) {
      expect(sources.includes(token)).toBe(false);
    }
  });

  test('does not expose a test-session path from the Agent editor', () => {
    const editor = sourceFile('AgentPresetEditor.tsx');
    const controller = sourceFile('useAgentSettingsController.ts');

    expect(editor.includes('<AgentResourcePicker')).toBe(false);
    expect(editor.includes("activeTab === 'test'")).toBe(false);
    expect(editor.includes('onTest')).toBe(false);
    expect(controller.includes('runAgentPresetTest')).toBe(false);
    expect(controller.includes('createTurn')).toBe(false);
  });

  test('uses the shared capability list for all editable presets', () => {
    const editor = sourceFile('AgentPresetEditor.tsx');
    const capabilityList = sourceFile('AgentCapabilityWorkspace.tsx');

    expect(editor.includes('<AgentCapabilityWorkspace')).toBe(true);
    expect(capabilityList.includes('<Select')).toBe(false);
    expect(capabilityList.includes('on_demand')).toBe(false);
    expect(capabilityList.includes('planModuleChange')).toBe(true);
    expect(capabilityList.includes('catalog.modules')).toBe(true);
    expect(capabilityList.includes('action_allowlist')).toBe(true);
    expect(capabilityList.includes('required_resource_kinds')).toBe(true);
  });

});
