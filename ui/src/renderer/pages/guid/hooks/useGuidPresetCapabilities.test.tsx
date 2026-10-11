import '../../../../../test/setup-dom.ts';

import { act, cleanup, renderHook, waitFor } from '@testing-library/react';
import { afterEach, beforeEach, expect, mock, spyOn, test } from 'bun:test';
import type { PropsWithChildren } from 'react';
import { SWRConfig } from 'swr';
import { agentPlatform } from '@/common/adapter/ipcBridge';
import { configService } from '@/common/config/configService';
import {
  asCapabilityId, asDigestHex, asPackageId, createEmptyAgentPresetDocument,
  type AgentPresetDocument, type AgentPresetEditorResponse, type AgentPresetLibraryResponse,
  type AgentPresetSummary, type CapabilityCatalogItem, type OfficialPresetTemplate,
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
const catalog: CapabilityCatalogItem[] = [{
  capability, kind: 'tool', display_name: 'Remote reviewer', description: '',
  source_package: { id: asPackageId('platform'), version: '1.0.0' },
  source_kind: 'bundled', materialization_state: 'materialized', supported_surfaces: ['conversation'],
  required_runtime_features: ['tool_calls'], required_resource_kinds: ['customer'],
  required_capabilities: [], conflicting_capabilities: [], action_count: 1, context_contributor_count: 0,
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

beforeEach(() => {
  configService.reset();
  setBrowserStorageGeneration('0190f5fe-7c00-7a00-8000-000000000110');
  spyOn(agentPlatform.capabilities, 'invoke').mockResolvedValue(catalog);
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
  expect(getEditor).toHaveBeenCalledWith({ preset_id: officialId });
  expect(prepare).not.toHaveBeenCalled();
  expect(save).not.toHaveBeenCalled();
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
  const response = editor(preset, createEmptyAgentPresetDocument());
  response.draft.document = extendedDocument();
  spyOn(agentPlatform.getEditor, 'invoke').mockResolvedValue(response);
  const hook = renderHook(() => useGuidPresetCapabilities(personalId, preset.current_stable_revision?.revision_digest));
  await waitFor(() => expect(hook.result.current.isLoading).toBe(false));
  expect(hook.result.current.actionIds.size).toBe(0);
  expect(hook.result.current.requiredResourceKinds.size).toBe(0);
  expect(hook.result.current.idmm).toEqual(createDefaultIdmmConfig());
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
