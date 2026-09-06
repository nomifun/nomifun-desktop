import { describe, expect, test } from 'bun:test';
import { readFileSync } from 'node:fs';

const sourceFile = (name: string) =>
  readFileSync(new URL(`./${name}`, import.meta.url), 'utf8');

describe('Agent Workbench capability and resource boundary', () => {
  test('keeps concrete resource selection out of every authoring surface', () => {
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

  test('uses one shared capability list for templates and editable presets', () => {
    const editor = sourceFile('AgentPresetEditor.tsx');
    const overview = sourceFile('OfficialTemplateOverview.tsx');
    const capabilityList = sourceFile('AgentCapabilityList.tsx');

    expect(editor.includes('<AgentCapabilityList')).toBe(true);
    expect(overview.match(/<AgentCapabilityList/g)?.length).toBe(2);
    expect(capabilityList.includes("<Radio.Group")).toBe(true);
    expect(capabilityList.includes("value='none'")).toBe(true);
    expect(capabilityList.includes("value='initial'")).toBe(true);
    expect(capabilityList.includes("value='on_demand'")).toBe(true);
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

  test('reads requirement counts and kinds from the corrected preview contract', () => {
    const inspector = sourceFile('PreviewInspector.tsx');

    expect(inspector.includes('preview.summary.required_resource_kind_count')).toBe(true);
    expect(inspector.includes('preview.inspector.required_resource_kinds')).toBe(true);
    expect(inspector.includes('resource_binding_count')).toBe(false);
    expect(inspector.includes('typed_resource_bindings')).toBe(false);
  });
});
