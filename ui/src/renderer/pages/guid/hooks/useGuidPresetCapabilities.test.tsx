import '../../../../../test/setup-dom.ts';

import { act, cleanup, renderHook, waitFor } from '@testing-library/react';
import { afterEach, beforeEach, expect, mock, spyOn, test } from 'bun:test';
import type { PropsWithChildren } from 'react';
import { SWRConfig } from 'swr';
import { agentPlatform } from '@/common/adapter/ipcBridge';
import { configService } from '@/common/config/configService';
import {
  asCapabilityId, asDigestHex, asPackageId, createEmptyAgentPresetDocument,
  AGENT_CHAT_MODEL_TASK, CHAT_ROUTE_RECORD_SCHEMA,
  type AgentPresetDocument, type AgentPresetEditorResponse, type AgentPresetLibraryResponse,
  type AgentPresetSummary, type CapabilityModuleCatalogItem, type OfficialPresetTemplate,
} from '@/common/types/agentPlatform';
import { parseAgentPresetId } from '@/common/types/ids';
import { createDefaultIdmmConfig } from '@/common/types/idmm';
import { setBrowserStorageGeneration } from '@/common/utils/browserStorageKey';
import { resolveAgentResourceSelections } from '@/renderer/hooks/agent/agentResourceSelection';
import { useGuidAgentSelection } from './useGuidAgentSelection';
import { useGuidPresetCapabilities } from './useGuidPresetCapabilities';

const officialId = parseAgentPresetId('0190f5fe-7c00-7a00-8000-000000000108');
const personalId = parseAgentPresetId('0190f5fe-7c00-7a00-8000-000000000109');
const template: OfficialPresetTemplate = {
  template_key: 'assistant.general',
  seed: { enabled_capabilities: [], skill_bindings: [], required_resource_kinds: [], required_runtime_features: [] },
  role_coverage: { required_capability_categories: [], required_capability_ids: [], required_runtime_features: [], required_resource_kinds: [] },
  immutable: true, forkable: true,
};
const capability = { id: asCapabilityId('platform.remote-review') };
const catalog: CapabilityModuleCatalogItem[] = [{
  module: capability, summary_kind: 'tool', display_name: 'Remote reviewer', description: '',
  source_package: { id: asPackageId('platform'), version: '1.0.0' },
  authoring_policy: 'direct', supported_surfaces: ['desktop'],
  actions: [{ action_id: 'platform.remote-review/read', effect_class: 'read_local', presentation: 'visible' }],
  context_schema_refs: [], event_schema_refs: [], required_host_ports: [],
  required_resource_kinds: ['customer'], required_modules: [], conflicting_modules: [],
}];
const summary = (revision?: number): AgentPresetSummary => ({
  preset_id: officialId, source: 'official', display_name: 'Assistant', bound_target_count: 0,
  ...(revision ? { current_stable_revision: {
    preset_id: officialId, revision, revision_digest: asDigestHex(String(revision).repeat(64)),
  } } : {}),
});
const editor = (preset: AgentPresetSummary, document: AgentPresetDocument): AgentPresetEditorResponse => ({
  preset,
  draft: { preset_id: preset.preset_id, display_name: preset.display_name, document: createEmptyAgentPresetDocument() },
  ...(preset.current_stable_revision ? { revision: {
    reference: preset.current_stable_revision, document, created_by: 'owner', created_at_ms: 1,
  } } : { draft: { preset_id: preset.preset_id, display_name: preset.display_name, document } }),
});
const library = (preset: AgentPresetSummary): AgentPresetLibraryResponse => ({
  official_templates: [template], official_presets: { 'assistant.general': preset },
  user_presets: [], active_bindings: [],
  fresh_start: { data_generation: 1, legacy_data_imported: false, official_template_count: 1, user_preset_count: 0 },
});
const extendedDocument = (): AgentPresetDocument => ({
  ...createEmptyAgentPresetDocument(),
  enabled_capabilities: [{ capability, action_allowlist: ['platform.remote-review/read'] }],
  runtime_policy: { idmm: { ...createDefaultIdmmConfig(), mode: 'rule_only' } },
});
const setDefaultModel = (document: AgentPresetDocument, model: string): AgentPresetDocument => {
  document.model_route_refs[AGENT_CHAT_MODEL_TASK] = `host-route-${model}`;
  document.chat_route_records[AGENT_CHAT_MODEL_TASK] = {
    schema: CHAT_ROUTE_RECORD_SCHEMA, task: AGENT_CHAT_MODEL_TASK,
    primary: { model_route_id: `host-route-${model}`, model_route_revision: 1,
      provider_id: '0190f5fe-7c00-7a00-8000-000000000201', model, protocol: 'openai_chat',
      connection_config_ref: 'default', config_revision_digest: asDigestHex('a'.repeat(64)),
      credential_ref: 'host-credential', features: ['text_input', 'text_output'] },
    failovers: [],
  };
  return document;
};

beforeEach(() => {
  configService.reset();
  setBrowserStorageGeneration('0190f5fe-7c00-7a00-8000-000000000110');
  spyOn(agentPlatform.catalog, 'invoke').mockResolvedValue({ modules: catalog, capabilities: [], skills: [], mcp_tools: [], roles: [] });
});
afterEach(() => { cleanup(); mock.restore(); sessionStorage.clear(); configService.reset(); });

function mount(preset: AgentPresetSummary) {
  const cache = new Map();
  const wrapper = ({ children }: PropsWithChildren) => <SWRConfig value={{
    provider: () => cache, fallback: { 'agent-presets.library': library(preset) },
    revalidateOnMount: false, shouldRetryOnError: false,
  }}>{children}</SWRConfig>;
  return renderHook(() => {
    const selection = useGuidAgentSelection({ selectedAgentTemplateKey: 'assistant.general' });
    const configured = selection.selectedOfficialPreset ?? selection.selectedPreset;
    const capabilities = useGuidPresetCapabilities(configured?.preset_id, configured?.current_stable_revision?.revision_digest);
    return { selection, capabilities };
  }, { wrapper });
}

test('official selection reads its baseline draft without preparing or saving a Revision', async () => {
  const preset = summary();
  const getEditor = spyOn(agentPlatform.getEditor, 'invoke').mockResolvedValue(editor(preset, extendedDocument()));
  const prepare = spyOn(agentPlatform.createFromTemplate, 'invoke');
  const save = spyOn(agentPlatform.saveRevision, 'invoke');
  const hook = mount(preset);
  expect(hook.result.current.capabilities.isLoading).toBe(true);
  await waitFor(() => expect(hook.result.current.capabilities.isLoading).toBe(false));
  expect(hook.result.current.selection.selection).toEqual({ kind: 'template', templateKey: 'assistant.general' });
  expect(hook.result.current.selection.selectedOfficialPreset?.preset_id).toBe(officialId);
  expect([...hook.result.current.capabilities.actionIds]).toEqual(['platform.remote-review/read']);
  expect([...hook.result.current.capabilities.requiredResourceKinds]).toEqual(['customer']);
  expect(hook.result.current.capabilities.idmm.mode).toBe('rule_only');
  expect(hook.result.current.capabilities.defaultModel).toBeNull();
  expect(getEditor).toHaveBeenCalledWith({ preset_id: officialId });
  expect(prepare).not.toHaveBeenCalled();
  expect(save).not.toHaveBeenCalled();
});

test('a saved general Agent keeps asset grants without showing canvas requirements', async () => {
  const preset = summary(1);
  const document: AgentPresetDocument = { ...createEmptyAgentPresetDocument(), enabled_capabilities: [{
    capability: { id: asCapabilityId('creative.workshop') },
    action_allowlist: ['creative.workshop/asset.read', 'creative.workshop/asset.write'],
  }] };
  spyOn(agentPlatform.getEditor, 'invoke').mockResolvedValue(editor(preset, document));
  spyOn(agentPlatform.catalog, 'invoke').mockResolvedValue({
    modules: [{ ...catalog[0], module: document.enabled_capabilities[0].capability,
      required_resource_kinds: ['asset_library', 'canvas'],
      actions: [...['read', 'write'].map(operation => ({
        ...catalog[0].actions[0], action_id: `creative.workshop/asset.${operation}`,
        required_resource_operations: { asset_library: [operation] },
      })), { ...catalog[0].actions[0], action_id: 'creative.workshop/canvas.read',
        required_resource_operations: { canvas: ['read'] },
      }],
    }], capabilities: [], skills: [], mcp_tools: [], roles: [],
  });
  const hook = mount(preset);
  await waitFor(() => expect(hook.result.current.capabilities.isLoading).toBe(false));
  expect(hook.result.current.capabilities.actionIds).toEqual(new Set(document.enabled_capabilities[0].action_allowlist));
  expect(hook.result.current.capabilities.requiredResourceKinds).toEqual(new Set(['asset_library']));
});

test('saving extended official capabilities at the same ID refreshes resource and tool preflight before sending', async () => {
  const initial = summary(1);
  const getEditor = spyOn(agentPlatform.getEditor, 'invoke').mockResolvedValue(editor(initial, createEmptyAgentPresetDocument()));
  const hook = mount(initial);
  await waitFor(() => expect(hook.result.current.capabilities.isLoading).toBe(false));
  expect(hook.result.current.capabilities.actionIds.size).toBe(0);
  expect(hook.result.current.capabilities.requiredResourceKinds.size).toBe(0);

  const saved = summary(2);
  spyOn(agentPlatform.library, 'invoke').mockResolvedValue(library(saved));
  let resolveEditor!: (value: AgentPresetEditorResponse) => void;
  getEditor.mockImplementation(() => new Promise(resolve => { resolveEditor = resolve; }));
  await act(async () => { await hook.result.current.selection.refreshPresets(); });
  expect(hook.result.current.selection.selectedOfficialPreset?.preset_id).toBe(officialId);
  expect(hook.result.current.capabilities.isLoading).toBe(true);
  expect(hook.result.current.capabilities.actionIds.size).toBe(0);
  await act(async () => { resolveEditor(editor(saved, extendedDocument())); });
  await waitFor(() => expect(hook.result.current.capabilities.isLoading).toBe(false));

  const resolved = hook.result.current.capabilities;
  expect(resolved.error).toBeUndefined();
  // Guid derives function_calling from these saved action requirements, and
  // its existing resource gate must now ask for the newly required customer.
  expect(resolved.actionIds.size).toBeGreaterThan(0);
  expect([...resolved.capabilityIds]).toEqual([capability.id]);
  expect(resolveAgentResourceSelections(resolved.requiredResourceKinds, {}).missingKinds).toEqual(['customer']);
  expect(resolveAgentResourceSelections(resolved.requiredResourceKinds, { customer: 'chosen-customer' }).missingKinds).toEqual([]);
  expect(resolved.idmm.mode).toBe('rule_only');
  expect(getEditor).toHaveBeenCalledTimes(2);
  expect(hook.result.current.selection.selection).toEqual({ kind: 'template', templateKey: 'assistant.general' });
});

test('a saved personal Revision remains authoritative over transient editor draft changes', async () => {
  const initial = summary(1);
  const preset = { ...initial, preset_id: personalId, source: 'user' as const,
    current_stable_revision: { ...initial.current_stable_revision!, preset_id: personalId },
  };
  const response = editor(preset, setDefaultModel(createEmptyAgentPresetDocument(), 'saved-model'));
  response.draft.document = setDefaultModel(extendedDocument(), 'unsaved-model');
  spyOn(agentPlatform.getEditor, 'invoke').mockResolvedValue(response);
  const hook = renderHook(() => useGuidPresetCapabilities(personalId, preset.current_stable_revision?.revision_digest));
  await waitFor(() => expect(hook.result.current.isLoading).toBe(false));
  expect(hook.result.current.actionIds.size).toBe(0);
  expect(hook.result.current.requiredResourceKinds.size).toBe(0);
  expect(hook.result.current.idmm).toEqual(createDefaultIdmmConfig());
  expect(hook.result.current.defaultModel).toEqual({
    provider_id: '0190f5fe-7c00-7a00-8000-000000000201', model: 'saved-model',
  });
});

test('late model defaults from another Agent or Revision cannot replace the currently selected document', async () => {
  const pending: ((response: AgentPresetEditorResponse) => void)[] = [];
  spyOn(agentPlatform.getEditor, 'invoke').mockImplementation(() => new Promise(resolve => { pending.push(resolve); }));
  const initial = summary(1);
  const hook = renderHook(({ id, digest }) => useGuidPresetCapabilities(id, digest), {
    initialProps: { id: officialId, digest: initial.current_stable_revision!.revision_digest },
  });
  const next = { ...summary(2), preset_id: personalId };
  hook.rerender({ id: personalId, digest: next.current_stable_revision!.revision_digest });
  expect(hook.result.current.defaultModel).toBeNull();
  expect(hook.result.current.isLoading).toBe(true);
  await act(async () => { pending[1](editor(next, setDefaultModel(createEmptyAgentPresetDocument(), 'new-model'))); });
  await waitFor(() => expect(hook.result.current.defaultModel?.model).toBe('new-model'));
  await act(async () => { pending[0](editor(initial, setDefaultModel(createEmptyAgentPresetDocument(), 'old-model'))); });
  expect(hook.result.current.defaultModel?.model).toBe('new-model');
  hook.rerender({ id: personalId, digest: asDigestHex('3'.repeat(64)) });
  expect(hook.result.current.defaultModel).toBeNull();
  expect(hook.result.current.isLoading).toBe(true);
});

test('an official editor read failure exposes an error and cannot use its immutable template as launch authority', async () => {
  const failure = new Error('Official configuration unavailable');
  spyOn(agentPlatform.getEditor, 'invoke').mockRejectedValue(failure);
  spyOn(console, 'error').mockImplementation(() => {});
  const hook = mount(summary(1));
  await waitFor(() => expect(hook.result.current.capabilities.error).toBe(failure));
  expect(hook.result.current.selection.selectedOfficialPreset).toBeDefined();
  expect(hook.result.current.capabilities.actionIds.size).toBe(0);
  expect(hook.result.current.capabilities.requiredResourceKinds.size).toBe(0);
});
