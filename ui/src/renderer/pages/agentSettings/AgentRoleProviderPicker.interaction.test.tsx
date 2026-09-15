import '../../../../test/setup-dom.ts';
import '@arco-design/web-react/lib/_util/react-19-adapter';
import { cleanup, fireEvent, render, waitFor, within } from '@testing-library/react';
import { afterEach, describe, expect, test } from 'bun:test';
import { createInstance } from 'i18next';
import { useState } from 'react';
import { I18nextProvider, initReactI18next } from 'react-i18next';
import { MemoryRouter } from 'react-router-dom';
import {
  asAgentPresetId, asCapabilityId, asPackageId, asDigestHex, createEmptyAgentPresetDocument,
  type AgentCatalogResponse, type AgentPresetDocument, type AgentPresetDraft, type RoleProviderSelection,
} from '@/common/types/agentPlatform';
import en from '../../services/i18n/locales/en-US/agentSettings.json';
import AgentRoleProviderPicker from './AgentRoleProviderPicker';
import AgentPresetEditor from './AgentPresetEditor';
import OfficialTemplateOverview from './OfficialTemplateOverview';
import { providerSelectionKey, relevantRoleIds, selectRoleProvider } from './roleProviders';

const i18n = createInstance();
await i18n.use(initReactI18next).init({ lng: 'en-US', resources: { 'en-US': { translation: { agentSettings: en } } }, interpolation: { escapeValue: false } });
const capability = { id: asCapabilityId('platform.search'), version: '1.0.0' };
const role = { key: { role_id: 'search', contract_version: '1.0.0' }, contract_digest: asDigestHex('a'.repeat(64)) };
const selection: RoleProviderSelection = { role, provider_mount_id: 'user-search' };
const catalog: AgentCatalogResponse = {
  capabilities: [{ capability, kind: 'tool', display_name: 'Web search', description: '',
    source_package: { id: asPackageId('platform'), version: '1.0.0' }, source_kind: 'bundled',
    materialization_state: 'materialized', supported_surfaces: ['desktop'], required_runtime_features: [],
    required_resource_kinds: [], required_capabilities: [], conflicting_capabilities: [], action_count: 1, context_contributor_count: 0 }],
  skills: [], mcp_tools: [],
  roles: [{ role, capabilities: [capability], providers: [{ selection, display_name: 'User search', description: 'Uses a local index',
    source_package: { id: asPackageId('acme.search'), version: '1.0.0' }, source_kind: 'managed_local', supported_capabilities: [capability] }] }],
};
const initial = (): AgentPresetDocument => ({ ...createEmptyAgentPresetDocument(), enabled_capabilities: [{ capability, action_allowlist: ['search'] }] });
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
afterEach(cleanup);

describe('component implementation selection', () => {
  test('template customization passes the Provider choice to the normal create action', async () => {
    let saved: AgentPresetDocument | undefined;
    const result = render(<I18nextProvider i18n={i18n}><MemoryRouter>
      <OfficialTemplateOverview busy={false} catalog={catalog} template={{
        template_key: 'chat.minimal', immutable: true, forkable: true,
        seed: { enabled_capabilities: [capability], skill_bindings: [], required_resource_kinds: [], required_runtime_features: [] },
        role_coverage: { required_capability_categories: [], required_capability_ids: [], required_resource_kinds: [], required_runtime_features: [] },
      }} onSave={(_name, document) => { saved = document; }} />
    </MemoryRouter></I18nextProvider>);
    const view = within(result.container);
    fireEvent.click(view.getByRole('tab', { name: en.providers.title }));
    fireEvent.click(view.getByRole('combobox', { name: 'Web search' }));
    fireEvent.click(await within(document.body).findByText('User search — acme.search@1.0.0'));
    fireEvent.click(view.getByRole('button', { name: en.workbench.saveAsMine }));
    expect(saved?.system_role_provider_overrides.search).toEqual(selection);
    expect(saved?.enabled_capabilities.map(item => item.capability)).toEqual([capability]);
  });

  test('professional generation preset saves through the existing form without a chat model', () => {
    const original: AgentPresetDraft = { preset_id: asAgentPresetId('0190f5fe-7c00-7a00-8000-000000000001'), display_name: 'Creative', document: createEmptyAgentPresetDocument() };
    original.document.enabled_capabilities = [{ capability: { id: asCapabilityId('creation.music'), version: '1.0.0' }, action_allowlist: [] }];
    const mediaCatalog: AgentCatalogResponse = { ...catalog, capabilities: [{ ...catalog.capabilities[0], capability: original.document.enabled_capabilities[0].capability }] };
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
    fireEvent.click(view.getByRole('combobox', { name: 'Web search' }));
    fireEvent.click(await within(document.body).findByText('User search — acme.search@1.0.0'));
    fireEvent.click(view.getByRole('button', { name: 'common.save' }));
    expect(saved?.document.system_role_provider_overrides.search).toEqual(selection);
    expect(saved?.document.enabled_capabilities).toEqual(original.document.enabled_capabilities);
    expect(original.document.system_role_provider_overrides).toEqual({});
  });

  test('chooses an exact catalog Provider and preserves capability identity through save', async () => {
    const original = initial();
    const view = mount(original);
    fireEvent.click(view.getByRole('combobox', { name: 'Web search' }));
    fireEvent.click(await within(document.body).findByText('User search — acme.search@1.0.0'));
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
    fireEvent.click(view.getByRole('combobox', { name: 'Web search' }));
    const options = await within(document.body).findAllByText(en.providers.inherit);
    fireEvent.click(options[options.length - 1]);
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
    const caller = { id: asCapabilityId('acme.caller'), version: '1.0.0' };
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
