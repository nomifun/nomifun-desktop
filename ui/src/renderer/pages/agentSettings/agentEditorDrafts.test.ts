import { expect, test } from 'bun:test';
import { asAgentPresetId, asCapabilityId, createEmptyAgentPresetDocument, type AgentPresetDraft } from '@/common/types/agentPlatform';
import { setBrowserStorageGeneration, type BrowserStoragePersistence } from '@/common/utils/browserStorageKey';
import { AgentEditorDrafts, rebaseEditorActionGrants } from './agentEditorDrafts';

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

const grantsDocument = (actions: string[]) => ({
  ...createEmptyAgentPresetDocument(),
  enabled_capabilities: [{ capability: { id: asCapabilityId('workspace.process') }, action_allowlist: actions }],
});

test('rebases a proved one-to-one grant change while preserving local edits and unknown actions', () => {
  const saved = grantsDocument(['workspace.process/retired', 'workspace.process/poll']);
  const current = grantsDocument(['workspace.process/start', 'workspace.process/poll']);
  const local = { ...grantsDocument(['workspace.process/retired', 'workspace.process/unknown']),
    persona: 'My unsaved persona', instructions: 'Keep my instructions',
  };
  const originals = structuredClone({ saved, current, local });
  const rebased = rebaseEditorActionGrants(local, saved, current);
  expect(rebased).toEqual({ ...local, enabled_capabilities: [{
    ...local.enabled_capabilities[0], action_allowlist: ['workspace.process/start', 'workspace.process/unknown'],
  }] });
  expect({ saved, current, local }).toEqual(originals);
});

test('rebasing preserves explicitly removed grants and modules without restoring server selections', () => {
  const saved = grantsDocument(['workspace.process/retired']);
  const current = grantsDocument(['workspace.process/start']);
  expect(rebaseEditorActionGrants(grantsDocument([]), saved, current)).toEqual(grantsDocument([]));
  const removed = { ...grantsDocument([]), enabled_capabilities: [] };
  expect(rebaseEditorActionGrants(removed, saved, current)).toEqual(removed);
  const selectedBoth = grantsDocument(['workspace.process/retired', 'workspace.process/start']);
  expect(rebaseEditorActionGrants(selectedBoth, saved, current)).toEqual(current);
});

test('removes the General Agent redundant retired grant only when local edits retain every current grant', () => {
  const currentActions = ['start', 'poll', 'input', 'close_stdin', 'resize', 'cancel']
    .map(action => `workspace.process/${action}`);
  const saved = grantsDocument(['workspace.process/exec', ...currentActions]);
  const current = grantsDocument(currentActions);
  const local = { ...grantsDocument(['workspace.process/exec', ...currentActions, 'workspace.process/unknown']),
    instructions: 'My unsaved instructions', persona: 'My unsaved persona',
  };
  const rebased = rebaseEditorActionGrants(local, saved, current);
  expect(rebased).toEqual({ ...local, enabled_capabilities: [{
    ...local.enabled_capabilities[0], action_allowlist: [...currentActions, 'workspace.process/unknown'],
  }] });
  const trimmed = { ...local, enabled_capabilities: [{
    ...local.enabled_capabilities[0], action_allowlist: ['workspace.process/exec', 'workspace.process/poll'],
  }] };
  expect(rebaseEditorActionGrants(trimmed, saved, current)).toEqual(trimmed);
});

test('rebasing does not infer ambiguous, add-only, empty or cross-module action replacements', () => {
  const local = grantsDocument(['workspace.process/retired', 'workspace.process/unknown']);
  for (const [before, after] of [
    [['workspace.process/retired'], ['workspace.process/start', 'workspace.process/input']],
    [['workspace.process/retired', 'workspace.process/other'], ['workspace.process/start']],
    [['workspace.process/retired'], []],
    [['workspace.process/retired'], ['workspace.process/retired', 'workspace.process/start']],
  ]) {
    expect(rebaseEditorActionGrants(local, grantsDocument(before), grantsDocument(after))).toEqual(local);
  }
  const otherModule = { ...grantsDocument(['another.module/start']), enabled_capabilities: [{
    capability: { id: asCapabilityId('another.module') }, action_allowlist: ['another.module/start'],
  }] };
  expect(rebaseEditorActionGrants(local, grantsDocument(['workspace.process/retired']), otherModule)).toEqual(local);
  expect(rebaseEditorActionGrants(local, undefined, grantsDocument(['workspace.process/start']))).toBe(local);
});
