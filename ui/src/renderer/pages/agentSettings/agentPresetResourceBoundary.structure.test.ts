import { describe, expect, test } from 'bun:test';
import { readFileSync } from 'node:fs';

const sourceFile = (name: string) =>
  readFileSync(new URL(`./${name}`, import.meta.url), 'utf8');

describe('Agent Workbench capability and resource boundary', () => {
  test('keeps concrete resource bindings out of the saved preset authoring contract', () => {
    const sources = [
      sourceFile('AgentPresetEditor.tsx'),
      sourceFile('OfficialTemplateOverview.tsx'),
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

  test('uses one shared capability list for templates and editable presets', () => {
    const editor = sourceFile('AgentPresetEditor.tsx');
    const overview = sourceFile('OfficialTemplateOverview.tsx');
    const capabilityList = sourceFile('AgentCapabilityWorkspace.tsx');

    expect(editor.includes('<AgentCapabilityWorkspace')).toBe(true);
    expect(overview.match(/<AgentCapabilityWorkspace/g)?.length).toBe(1);
    expect(capabilityList.includes('<Select')).toBe(false);
    expect(capabilityList.includes('on_demand')).toBe(false);
    expect(capabilityList.includes('planCapabilityChange')).toBe(true);
    expect(capabilityList.includes('required_resource_kinds')).toBe(true);
  });

  test('forks a template without submitting concrete resource bindings', () => {
    const controller = sourceFile('useAgentSettingsController.ts');
    const forkStart = controller.indexOf('const forkTemplate =');
    const deleteStart = controller.indexOf('const deletePreset =', forkStart);
    const forkBlock = controller.slice(forkStart, deleteStart);

    expect(forkBlock.includes('model_route_refs: modelRouteRefs')).toBe(true);
    expect(forkBlock.includes('chat_route_records: chatRouteRecords')).toBe(true);
    expect(forkBlock.includes('resource_bindings')).toBe(false);
  });

});
