import '../../../../test/setup-dom.ts';
import '@arco-design/web-react/lib/_util/react-19-adapter';
import { act, cleanup, fireEvent, render, waitFor, within } from '@testing-library/react';
import { afterEach, beforeEach, expect, mock, spyOn, test } from 'bun:test';
import { useEffect } from 'react';
import { createInstance } from 'i18next';
import { I18nextProvider, initReactI18next } from 'react-i18next';
import { MemoryRouter, Route, Routes, useLocation, useNavigate } from 'react-router-dom';
import { SWRConfig } from 'swr';
import { NavigationHistoryProvider, useNavigationHistory } from '@/renderer/hooks/context/NavigationHistoryContext';
import { AGENT_SIDER_TOGGLE_EVENT } from '@/renderer/utils/workspace/agentSiderEvents';
import { agentPlatform } from '@/common/adapter/ipcBridge';
import { useAgentPresets } from '@/renderer/hooks/agent/useAgentPresets';

import { AGENT_CHAT_MODEL_TASK, CHAT_ROUTE_RECORD_SCHEMA, OFFICIAL_PRESET_KEYS, asAgentPresetId, asCapabilityId, asPackageId, asDigestHex, asResolvedSnapshotId, createEmptyAgentPresetDocument, type AgentCatalogResponse, type AgentPresetEditorResponse,
  type AgentPresetLibraryResponse, type CapabilityCatalogItem, type CapabilityModuleCatalogItem, type ChatRouteRecord, type OfficialPresetTemplate } from '@/common/types/agentPlatform';
import * as roleDefaults from './AgentRoleDefaults';
import * as libraryPanel from './AgentPresetLibrary';
import * as authContext from '@/renderer/hooks/context/AuthContext';
import * as modelProviders from '@/renderer/hooks/agent/useModelProviderList';
import { setBrowserStorageGeneration } from '@/common/utils/browserStorageKey';
const APP_HOSTING_MODULE = 'app.hosting';
const APP_HOSTING_ACTIONS = ['app.hosting/list', 'app.hosting/get', 'app.hosting/update'];
import { agentEditorReturn, editingDocument, isAgentModelConfigurationMissing, TEMPLATE_I18N_PATH } from './model';
import { AgentEditorDrafts } from './agentEditorDrafts';
import AgentSettingsPage from './AgentSettingsPage';
import en from '../../services/i18n/locales/en-US/agentSettings.json';
import common from '../../services/i18n/locales/en-US/common.json';

const i18n = createInstance();
await i18n.use(initReactI18next).init({ lng: 'en-US', resources: { 'en-US': { translation: { agentSettings: en, common } } }, interpolation: { escapeValue: false } });
const template: OfficialPresetTemplate = { template_key: 'chat.minimal', immutable: true, forkable: true,
  seed: { enabled_capabilities: [], skill_bindings: [], required_resource_kinds: [], required_runtime_features: [] },
  role_coverage: { required_capability_categories: [], required_capability_ids: [], required_runtime_features: [], required_resource_kinds: [] } };
const capability: CapabilityCatalogItem = { capability: { id: asCapabilityId('business.check') }, kind: 'tool',
  display_name: 'Business check', description: 'Check business input', source_package: { id: asPackageId('test'), version: '1.0.0' },
  source_kind: 'managed_local', materialization_state: 'materialized', supported_surfaces: ['desktop'], required_runtime_features: [],
  required_resource_kinds: [], required_capabilities: [], conflicting_capabilities: [], action_count: 1, context_contributor_count: 0 };
const capabilityModule: CapabilityModuleCatalogItem = {
  module: capability.capability, display_name: capability.display_name, description: capability.description,
  source_package: capability.source_package, authoring_policy: 'direct', summary_kind: 'tool',
  actions: [{ action_id: 'business.check/run', input_schema: 'input', output_schema: 'output', effect_class: 'pure', presentation: 'function_tool' }],
  context_schema_refs: [], event_schema_refs: [], required_resource_kinds: [], required_host_ports: [],
  required_modules: [], conflicting_modules: [], supported_surfaces: ['desktop'],
};
const companionCoreModules = ['companion', 'companion.memory'].map(id => ({
  ...capabilityModule, module: { id: asCapabilityId(id) }, display_name: id,
  actions: [{ ...capabilityModule.actions[0], action_id: `${id}/run` }],
}));
const companionCoreCapabilities = companionCoreModules.map(module => ({
  ...capability, capability: module.module, display_name: module.display_name, source_kind: 'bundled',
}));
const companionCoreGrants = companionCoreModules.map(module => ({
  capability: module.module, action_allowlist: module.actions.map(action => action.action_id),
}));
const presetId = asAgentPresetId('0190f5fe-7c00-7a00-8000-000000000101');
const revision = { preset_id: presetId, revision: 1, revision_digest: asDigestHex('b'.repeat(64)) };
const modelRecord: ChatRouteRecord = {
  schema: CHAT_ROUTE_RECORD_SCHEMA, task: AGENT_CHAT_MODEL_TASK,
  primary: { model_route_id: 'chat-route', model_route_revision: 1, provider_id: 'test-provider', model: 'test-model',
    protocol: 'openai_chat', connection_config_ref: 'config-ref', credential_ref: 'credential-ref',
    config_revision_digest: asDigestHex('a'.repeat(64)), features: ['text_input', 'text_output', 'tool_calls'] },
  failovers: [],
};
const documentWithModel = () => {
  const document = createEmptyAgentPresetDocument();
  document.model_route_refs[AGENT_CHAT_MODEL_TASK] = modelRecord.primary.model_route_id;
  document.chat_route_records[AGENT_CHAT_MODEL_TASK] = structuredClone(modelRecord);
  return document;
};
beforeEach(() => {
  localStorage.clear();
  setBrowserStorageGeneration('01900000-0000-7000-8000-000000000001');
});
afterEach(() => {
  delete (window as typeof window & { __backendPort?: number }).__backendPort;
  cleanup();
  mock.restore();
});

async function mount(error?: unknown, initialEditor?: AgentPresetEditorResponse,
  options: { entry?: string; expectedHeading?: string; templates?: OfficialPresetTemplate[]; roles?: AgentCatalogResponse['roles'];
    missingModel?: boolean; noRevision?: boolean; rawOfficialName?: boolean } = {}) {
  spyOn(authContext, 'useAuth').mockReturnValue({ user: { id: '01900000-0000-7000-8000-000000000071', username: 'owner' } } as ReturnType<typeof authContext.useAuth>);
  spyOn(modelProviders, 'useProvidersQuery').mockReturnValue({ data: [], error: undefined, isLoading: false,
    isValidating: false, mutate: async () => [] });
  spyOn(roleDefaults, 'default').mockImplementation(() => <div>Default roles</div>);
  spyOn(libraryPanel, 'default').mockImplementation(({ library, dirtyPresetId, onSelectTemplate }) => <nav>
    {library.official_templates.map(value => <button key={value.template_key} onClick={() => onSelectTemplate(value)}>{value.template_key}</button>)}
    {[...Object.values(library.official_presets), ...library.user_presets].map(value => <output key={value.preset_id} data-testid={`library-preset-${value.preset_id}`}>
      {value.display_name}|{dirtyPresetId === value.preset_id ? 'dirty' : 'saved'}
    </output>)}
  </nav>);
  let editor: AgentPresetEditorResponse | undefined = initialEditor;
  const templates = OFFICIAL_PRESET_KEYS.map(templateKey => options.templates?.find(value => value.template_key === templateKey)
    ?? ({ ...template, template_key: templateKey, seed: { ...template.seed,
      enabled_capabilities: templateKey === 'companion.default' ? companionCoreGrants : [],
    } } as OfficialPresetTemplate));
  const officialEditors = new Map<string, AgentPresetEditorResponse>();
  templates.forEach((officialTemplate, index) => {
    const id = asAgentPresetId(`0190f5fe-7c00-7a00-8000-00000000020${index}`);
    const currentRevision = options.noRevision ? undefined : { ...revision, preset_id: id };
    const document = options.missingModel ? createEmptyAgentPresetDocument() : documentWithModel();
    document.enabled_capabilities = structuredClone(officialTemplate.seed.enabled_capabilities);
    const displayName = options.rawOfficialName ? officialTemplate.template_key
      : i18n.t(`agentSettings.template.${TEMPLATE_I18N_PATH[officialTemplate.template_key]}.name`);
    officialEditors.set(id, { preset: { preset_id: id, source: 'official',
      display_name: displayName, bound_target_count: 1, current_stable_revision: currentRevision },
      draft: { preset_id: id, source_template_key: officialTemplate.template_key, display_name: displayName, document, current_revision: currentRevision },
      revision: currentRevision ? { reference: currentRevision, document, created_by: 'owner', created_at_ms: 1 } : undefined });
  });
  const officialId = (key: OfficialPresetTemplate['template_key']) => [...officialEditors.values()].find(value => value.draft.source_template_key === key)!.preset.preset_id;
  const library = spyOn(agentPlatform.library, 'invoke').mockImplementation(async () => ({
    official_templates: templates, official_presets: Object.fromEntries([...officialEditors.values()].map(value => [value.draft.source_template_key, value.preset])),
    user_presets: editor ? [editor.preset] : [], active_bindings: [],
    fresh_start: { data_generation: 6, legacy_data_imported: false, official_template_count: 6, user_preset_count: editor ? 1 : 0 },
  } as AgentPresetLibraryResponse));
  const appModule = { ...capabilityModule, module: { id: asCapabilityId(APP_HOSTING_MODULE) }, display_name: 'NomiFun app development',
    actions: APP_HOSTING_ACTIONS.map(action_id => ({ ...capabilityModule.actions[0], action_id })) };
  const appCapability = { ...capability, capability: appModule.module, display_name: appModule.display_name, action_count: appModule.actions.length };
  spyOn(agentPlatform.catalog, 'invoke').mockResolvedValue({ modules: [capabilityModule, appModule, ...companionCoreModules],
    capabilities: [capability, appCapability, ...companionCoreCapabilities], skills: [], mcp_tools: [], roles: options.roles ?? [] });
  const create = spyOn(agentPlatform.createPreset, 'invoke').mockImplementation(async request => {
    editor = { preset: { preset_id: presetId, source: 'user', display_name: request.display_name, bound_target_count: 0, current_stable_revision: revision },
      draft: { preset_id: presetId, display_name: request.display_name, document: request.document!, current_revision: revision },
      revision: { reference: revision, document: request.document!, created_by: 'owner', created_at_ms: 1 } };
    return editor;
  });
  const save = spyOn(agentPlatform.saveRevision, 'invoke').mockImplementation(async ({ preset_id, request }) => {
    const previous = officialEditors.get(preset_id) ?? editor!;
    const nextRevision = { ...revision, preset_id: asAgentPresetId(preset_id), ...previous.draft.current_revision,
      revision: (previous.draft.current_revision?.revision ?? 0) + 1 };
    const next: AgentPresetEditorResponse = { preset: { ...previous.preset, display_name: request.draft.display_name,
      description: request.draft.description, current_stable_revision: nextRevision },
      draft: { ...request.draft, current_revision: nextRevision },
      revision: { reference: nextRevision, document: request.draft.document, created_by: 'owner', created_at_ms: 2 } };
    if (next.preset.source === 'official') officialEditors.set(preset_id, next);
    else editor = next;
    return { preset: next.preset, revision: next.revision!, resolved_snapshot_ref: {
      snapshot_id: asResolvedSnapshotId('0190f5fe-7c00-7a00-8000-000000000199'), snapshot_digest: asDigestHex('c'.repeat(64)) } };
  });
  if (error) save.mockRejectedValueOnce(error);
  const getEditor = spyOn(agentPlatform.getEditor, 'invoke').mockImplementation(async request => officialEditors.get(request.preset_id) ?? editor!);
  const turn = spyOn(agentPlatform.sessions.createTurn, 'invoke');
  const states: unknown[] = [];
  const Probe = () => { const location = useLocation(); useEffect(() => { states.push(location.state); }, [location]); return null; };
  const Models = () => { const navigate = useNavigate(), history = useNavigationHistory()!; return <div><h1>Model management</h1>
    <button onClick={history.back}>Back to editing</button><button onClick={() => navigate('/agent?template=coding.codex')}>Other template</button>
  </div>; };
  const HistoryControls = () => { const history = useNavigationHistory()!; return <button disabled={!history.canForward} onClick={history.forward}>App forward</button>; };
  const SidebarControls = () => { const navigate = useNavigate(); return <nav><button onClick={() => navigate('/models')}>Sidebar models</button>
    <button onClick={() => navigate('/agent')}>Sidebar Agent</button></nav>; };
  const PresetCacheProbe = () => {
    const { presets } = useAgentPresets();
    return <output data-testid='preset-cache-name'>{presets[0]?.display_name ?? ''}</output>;
  };
  const view = render(<I18nextProvider i18n={i18n}><SWRConfig value={{ provider: () => new Map(), revalidateOnMount: false,
    fallback: { providers: [] } }}><PresetCacheProbe /><MemoryRouter initialEntries={[options.entry ?? (initialEditor ? `/agent?preset=${presetId}` : '/agent?template=chat.minimal')]}>
    <NavigationHistoryProvider><Probe /><HistoryControls /><SidebarControls /><Routes><Route path='/agent' element={<AgentSettingsPage />} /><Route path='/models' element={<Models />} />
      <Route path='/guid' element={<h1>New conversation</h1>} />
      <Route path='/nomi' element={<h1>Desktop companions</h1>} />
      <Route path='/customer-service' element={<h1>Customer service</h1>} />
      </Routes></NavigationHistoryProvider>
  </MemoryRouter></SWRConfig></I18nextProvider>);
  if (initialEditor) await view.findByRole('heading', { name: options.expectedHeading ?? initialEditor.preset.display_name, level: 2 });
  else {
    const entryTemplate = templates.find(value => options.entry?.includes(value.template_key)) ?? template;
    await view.findByRole('heading', { name: options.expectedHeading ?? i18n.t(`agentSettings.template.${TEMPLATE_I18N_PATH[entryTemplate.template_key]}.name`), level: 2 });
  }
  return { ...view, create, save, turn, states, library, getEditor, officialEditors, officialId };
}

test('starting a conversation from an official Agent preserves its template selection key', async () => {
  const v = await mount(undefined, undefined, { entry: '/agent?template=coding.codex' });
  fireEvent.click(v.getByRole('button', { name: en.actions.startConversation }));
  await v.findByRole('heading', { name: 'New conversation' });
  expect(v.states.at(-1)).toEqual({ selectedAgentTemplateKey: 'coding.codex' });
  expect(v.create).not.toHaveBeenCalled();
  expect(v.turn).not.toHaveBeenCalled();
});

test('starting a conversation from a personal Agent preserves its preset ID', async () => {
  const document = documentWithModel();
  const original: AgentPresetEditorResponse = {
    preset: { preset_id: presetId, source: 'user', display_name: 'Saved Agent', bound_target_count: 0, current_stable_revision: revision },
    draft: { preset_id: presetId, display_name: 'Saved Agent', document, current_revision: revision },
    revision: { reference: revision, document, created_by: 'owner', created_at_ms: 1 },
  };
  const v = await mount(undefined, original);
  fireEvent.click(v.getByRole('button', { name: en.actions.startConversation }));
  await v.findByRole('heading', { name: 'New conversation' });
  expect(v.states.at(-1)).toEqual({ selectedAgentPresetId: presetId });
  expect(v.create).not.toHaveBeenCalled();
  expect(v.turn).not.toHaveBeenCalled();
});

test.each([
  ['companion.default', en.workbench.openCompanions, 'Desktop companions'],
  ['customer-service.default', en.workbench.openCustomerService, 'Customer service'],
] as const)('official %s opens its fixed product workspace', async (templateKey, label, heading) => {
  const v = await mount(undefined, undefined, { entry: `/agent?template=${templateKey}` });
  fireEvent.click(v.getByRole('button', { name: label }));
  await v.findByRole('heading', { name: heading });
  expect(v.states.at(-1)).toBeNull();
  expect(v.create).not.toHaveBeenCalled();
  expect(v.turn).not.toHaveBeenCalled();
});

test('untouched official Agents without a saved revision open with localized names and switch without a discard warning', async () => {
  const v = await mount(undefined, undefined, { entry: '/agent', noRevision: true, rawOfficialName: true });
  expect(v.getByRole('heading', { name: en.template.chat.minimal.name, level: 2 })).toBeTruthy();
  expect((v.getByRole('button', { name: common.save }) as HTMLButtonElement).disabled).toBe(false);
  expect(v.queryByRole('button', { name: en.workbench.resetChanges })).toBeNull();

  if (!v.queryByRole('button', { name: 'coding.codex' })) {
    await act(async () => { window.dispatchEvent(new Event(AGENT_SIDER_TOGGLE_EVENT)); });
  }
  fireEvent.click(v.getByRole('button', { name: 'coding.codex' }));
  await v.findByRole('heading', { name: en.template.coding.codex.name, level: 2 });
  expect(v.queryByRole('dialog', { name: en.workbench.leaveTitle })).toBeNull();
  expect(v.queryByRole('button', { name: en.workbench.resetChanges })).toBeNull();
  expect((v.getByRole('button', { name: common.save }) as HTMLButtonElement).disabled).toBe(false);
  expect(v.save).not.toHaveBeenCalled();
  expect(v.create).not.toHaveBeenCalled();
});

test('an official Agent keeps its ID, unsaved name and exact Module grant through model management and a single explicit save', async () => {
  const v = await mount(undefined, undefined, { missingModel: true });
  const officialPresetId = v.officialId('chat.minimal');
  fireEvent.click(v.getByRole('button', { name: en.workbench.editName }));
  const name = v.getByRole('textbox', { name: en.fields.name });
  fireEvent.input(name, { target: { value: 'My inspection Agent' } });
  fireEvent.keyDown(name, { key: 'Enter' });
  fireEvent.click(v.getByRole('switch', { name: 'Enable Business check' }));
  expect((v.getByRole('button', { name: common.save }) as HTMLButtonElement).disabled).toBe(true);
  fireEvent.click(v.getByRole('button', { name: en.workbench.configureChatModel }));
  await v.findByRole('heading', { name: 'Model management' });
  expect(v.create).not.toHaveBeenCalled();
  expect(v.save).not.toHaveBeenCalled();
  const previous = v.officialEditors.get(officialPresetId)!;
  const document = documentWithModel();
  v.officialEditors.set(officialPresetId, { ...previous, draft: { ...previous.draft, document },
    revision: { ...previous.revision!, document } });
  fireEvent.click(v.getByRole('button', { name: 'Back to editing' }));
  await v.findByRole('heading', { name: 'My inspection Agent' });
  const snapshot = v.states.find(value => value && typeof value === 'object' && 'agentEditorReturn' in value) as { agentEditorReturn: { kind: string; draft: { preset_id: string; document: Record<string, unknown> } } };
  expect(snapshot.agentEditorReturn.kind).toBe('preset');
  expect(snapshot.agentEditorReturn.draft.preset_id).toBe(officialPresetId);
  expect(snapshot.agentEditorReturn.draft.document.model_route_refs).toBeUndefined();
  expect(snapshot.agentEditorReturn.draft.document.chat_route_records).toBeUndefined();
  expect(v.getByRole('tab', { name: en.workbench.capabilityTab }).getAttribute('aria-selected')).toBe('true');
  expect(v.queryByRole('combobox')).toBeNull();
  expect(v.create).not.toHaveBeenCalled();
  fireEvent.click(v.getByRole('tab', { name: en.workbench.capabilityTab }));
  expect(v.getByRole('switch', { name: 'Disable Business check' })).toBeTruthy();
  await act(async () => { fireEvent.click(v.getByRole('button', { name: common.save })); });
  expect(v.create).not.toHaveBeenCalled();
  expect(v.save).toHaveBeenCalledTimes(1);
  const request = v.save.mock.calls[0][0];
  expect(request.preset_id).toBe(officialPresetId);
  expect(request.request.draft.preset_id).toBe(officialPresetId);
  expect(request.request.draft.display_name).toBe('My inspection Agent');
  expect(request.request.draft.document.enabled_capabilities).toEqual([{ capability: capability.capability, action_allowlist: ['business.check/run'] }]);
  expect(request.request.draft.document.chat_route_records[AGENT_CHAT_MODEL_TASK]?.primary.model).toBe('test-model');
  expect(v.turn).not.toHaveBeenCalled();
});

test('normal navigation to a different template does not restore the previous unsaved input', async () => {
  const v = await mount(undefined, undefined, { missingModel: true });
  fireEvent.click(v.getByRole('button', { name: en.workbench.editName }));
  const name = v.getByRole('textbox', { name: en.fields.name });
  fireEvent.input(name, { target: { value: 'Do not copy me' } });
  fireEvent.keyDown(name, { key: 'Enter' });
  await v.findByRole('button', { name: en.workbench.configureChatModel });
  fireEvent.click(v.getByRole('button', { name: en.workbench.configureChatModel }));
  await v.findByRole('heading', { name: 'Model management' });
  fireEvent.click(v.getByRole('button', { name: 'Other template' }));
  await v.findByRole('heading', { name: en.template.coding.codex.name });
  expect(v.create).not.toHaveBeenCalled();
  expect(v.save).not.toHaveBeenCalled();
});

test('the bound official companion Agent saves added capabilities directly without creating a copy', async () => {
  const v = await mount(undefined, undefined, { entry: '/agent?template=companion.default', expectedHeading: en.template.companion.default.name });
  const officialPresetId = v.officialId('companion.default');
  fireEvent.click(v.getByRole('switch', { name: 'Enable Business check' }));
  await act(async () => { fireEvent.click(v.getByRole('button', { name: common.save })); });
  expect(v.save).toHaveBeenCalledTimes(1);
  expect(v.save.mock.calls[0][0].preset_id).toBe(officialPresetId);
  expect(v.save.mock.calls[0][0].request.expected_current_revision?.preset_id).toBe(officialPresetId);
  expect(v.officialEditors.get(officialPresetId)?.preset.bound_target_count).toBe(1);
  expect(v.officialEditors.get(officialPresetId)?.draft.document.enabled_capabilities).toEqual([
    { capability: capability.capability, action_allowlist: ['business.check/run'] }, ...companionCoreGrants,
  ]);
  expect(v.create).not.toHaveBeenCalled();
  expect(v.turn).not.toHaveBeenCalled();
  fireEvent.click(v.getByRole('button', { name: 'Sidebar models' }));
  await v.findByRole('heading', { name: 'Model management' });
  fireEvent.click(v.getByRole('button', { name: 'Sidebar Agent' }));
  await v.findByRole('heading', { name: en.template.companion.default.name, level: 2 });
  expect(v.getByRole('switch', { name: 'Disable Business check' })).toBeTruthy();
});

test('the official companion Agent explains and blocks removal of either required core capability', async () => {
  const v = await mount(undefined, undefined, { entry: '/agent?template=companion.default' });
  fireEvent.click(v.getByRole('switch', { name: 'Enable Business check' }));
  expect((v.getByRole('button', { name: common.save }) as HTMLButtonElement).disabled).toBe(false);

  for (const name of [en.modules.companion.name, en.modules.companionMemory.name]) {
    fireEvent.click(v.getByRole('switch', { name: `Disable ${name}` }));
    const save = v.getByRole('button', { name: common.save }) as HTMLButtonElement;
    expect(save.disabled).toBe(true);
    expect(v.getByText(en.workbench.companionCoreNeeded)).toBeTruthy();
    fireEvent.click(save);
    expect(v.save).not.toHaveBeenCalled();
    fireEvent.click(v.getByRole('switch', { name: `Enable ${name}` }));
    expect((v.getByRole('button', { name: common.save }) as HTMLButtonElement).disabled).toBe(false);
    expect(v.queryByText(en.workbench.companionCoreNeeded)).toBeNull();
  }

  expect(v.create).not.toHaveBeenCalled();
  expect(v.turn).not.toHaveBeenCalled();
});

test('creating an explicit personal copy keeps official edits separate and preserves the official draft', async () => {
  const v = await mount();
  const officialPresetId = v.officialId('chat.minimal');
  fireEvent.click(v.getByRole('button', { name: en.workbench.editName }));
  const name = v.getByRole('textbox', { name: en.fields.name });
  fireEvent.input(name, { target: { value: 'Personal inspection Agent' } });
  fireEvent.keyDown(name, { key: 'Enter' });
  fireEvent.click(v.getByRole('switch', { name: 'Enable Business check' }));
  await act(async () => { fireEvent.click(v.getByRole('button', { name: en.workbench.createCopy })); });
  expect(v.create).toHaveBeenCalledTimes(1);
  expect(v.create.mock.calls[0][0].display_name).toBe('Personal inspection Agent');
  expect(v.create.mock.calls[0][0].document?.enabled_capabilities).toEqual([{ capability: capability.capability, action_allowlist: ['business.check/run'] }]);
  expect(v.officialEditors.get(officialPresetId)?.preset.display_name).toBe(en.template.chat.minimal.name);
  expect(v.officialEditors.get(officialPresetId)?.draft.document.enabled_capabilities).toEqual([]);
  expect(v.queryByRole('button', { name: en.workbench.createCopy })).toBeNull();
  expect(v.save).not.toHaveBeenCalled();
  await act(async () => { window.dispatchEvent(new Event(AGENT_SIDER_TOGGLE_EVENT)); });
  fireEvent.click(v.getByRole('button', { name: 'chat.minimal' }));
  await v.findByRole('button', { name: en.workbench.createCopy });
  expect(v.getByRole('heading', { name: 'Personal inspection Agent' })).toBeTruthy();
  expect(v.getByRole('switch', { name: 'Disable Business check' })).toBeTruthy();
  fireEvent.click(v.getByRole('button', { name: en.workbench.resetChanges }));
  await v.findByRole('heading', { name: en.template.chat.minimal.name });
  expect(v.getByRole('switch', { name: 'Enable Business check' })).toBeTruthy();
});

test('personal Agent header exposes name and description editing and saves metadata changes', async () => {
  const document = createEmptyAgentPresetDocument();
  const original: AgentPresetEditorResponse = {
    preset: {
      preset_id: presetId,
      source: 'user',
      display_name: 'Saved Agent',
      description: 'Old description',
      bound_target_count: 0,
      current_stable_revision: revision,
    },
    draft: {
      preset_id: presetId,
      display_name: 'Saved Agent',
      description: 'Old description',
      document,
      current_revision: revision,
    },
    revision: { reference: revision, document, created_by: 'owner', created_at_ms: 1 },
  };
  const v = await mount(undefined, original);
  v.save.mockImplementation(async ({ request }) => ({
    preset: {
      ...original.preset,
      display_name: request.draft.display_name,
      description: request.draft.description,
    },
    revision: original.revision!,
    resolved_snapshot_ref: {
      snapshot_id: asResolvedSnapshotId('0190f5fe-7c00-7a00-8000-000000000199'),
      snapshot_digest: asDigestHex('c'.repeat(64)),
    },
  }));

  fireEvent.click(v.getByRole('button', { name: en.workbench.editName }));
  const name = v.getByRole('textbox', { name: en.fields.name }) as HTMLInputElement;
  fireEvent.input(name, { target: { value: 'Renamed Agent' } });
  fireEvent.keyDown(name, { key: 'Enter' });
  expect(v.getByRole('heading', { name: 'Renamed Agent' })).toBeTruthy();
  await act(async () => { window.dispatchEvent(new Event(AGENT_SIDER_TOGGLE_EVENT)); });
  expect(v.getByTestId(`library-preset-${presetId}`).textContent).toBe('Renamed Agent|dirty');
  await act(async () => { window.dispatchEvent(new Event(AGENT_SIDER_TOGGLE_EVENT)); });
  fireEvent.click(v.getByRole('button', { name: en.workbench.editIdentity }));
  expect(v.getByRole('tab', { name: en.workbench.settingsTab }).getAttribute('aria-selected')).toBe('true');
  fireEvent.input(v.getByRole('textbox', { name: en.fields.description }), {
    target: { value: 'Updated description' },
  });
  await act(async () => { fireEvent.click(v.getByRole('button', { name: common.save })); });
  expect(v.save).toHaveBeenCalledTimes(1);
  expect(v.save.mock.calls[0][0].request.draft.display_name).toBe('Renamed Agent');
  expect(v.save.mock.calls[0][0].request.draft.description).toBe('Updated description');
  await act(async () => { window.dispatchEvent(new Event(AGENT_SIDER_TOGGLE_EVENT)); });
  await waitFor(() => expect(v.getByTestId(`library-preset-${presetId}`).textContent).toBe('Renamed Agent|saved'));
  expect(v.getByTestId('preset-cache-name').textContent).toBe('Renamed Agent');
});

test('compact editor responses can open the component implementation tab', async () => {
  const document = createEmptyAgentPresetDocument();
  document.enabled_capabilities = [{ capability: capability.capability, action_allowlist: [] }];
  delete (document as Partial<typeof document>).system_role_provider_overrides;
  const original: AgentPresetEditorResponse = {
    preset: {
      preset_id: presetId,
      source: 'user',
      display_name: 'Compact Agent',
      bound_target_count: 0,
      current_stable_revision: revision,
    },
    draft: {
      preset_id: presetId,
      display_name: 'Compact Agent',
      document,
      current_revision: revision,
    },
    revision: { reference: revision, document, created_by: 'owner', created_at_ms: 1 },
  };
  const role = {
    key: { role_id: 'business.check', contract_version: '1.0.0' },
    contract_digest: asDigestHex('d'.repeat(64)),
  };
  new AgentEditorDrafts('01900000-0000-7000-8000-000000000071').writePreset({
    ...original.draft,
    display_name: 'Restored Compact Agent',
  });
  const v = await mount(undefined, original, {
    expectedHeading: 'Restored Compact Agent',
    roles: [{ role, capabilities: [capability.capability], providers: [] }],
  });

  fireEvent.click(v.getByRole('tab', { name: en.providers.title }));

  const panel = v.getByRole('tabpanel', { name: en.providers.title });
  expect(panel.querySelector('select')?.getAttribute('aria-label')).toBe(capability.display_name);
});

test('unrelated failures never offer model configuration as the recovery action', async () => {
  const v = await mount({ code: 'CAPABILITY_UNAVAILABLE', status: 400 });
  fireEvent.click(v.getByRole('switch', { name: 'Enable Business check' }));
  fireEvent.click(v.getByRole('button', { name: common.save }));
  await waitFor(() => expect(v.save).toHaveBeenCalledTimes(1));
  expect(v.queryByRole('button', { name: en.workbench.configureChatModel }) === null).toBe(true);
  expect(isAgentModelConfigurationMissing({ code: 'MODEL_ROUTE_NOT_CONFIGURED' })).toBe(true);
  expect(isAgentModelConfigurationMissing({ code: 'CAPABILITY_UNAVAILABLE' })).toBe(false);
});

test('preset return snapshots exclude model configuration and cannot cross a changed template or preset query', () => {
  const document = { schema_version: '1', model_route_refs: { private: 'model-route' }, chat_route_records: { private: 'credential-ref' },
    enabled_capabilities: [], skill_bindings: [], system_role_provider_overrides: {}, persona: '', instructions: '', starter_prompts: [] };
  const safe = editingDocument(document as unknown as Parameters<typeof editingDocument>[0]);
  expect(JSON.stringify(safe).includes('credential')).toBe(false);
  const snapshot = { version: 1, kind: 'preset', search: `?preset=${presetId}`,
    draft: { preset_id: presetId, display_name: 'Unsaved', current_revision: revision, document: safe } };
  expect(agentEditorReturn({ agentEditorReturn: snapshot }, `?preset=${presetId}`)).toEqual(snapshot);
  expect(agentEditorReturn({ agentEditorReturn: snapshot }, '?template=chat.minimal')).toBeNull();
  expect(agentEditorReturn({ agentEditorReturn: snapshot }, '?preset=another')).toBeNull();
});

test('a changed saved revision cannot receive an older model-management navigation snapshot', async () => {
  const document = createEmptyAgentPresetDocument();
  const original: AgentPresetEditorResponse = { preset: { preset_id: presetId, source: 'user', display_name: 'Saved Agent', bound_target_count: 0,
    current_stable_revision: revision }, draft: { preset_id: presetId, display_name: 'Saved Agent', document, current_revision: revision },
    revision: { reference: revision, document, created_by: 'owner', created_at_ms: 1 } };
  const v = await mount(undefined, original);
  fireEvent.click(v.getByRole('switch', { name: 'Enable Business check' }));
  fireEvent.click(v.getByRole('tab', { name: en.workbench.settingsTab }));
  fireEvent.change(v.getByRole('textbox', { name: en.fields.name }), { target: { value: 'Old unsaved name' } });
  fireEvent.click(v.getByRole('button', { name: en.workbench.manageModels }));
  await v.findByRole('heading', { name: 'Model management' });
  const newer = { ...revision, revision: 2 };
  v.getEditor.mockResolvedValue({ ...original, preset: { ...original.preset, display_name: 'New saved name', current_stable_revision: newer },
    draft: { ...original.draft, display_name: 'New saved name', current_revision: newer },
    revision: { ...original.revision!, reference: newer } });
  fireEvent.click(v.getByRole('button', { name: 'Back to editing' }));
  await v.findByRole('heading', { name: 'New saved name' });
  expect(v.getByText(en.workbench.returnChanged)).toBeTruthy();
  expect(v.queryByRole('heading', { name: 'Old unsaved name' }) === null).toBe(true);
  expect(v.getByRole('switch', { name: 'Enable Business check' })).toBeTruthy();
  expect(v.save).not.toHaveBeenCalled();
});


test('personal Agent app grants survive sidebar navigation until explicitly discarded', async () => {
  const document = createEmptyAgentPresetDocument();
  const original: AgentPresetEditorResponse = { preset: { preset_id: presetId, source: 'user', display_name: 'Saved Agent', bound_target_count: 0,
    current_stable_revision: revision }, draft: { preset_id: presetId, display_name: 'Saved Agent', document, current_revision: revision },
    revision: { reference: revision, document, created_by: 'owner', created_at_ms: 1 } };
  const v = await mount(undefined, original);
  const appName = en.modules.appHosting.name;
  fireEvent.click(v.getByRole('switch', { name: `Enable ${appName}` }));
  fireEvent.click(v.getByRole('button', { name: 'Sidebar models' }));
  await v.findByRole('heading', { name: 'Model management' });
  fireEvent.click(v.getByRole('button', { name: 'Sidebar Agent' }));
  await v.findByRole('heading', { name: 'Saved Agent' });
  expect(v.getByRole('switch', { name: `Disable ${appName}` })).toBeTruthy();
  fireEvent.click(v.getByRole('button', { name: en.workbench.resetChanges }));
  fireEvent.click(v.getByRole('button', { name: 'Sidebar models' }));
  await v.findByRole('heading', { name: 'Model management' });
  fireEvent.click(v.getByRole('button', { name: 'Sidebar Agent' }));
  await v.findByRole('heading', { name: 'Saved Agent' });
  expect(v.getByRole('switch', { name: `Enable ${appName}` })).toBeTruthy();
  expect(v.save).not.toHaveBeenCalled();
});

const customizedOfficialEditor = (): AgentPresetEditorResponse => {
  const document = documentWithModel();
  document.persona = 'Keep my personal style';
  document.instructions = 'Keep my instructions';
  document.starter_prompts = ['Keep this prompt'];
  document.context_order = [asCapabilityId('app.hosting')];
  document.middleware_order = [asCapabilityId('app.hosting')];
  document.enabled_capabilities = [{ capability: { id: asCapabilityId('app.hosting') }, action_allowlist: [APP_HOSTING_ACTIONS[0]] }];
  return { preset: { preset_id: presetId, source: 'official', display_name: 'My Coding Agent', description: 'My description',
    bound_target_count: 1, current_stable_revision: revision },
    draft: { preset_id: presetId, source_template_key: 'coding.codex', display_name: 'My Coding Agent', description: 'My description', document, current_revision: revision },
    revision: { reference: revision, document, created_by: 'owner', created_at_ms: 1 } };
};

test('recommended additions are explicitly reviewed and saved with the preview CAS while preserving custom settings', async () => {
  const original = customizedOfficialEditor();
  const merged = structuredClone(original.draft);
  merged.document.enabled_capabilities[0].action_allowlist = [...APP_HOSTING_ACTIONS];
  merged.document.enabled_capabilities.push({ capability: capability.capability, action_allowlist: ['business.check/run'] });
  const preview = spyOn(agentPlatform.recommendedCapabilities, 'invoke').mockResolvedValue({ draft: merged,
    added_capabilities: [
      { capability: { id: asCapabilityId('app.hosting') }, action_allowlist: APP_HOSTING_ACTIONS.slice(1) },
      { capability: capability.capability, action_allowlist: ['business.check/run'] },
    ], added_skills: [] });
  const v = await mount(undefined, original);
  expect(preview).not.toHaveBeenCalled();
  expect(v.save).not.toHaveBeenCalled();
  fireEvent.click(v.getByRole('button', { name: en.recommendation.preview }));
  const additions = within(await v.findByRole('region', { name: en.recommendation.title }));
  expect(additions.getByText('Business check')).toBeTruthy();
  expect(additions.getByText(en.recommendation.reviewHint)).toBeTruthy();
  expect(v.save).not.toHaveBeenCalled();
  fireEvent.click(additions.getByRole('button', { name: en.recommendation.apply }));
  await waitFor(() => expect(v.save).toHaveBeenCalledTimes(1));
  expect(v.save.mock.calls[0][0]).toEqual({ preset_id: presetId, request: { expected_current_revision: revision, draft: merged } });
  await waitFor(() => expect(v.queryByRole('region', { name: en.recommendation.title })).toBeNull());
  expect(v.getByRole('heading', { name: original.draft.display_name })).toBeTruthy();
  expect(v.getByRole('switch', { name: 'Disable Business check' })).toBeTruthy();
  expect(v.create).not.toHaveBeenCalled();
  expect(v.turn).not.toHaveBeenCalled();
});

test('unsaved custom changes disable recommended completion and are not replaced', async () => {
  const preview = spyOn(agentPlatform.recommendedCapabilities, 'invoke');
  const v = await mount(undefined, customizedOfficialEditor());
  fireEvent.click(v.getByRole('switch', { name: 'Enable Business check' }));
  const complete = v.getByRole('button', { name: en.recommendation.preview }) as HTMLButtonElement;
  expect(complete.disabled).toBe(true);
  expect(complete.title).toBe(en.recommendation.saveFirst);
  fireEvent.click(complete);
  expect(preview).not.toHaveBeenCalled();
  expect(v.getByRole('switch', { name: 'Disable Business check' })).toBeTruthy();
  expect(v.save).not.toHaveBeenCalled();
});

test('an already complete recommendation does not create a revision', async () => {
  const original = customizedOfficialEditor();
  spyOn(agentPlatform.recommendedCapabilities, 'invoke').mockResolvedValue({ draft: original.draft, added_capabilities: [], added_skills: [] });
  const v = await mount(undefined, original);
  fireEvent.click(v.getByRole('button', { name: en.recommendation.preview }));
  await v.findByText(en.recommendation.current);
  expect(v.queryByRole('button', { name: en.recommendation.apply })).toBeNull();
  expect(v.save).not.toHaveBeenCalled();
});

test('a recommendation from a newer saved revision requires refreshing instead of replacing the editor', async () => {
  const original = customizedOfficialEditor();
  const newer = { ...original.draft, display_name: 'Another saved edit', current_revision: { ...revision, revision: 2 } };
  spyOn(agentPlatform.recommendedCapabilities, 'invoke').mockResolvedValue({ draft: newer, added_capabilities: [], added_skills: [] });
  const v = await mount(undefined, original);
  fireEvent.click(v.getByRole('button', { name: en.recommendation.preview }));
  await v.findByText(en.errors.conflict);
  expect(v.getByRole('heading', { name: original.draft.display_name })).toBeTruthy();
  expect(v.queryByRole('region', { name: en.recommendation.title })).toBeNull();
  expect(v.save).not.toHaveBeenCalled();
  v.getEditor.mockResolvedValue({ ...original, draft: newer, preset: { ...original.preset,
    display_name: newer.display_name, current_stable_revision: newer.current_revision }, revision: { ...original.revision!, reference: newer.current_revision } });
  fireEvent.click(v.getByRole('button', { name: en.actions.retry }));
  await v.findByRole('heading', { name: newer.display_name });
  fireEvent.click(v.getByRole('button', { name: en.recommendation.preview }));
  await v.findByText(en.recommendation.current);
  expect(v.save).not.toHaveBeenCalled();
});

test('failed recommended completion keeps the reviewed additions and does not change the saved capabilities', async () => {
  const original = customizedOfficialEditor();
  const merged = structuredClone(original.draft);
  merged.document.enabled_capabilities.push({ capability: capability.capability, action_allowlist: ['business.check/run'] });
  spyOn(agentPlatform.recommendedCapabilities, 'invoke').mockResolvedValue({ draft: merged,
    added_capabilities: [{ capability: capability.capability, action_allowlist: ['business.check/run'] }], added_skills: [] });
  const v = await mount({ status: 409 }, original);
  fireEvent.click(v.getByRole('button', { name: en.recommendation.preview }));
  const additions = within(await v.findByRole('region', { name: en.recommendation.title }));
  fireEvent.click(additions.getByRole('button', { name: en.recommendation.apply }));
  await v.findByText(en.errors.conflict);
  expect(v.getByRole('region', { name: en.recommendation.title })).toBeTruthy();
  expect(v.getByRole('switch', { name: 'Enable Business check' })).toBeTruthy();
  expect(v.save).toHaveBeenCalledTimes(1);
});
