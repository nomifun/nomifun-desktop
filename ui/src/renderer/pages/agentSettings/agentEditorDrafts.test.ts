import { expect, test } from 'bun:test';
import { asAgentPresetId, asCapabilityId, createEmptyAgentPresetDocument, type OfficialPresetTemplate } from '@/common/types/agentPlatform';
import { setBrowserStorageGeneration, type BrowserStoragePersistence } from '@/common/utils/browserStorageKey';
import { editingDocument } from './model';
import { AgentEditorDrafts } from './agentEditorDrafts';

const generation = '01900000-0000-7000-8000-000000000001';
const template: OfficialPresetTemplate = { template_key: 'assistant.general', immutable: true, forkable: true,
  seed: { enabled_capabilities: [], skill_bindings: [], required_resource_kinds: [], required_runtime_features: [] },
  role_coverage: { required_capability_ids: [], required_capability_categories: [], required_resource_kinds: [], required_runtime_features: [] } };
function storage(): BrowserStoragePersistence {
  const values = new Map<string, string>();
  return { getItem: key => values.get(key) ?? null, setItem: (key, value) => { values.set(key, value); }, removeItem: key => { values.delete(key); } };
}

test('editing state survives remounts but is isolated by owner, dataset and Agent', () => {
  setBrowserStorageGeneration(generation);
  const persistence = storage();
  const drafts = new AgentEditorDrafts('owner-a', persistence);
  const editing = { displayName: 'General', document: editingDocument(createEmptyAgentPresetDocument()), activeTab: 'capabilities' };
  drafts.writeTemplate(template, editing);
  drafts.rememberSelection({ kind: 'template', template_key: template.template_key });
  expect(new AgentEditorDrafts('owner-a', persistence).readTemplate(template)).toEqual(editing);
  expect(new AgentEditorDrafts('owner-b', persistence).readTemplate(template)).toBeUndefined();
  expect(new AgentEditorDrafts('owner-b', persistence).readSelection()).toBeNull();
  expect(drafts.readTemplate({ ...template, template_key: 'coding.codex' })).toBeUndefined();
  setBrowserStorageGeneration('01900000-0000-7000-8000-000000000002');
  expect(new AgentEditorDrafts('owner-a', persistence).readTemplate(template)).toBeUndefined();
  expect(new AgentEditorDrafts('owner-a', persistence).readSelection()).toBeNull();
});

test('a new official seed cannot be overwritten by a draft of its old defaults', () => {
  setBrowserStorageGeneration(generation);
  const drafts = new AgentEditorDrafts('owner', storage());
  drafts.writeTemplate(template, { displayName: 'General', document: editingDocument(createEmptyAgentPresetDocument()), activeTab: 'capabilities' });
  expect(drafts.readTemplate({ ...template, seed: { ...template.seed, enabled_capabilities: [{ capability: { id: asCapabilityId('plugin.development') },
    action_allowlist: ['plugin.development/list'] }] } })).toBeUndefined();
});

test('persisted personal edits omit model routes and chat route records', () => {
  setBrowserStorageGeneration(generation);
  const drafts = new AgentEditorDrafts('owner', storage());
  const document = createEmptyAgentPresetDocument();
  document.model_route_refs = { 'chat.default': 'private-model-route' };
  drafts.writePreset({ preset_id: asAgentPresetId('01900000-0000-7000-8000-000000000011'), display_name: 'Mine', document });
  const restored = drafts.readPreset('01900000-0000-7000-8000-000000000011')!;
  expect(restored.document).not.toHaveProperty('model_route_refs');
  expect(restored.document).not.toHaveProperty('chat_route_records');
});
