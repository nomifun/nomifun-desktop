import { expect, test } from 'bun:test';
import { asAgentPresetId, createEmptyAgentPresetDocument, type AgentPresetDraft } from '@/common/types/agentPlatform';
import { setBrowserStorageGeneration, type BrowserStoragePersistence } from '@/common/utils/browserStorageKey';
import { AgentEditorDrafts } from './agentEditorDrafts';

const generation = '01900000-0000-7000-8000-000000000001';
const presetId = asAgentPresetId('01900000-0000-7000-8000-000000000011');
function storage(): BrowserStoragePersistence {
  const values = new Map<string, string>();
  return { getItem: key => values.get(key) ?? null, setItem: (key, value) => { values.set(key, value); }, removeItem: key => { values.delete(key); } };
}

test('editing state survives remounts but is isolated by owner, dataset and Agent', () => {
  setBrowserStorageGeneration(generation);
  const persistence = storage();
  const drafts = new AgentEditorDrafts('owner-a', persistence);
  const draft: AgentPresetDraft = { preset_id: presetId, display_name: 'General', source_template_key: 'assistant.general', document: createEmptyAgentPresetDocument() };
  drafts.writePreset(draft);
  drafts.rememberSelection({ kind: 'preset', preset_id: presetId });
  const restored = new AgentEditorDrafts('owner-a', persistence).readPreset(presetId)!;
  expect(restored.preset_id).toBe(presetId);
  expect(restored.display_name).toBe('General');
  expect(restored.source_template_key).toBe('assistant.general');
  expect(new AgentEditorDrafts('owner-a', persistence).readSelection()).toEqual({ kind: 'preset', preset_id: presetId });
  expect(new AgentEditorDrafts('owner-b', persistence).readPreset(presetId)).toBeUndefined();
  expect(new AgentEditorDrafts('owner-b', persistence).readSelection()).toBeNull();
  expect(drafts.readPreset('01900000-0000-7000-8000-000000000012')).toBeUndefined();
  setBrowserStorageGeneration('01900000-0000-7000-8000-000000000002');
  expect(new AgentEditorDrafts('owner-a', persistence).readPreset(presetId)).toBeUndefined();
  expect(new AgentEditorDrafts('owner-a', persistence).readSelection()).toBeNull();
});

test('discarding a preset draft leaves the remembered selection intact', () => {
  setBrowserStorageGeneration(generation);
  const drafts = new AgentEditorDrafts('owner', storage());
  drafts.writePreset({ preset_id: presetId, display_name: 'Mine', document: createEmptyAgentPresetDocument() });
  drafts.rememberSelection({ kind: 'preset', preset_id: presetId });
  drafts.removePreset(presetId);
  expect(drafts.readPreset(presetId)).toBeUndefined();
  expect(drafts.readSelection()).toEqual({ kind: 'preset', preset_id: presetId });
});

test('persisted preset edits omit model routes and chat route records', () => {
  setBrowserStorageGeneration(generation);
  const drafts = new AgentEditorDrafts('owner', storage());
  const document = createEmptyAgentPresetDocument();
  document.model_route_refs = { 'chat.default': 'private-model-route' };
  drafts.writePreset({ preset_id: presetId, display_name: 'Mine', document });
  const restored = drafts.readPreset(presetId)!;
  expect(restored.document).not.toHaveProperty('model_route_refs');
  expect(restored.document).not.toHaveProperty('chat_route_records');
});
