import '../../../../test/setup-dom.ts';
import '@arco-design/web-react/lib/_util/react-19-adapter';
import { cleanup, fireEvent, render, waitFor, within } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, mock, spyOn, test } from 'bun:test';
import { createInstance } from 'i18next';
import { useEffect, useRef, useState } from 'react';
import { I18nextProvider, initReactI18next } from 'react-i18next';
import { MemoryRouter } from 'react-router-dom';
import { SWRConfig } from 'swr';
import { agentPlatform } from '@/common/adapter/ipcBridge';
import { appHosting } from '@/common/adapter/appHostingBridge';
import * as modelProviders from '@/renderer/hooks/agent/useModelProviderList';
import {
  asAgentPresetId, asCapabilityId, asPackageId, asDigestHex, asResolvedSnapshotId, createEmptyAgentPresetDocument,
  type AgentCatalogResponse, type AgentPresetDocument, type AgentPresetDraft, type CapabilityModuleCatalogItem, type RoleProviderSelection,
} from '@/common/types/agentPlatform';
import * as authContext from '@/renderer/hooks/context/AuthContext';
import en from '../../services/i18n/locales/en-US/agentSettings.json';
import AgentRoleProviderPicker from './AgentRoleProviderPicker';
import AgentPresetEditor from './AgentPresetEditor';
import { useAgentSettingsController } from './useAgentSettingsController';
import { providerSelectionKey, relevantRoleIds, selectRoleProvider } from './roleProviders';

const i18n = createInstance();
await i18n.use(initReactI18next).init({ lng: 'en-US', resources: { 'en-US': { translation: { agentSettings: en } } }, interpolation: { escapeValue: false } });
const capability = { id: asCapabilityId('platform.search') };
const role = { key: { role_id: 'search', contract_version: '1.0.0' }, contract_digest: asDigestHex('a'.repeat(64)) };
const selection: RoleProviderSelection = { role, provider_mount_id: 'user-search' };
const capabilityModule: CapabilityModuleCatalogItem = {
  module: capability, display_name: 'Web search', description: '',
  source_package: { id: asPackageId('platform'), version: '1.0.0' }, authoring_policy: 'direct', summary_kind: 'tool',
  actions: [{ action_id: 'platform.search/search', effect_class: 'external_transmit', presentation: 'function_tool' }],
  context_schema_refs: [], event_schema_refs: [], required_resource_kinds: [], required_host_ports: [],
  required_modules: [], conflicting_modules: [], supported_surfaces: ['desktop'],
};
const catalog: AgentCatalogResponse = {
  modules: [capabilityModule],
  capabilities: [{ capability, kind: 'tool', display_name: 'Web search', description: '',
    source_package: { id: asPackageId('platform'), version: '1.0.0' }, source_kind: 'bundled',
    materialization_state: 'materialized', supported_surfaces: ['desktop'], required_runtime_features: [],
    required_resource_kinds: [], required_capabilities: [], conflicting_capabilities: [], action_count: 1, context_contributor_count: 0 }],
  skills: [], mcp_tools: [],
  roles: [{ role, capabilities: [capability], providers: [{ selection, display_name: 'User search', description: 'Uses a local index',
    source_package: { id: asPackageId('acme.search'), version: '1.0.0' }, source_kind: 'managed_local', supported_capabilities: [capability] }] }],
};
const initial = (): AgentPresetDocument => ({ ...createEmptyAgentPresetDocument(), enabled_capabilities: [{ capability, action_allowlist: ['platform.search/search'] }] });
function mount(document = initial(), currentCatalog = catalog, disabled = false) {
  let current = document;
  let saved: AgentPresetDocument | undefined;
  const Harness = () => {
    const [value, setValue] = useState(document);
    return <><AgentRoleProviderPicker document={value} catalog={currentCatalog} disabled={disabled}
      onChange={next => { current = next; setValue(next); }} />
      <button onClick={() => { saved = structuredClone(value); }}>Save</button></>;
  };
  return { ...render(<I18nextProvider i18n={i18n}><Harness /></I18nextProvider>), state: () => current, saved: () => saved };
}
beforeEach(() => {
  spyOn(appHosting.mcpSources, 'invoke').mockResolvedValue([]);
  spyOn(modelProviders, 'useProvidersQuery').mockReturnValue({
    data: [], error: undefined, isLoading: false, isValidating: false, mutate: async () => [],
  });
});
afterEach(() => { cleanup(); mock.restore(); });

describe('component implementation selection', () => {
  test('official editor saves the Provider choice as a revision of the same preset', async () => {
    spyOn(authContext, 'useAuth').mockReturnValue({ user: null } as ReturnType<typeof authContext.useAuth>);
    const original: AgentPresetDraft = { preset_id: asAgentPresetId('0190f5fe-7c00-7a00-8000-000000000021'),
      source_template_key: 'chat.minimal', display_name: 'Official research', document: initial() };
    original.current_revision = { preset_id: original.preset_id, revision: 1, revision_digest: asDigestHex('b'.repeat(64)) };
    original.document.chat_route_records.agent_chat = { schema: 'nomifun.chat-route-record.v1', task: 'agent_chat', failovers: [], primary: {
      model_route_id: 'test-route', model_route_revision: 1, provider_id: 'test-provider', model: 'test-model', protocol: 'openai_chat',
      connection_config_ref: 'test-config', config_revision_digest: asDigestHex('c'.repeat(64)), credential_ref: 'test-credential', features: ['text_input', 'text_output'],
    } };
    const preset = { preset_id: original.preset_id, display_name: original.display_name,
      source: 'official' as const, source_template_key: 'chat.minimal' as const, bound_target_count: 1,
      current_stable_revision: original.current_revision };
    spyOn(agentPlatform.library, 'invoke').mockResolvedValue({ official_templates: [], official_presets: { 'chat.minimal': preset },
      user_presets: [], active_bindings: [], fresh_start: { data_generation: 6, legacy_data_imported: false, official_template_count: 1, user_preset_count: 0 } });
    spyOn(agentPlatform.catalog, 'invoke').mockResolvedValue(catalog);
    spyOn(agentPlatform.getEditor, 'invoke').mockResolvedValue({ preset, draft: original,
      revision: { reference: original.current_revision, document: original.document, created_by: 'owner', created_at_ms: 1 } });
    const save = spyOn(agentPlatform.saveRevision, 'invoke').mockImplementation(async ({ request }) => ({ preset,
      revision: { reference: { ...original.current_revision!, revision: 2 }, document: request.draft.document, created_by: 'owner', created_at_ms: 2 },
      resolved_snapshot_ref: { snapshot_id: asResolvedSnapshotId('0190f5fe-7c00-7a00-8000-000000000022'), snapshot_digest: asDigestHex('d'.repeat(64)) } }));
    const create = spyOn(agentPlatform.createPreset, 'invoke');
    const Harness = () => {
      const settings = useAgentSettingsController();
      const opened = useRef(false);
      useEffect(() => {
        if (!settings.library || opened.current) return;
        opened.current = true;
        void settings.openPreset(preset);
      }, [settings.library, settings.openPreset]);
      if (!settings.editor || !settings.draft) return null;
      return <AgentPresetEditor editor={settings.editor} draft={settings.draft} catalog={settings.catalog}
        busyAction={settings.busyAction} dirty={settings.dirty} onDraftChange={settings.setDraft}
        onSave={() => { void settings.saveRevision(); }} onStartConversation={() => {}} />;
    };
    const result = render(<I18nextProvider i18n={i18n}><SWRConfig value={{ provider: () => new Map() }}><MemoryRouter><Harness />
    </MemoryRouter></SWRConfig></I18nextProvider>);
    const view = within(result.container);
    fireEvent.click(await view.findByRole('tab', { name: en.providers.title }));
    fireEvent.change(view.getByRole('combobox', { name: 'Web search' }), { target: { value: providerSelectionKey(selection) } });
    fireEvent.click(view.getByRole('button', { name: 'common.save' }));
    await waitFor(() => expect(save).toHaveBeenCalledTimes(1));
    const saved = save.mock.calls[0][0];
    expect(saved.preset_id).toBe(original.preset_id);
    expect(saved.request.expected_current_revision).toEqual(original.current_revision);
    expect(saved.request.draft.document.system_role_provider_overrides.search).toEqual(selection);
    expect(saved.request.draft.document.enabled_capabilities).toEqual(original.document.enabled_capabilities);
    expect(original.document.system_role_provider_overrides).toEqual({});
    expect(create).not.toHaveBeenCalled();
  });

  test('professional generation preset saves through the existing form without a chat model', () => {
    const original: AgentPresetDraft = { preset_id: asAgentPresetId('0190f5fe-7c00-7a00-8000-000000000001'), display_name: 'Creative', document: createEmptyAgentPresetDocument() };
    original.document.enabled_capabilities = [
      { capability: { id: asCapabilityId('creation.media') }, action_allowlist: ['creation.media/music'] },
      { capability: { id: asCapabilityId('workspace.files') }, action_allowlist: ['workspace.files/read'] },
    ];
    const mediaModule: CapabilityModuleCatalogItem = { ...capabilityModule,
      module: original.document.enabled_capabilities[0].capability, display_name: 'Media creation',
      actions: [{ action_id: 'creation.media/music', effect_class: 'external_transmit', presentation: 'function_tool' }] };
    const workspaceModule: CapabilityModuleCatalogItem = { ...capabilityModule,
      module: original.document.enabled_capabilities[1].capability, display_name: 'Workspace files',
      actions: [{ action_id: 'workspace.files/read', effect_class: 'read_local', presentation: 'function_tool' }] };
    const mediaCatalog: AgentCatalogResponse = { ...catalog, modules: [mediaModule, workspaceModule], capabilities: [
      { ...catalog.capabilities[0], capability: mediaModule.module },
      { ...catalog.capabilities[0], capability: workspaceModule.module },
    ] };
    let saved = false;
    const result = render(<I18nextProvider i18n={i18n}><MemoryRouter>
      <AgentPresetEditor editor={{ preset: { preset_id: original.preset_id, display_name: original.display_name, source: 'user', bound_target_count: 0 }, draft: original }}
        draft={original} catalog={mediaCatalog} busyAction={null} dirty={true}
        onDraftChange={() => {}} onSave={() => { saved = true; }} onStartConversation={() => {}} />
    </MemoryRouter></I18nextProvider>);
    const button = within(result.container).getByRole('button', { name: 'common.save' });
    expect((button as HTMLButtonElement).disabled).toBe(false);
    fireEvent.click(button);
    expect(saved).toBe(true);
    expect(original.document.chat_route_records).toEqual({});
  });

  test('personal editor passes the selected implementation in its existing save draft', async () => {
    const original: AgentPresetDraft = { preset_id: asAgentPresetId('0190f5fe-7c00-7a00-8000-000000000001'), display_name: 'Research', document: initial() };
    original.document.chat_route_records.agent_chat = { schema: 'nomifun.chat-route-record.v1', task: 'agent_chat', failovers: [], primary: {
      model_route_id: 'test-route', model_route_revision: 1, provider_id: 'test-provider', model: 'test-model', protocol: 'openai_chat',
      connection_config_ref: 'test-config', config_revision_digest: asDigestHex('c'.repeat(64)), credential_ref: 'test-credential', features: ['text_input', 'text_output'],
    } };
    let saved: AgentPresetDraft | undefined;
    const Harness = () => {
      const [draft, setDraft] = useState(original);
      return <AgentPresetEditor editor={{ preset: { preset_id: draft.preset_id, display_name: draft.display_name, source: 'user', bound_target_count: 0 }, draft: original }}
        draft={draft} catalog={catalog} busyAction={null} dirty={draft !== original}
        onDraftChange={setDraft} onSave={() => { saved = draft; }} onStartConversation={() => {}} />;
    };
    const result = render(<I18nextProvider i18n={i18n}><MemoryRouter><Harness /></MemoryRouter></I18nextProvider>);
    const view = within(result.container);
    fireEvent.click(view.getByRole('tab', { name: en.providers.title }));
    fireEvent.change(view.getByRole('combobox', { name: 'Web search' }), { target: { value: providerSelectionKey(selection) } });
    fireEvent.click(view.getByRole('button', { name: 'common.save' }));
    expect(saved?.document.system_role_provider_overrides.search).toEqual(selection);
    expect(saved?.document.enabled_capabilities).toEqual(original.document.enabled_capabilities);
    expect(original.document.system_role_provider_overrides).toEqual({});
  });

  test('chooses an exact catalog Provider and preserves capability identity through save', async () => {
    const original = initial();
    const view = mount(original);
    fireEvent.change(view.getByRole('combobox', { name: 'Web search' }), { target: { value: providerSelectionKey(selection) } });
    await waitFor(() => expect(view.state().system_role_provider_overrides.search).toEqual(selection));
    expect(view.state().enabled_capabilities).toEqual(original.enabled_capabilities);
    expect(original.system_role_provider_overrides).toEqual({});
    fireEvent.click(view.getByRole('button', { name: 'Save' }));
    expect(view.saved()?.system_role_provider_overrides.search).toEqual(selection);
  });

  test('keeps a withdrawn selection visible and resets only that explicit override', async () => {
    const old = { ...selection, provider_mount_id: 'withdrawn' };
    const unrelated = { ...selection, role: { ...role, key: { ...role.key, role_id: 'other' } } };
    const original = initial();
    original.system_role_provider_overrides = { search: old, other: unrelated };
    const view = mount(original);
    expect(within(view.container).getAllByRole('alert')).toHaveLength(2);
    expect(view.state().system_role_provider_overrides.search).toEqual(old);
    fireEvent.change(view.getByRole('combobox', { name: 'Web search' }), { target: { value: '' } });
    await waitFor(() => expect(view.state().system_role_provider_overrides).toEqual({ other: unrelated }));
    expect(original.system_role_provider_overrides.search).toEqual(old);
  });

  test('a changed contract is not silently selected even when the Mount is unchanged', () => {
    const original = selectRoleProvider(initial(), 'search', { ...selection, role: { ...role, contract_digest: asDigestHex('b'.repeat(64)) } });
    const view = mount(original);
    expect(within(view.container).getByRole('alert').textContent).toBe(en.providers.missingHint);
    expect(view.state()).toBe(original);
    expect(providerSelectionKey(selection)).not.toBe(providerSelectionKey(original.system_role_provider_overrides.search));
  });

  test('shows dependency Roles without adding tools, and retains missing Role choices', () => {
    const caller = { id: asCapabilityId('acme.caller') };
    const otherCatalog = structuredClone(catalog);
    otherCatalog.capabilities.push({ ...catalog.capabilities[0], capability: caller, required_capabilities: [capability] });
    otherCatalog.capabilities[0].required_capabilities = [caller]; // visibility traversal terminates on cycles
    const document = initial();
    document.enabled_capabilities = [{ capability: caller, action_allowlist: [] }];
    expect(relevantRoleIds(document, otherCatalog)).toEqual(['search']);
    expect(document.enabled_capabilities).toHaveLength(1);
    const withdrawn = selectRoleProvider(createEmptyAgentPresetDocument(), 'search', selection);
    expect(relevantRoleIds(withdrawn, { ...catalog, roles: [] })).toEqual(['search']);
    expect(() => selectRoleProvider(document, 'other', selection)).toThrow('another role');
  });
});
