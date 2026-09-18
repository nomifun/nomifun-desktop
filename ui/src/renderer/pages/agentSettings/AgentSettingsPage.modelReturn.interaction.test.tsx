import '../../../../test/setup-dom.ts';
import '@arco-design/web-react/lib/_util/react-19-adapter';
import { act, cleanup, fireEvent, render, waitFor } from '@testing-library/react';
import { afterEach, expect, mock, spyOn, test } from 'bun:test';
import { useEffect } from 'react';
import { createInstance } from 'i18next';
import { I18nextProvider, initReactI18next } from 'react-i18next';
import { MemoryRouter, Route, Routes, useLocation, useNavigate } from 'react-router-dom';
import { SWRConfig } from 'swr';
import { NavigationHistoryProvider, useNavigationHistory } from '@/renderer/hooks/context/NavigationHistoryContext';
import { agentPlatform } from '@/common/adapter/ipcBridge';
import { pluginRuntimeProduct, type PluginRuntimeDraft } from '@/common/adapter/pluginRuntimeProductBridge';
import { asAgentPresetId, asCapabilityId, asPackageId, asDigestHex, createEmptyAgentPresetDocument, type AgentPresetEditorResponse,
  type AgentPresetLibraryResponse, type CapabilityCatalogItem, type CapabilityModuleCatalogItem, type OfficialPresetTemplate } from '@/common/types/agentPlatform';
import * as roleDefaults from './AgentRoleDefaults';
import * as libraryPanel from './AgentPresetLibrary';
import { agentEditorReturn, editingDocument, isAgentModelConfigurationMissing } from './model';
import AgentSettingsPage from './AgentSettingsPage';
import en from '../../services/i18n/locales/en-US/agentSettings.json';
import common from '../../services/i18n/locales/en-US/common.json';

const i18n = createInstance();
await i18n.use(initReactI18next).init({ lng: 'en-US', resources: { 'en-US': { translation: { agentSettings: en, common } } }, interpolation: { escapeValue: false } });
const template: OfficialPresetTemplate = { template_key: 'chat.minimal', immutable: true, forkable: true,
  seed: { enabled_capabilities: [], skill_bindings: [], required_resource_kinds: [], required_runtime_features: [] },
  role_coverage: { required_capability_categories: [], required_capability_ids: [], required_runtime_features: [], required_resource_kinds: [] } };
const otherTemplate = { ...template, template_key: 'coding.codex' } as OfficialPresetTemplate;
const capability: CapabilityCatalogItem = { capability: { id: asCapabilityId('business.check'), version: '1.0.0' }, kind: 'tool',
  display_name: 'Business check', description: 'Check business input', source_package: { id: asPackageId('test'), version: '1.0.0' },
  source_kind: 'plugin_product', materialization_state: 'materialized', supported_surfaces: ['desktop'], required_runtime_features: [],
  required_resource_kinds: [], required_capabilities: [], conflicting_capabilities: [], action_count: 1, context_contributor_count: 0 };
const capabilityModule: CapabilityModuleCatalogItem = {
  module: capability.capability, display_name: capability.display_name, description: capability.description,
  source_package: capability.source_package, authoring_policy: 'direct', summary_kind: 'tool',
  actions: [{ action_id: 'business.check/run', input_schema: 'input', output_schema: 'output', effect_class: 'pure', presentation: 'function_tool' }],
  context_schema_refs: [], event_schema_refs: [], required_resource_kinds: [], required_host_ports: [],
  required_modules: [], conflicting_modules: [], supported_surfaces: ['desktop'],
};
const presetId = asAgentPresetId('0190f5fe-7c00-7a00-8000-000000000101');
const revision = { preset_id: presetId, revision: 1, revision_digest: asDigestHex('b'.repeat(64)) };
afterEach(() => { cleanup(); mock.restore(); });

async function mount(error: unknown = { code: 'MODEL_ROUTE_NOT_CONFIGURED' }, initialEditor?: AgentPresetEditorResponse) {
  spyOn(roleDefaults, 'default').mockImplementation(() => <div>Default roles</div>);
  spyOn(libraryPanel, 'default').mockImplementation(({ library, onSelectTemplate }) => <nav>
    {library.official_templates.map(value => <button key={value.template_key} onClick={() => onSelectTemplate(value)}>{value.template_key}</button>)}
  </nav>);
  let editor: AgentPresetEditorResponse | undefined = initialEditor;
  const library = spyOn(agentPlatform.library, 'invoke').mockImplementation(async () => ({
    official_templates: [template, otherTemplate], user_presets: editor ? [editor.preset] : [], active_bindings: [],
    fresh_start: { data_generation: 4, legacy_data_imported: false, official_template_count: 2, user_preset_count: editor ? 1 : 0 },
  } as AgentPresetLibraryResponse));
  spyOn(agentPlatform.catalog, 'invoke').mockResolvedValue({ modules: [capabilityModule], capabilities: [capability], skills: [], mcp_tools: [], roles: [] });
  const create = spyOn(agentPlatform.createPreset, 'invoke').mockRejectedValueOnce(error).mockImplementation(async request => {
    editor = { preset: { preset_id: presetId, source: 'user', display_name: request.display_name, bound_target_count: 0, current_stable_revision: revision },
      draft: { preset_id: presetId, display_name: request.display_name, document: request.document!, current_revision: revision },
      revision: { reference: revision, document: request.document!, created_by: 'owner', created_at_ms: 1 } };
    return editor;
  });
  const save = spyOn(agentPlatform.saveRevision, 'invoke');
  const getEditor = spyOn(agentPlatform.getEditor, 'invoke').mockImplementation(async () => editor!);
  const turn = spyOn(agentPlatform.sessions.createTurn, 'invoke');
  const states: unknown[] = [];
  const Probe = () => { const location = useLocation(); useEffect(() => { states.push(location.state); }, [location]); return null; };
  const Models = () => { const navigate = useNavigate(), history = useNavigationHistory()!; return <div><h1>Model management</h1>
    <button onClick={history.back}>Back to editing</button><button onClick={() => navigate('/agent?template=coding.codex')}>Other template</button>
  </div>; };
  const Author = () => { const navigate = useNavigate(), history = useNavigationHistory()!; return <div><h1>Check authoring</h1>
    <button onClick={history.back}>Back to Agent</button><button onClick={() => navigate('/plugins/run/check?saved=1')}>Publish destination</button></div>; };
  const Published = () => { const history = useNavigationHistory()!; return <div><h1>Published check</h1><button onClick={history.back}>Back to author</button></div>; };
  const HistoryControls = () => { const history = useNavigationHistory()!; return <button disabled={!history.canForward} onClick={history.forward}>App forward</button>; };
  const view = render(<I18nextProvider i18n={i18n}><SWRConfig value={{ provider: () => new Map(), revalidateOnMount: false,
    fallback: { providers: [] } }}><MemoryRouter initialEntries={[initialEditor ? `/agent?preset=${presetId}` : '/agent?template=chat.minimal']}>
    <NavigationHistoryProvider><Probe /><HistoryControls /><Routes><Route path='/agent' element={<AgentSettingsPage />} /><Route path='/models' element={<Models />} />
      <Route path='/plugins/create/:id' element={<Author />} /><Route path='/plugins/run/:id' element={<Published />} /></Routes></NavigationHistoryProvider>
  </MemoryRouter></SWRConfig></I18nextProvider>);
  if (initialEditor) await view.findByRole('heading', { name: initialEditor.preset.display_name });
  else await view.findByRole('textbox', { name: en.workbench.customName });
  return { ...view, create, save, turn, states, library, getEditor };
}

test('missing-model CTA keeps the unsaved name and exact Module grant through model management and a single explicit save', async () => {
  const v = await mount();
  fireEvent.input(v.getByRole('textbox', { name: en.workbench.customName }), { target: { value: 'My inspection Agent' } });
  fireEvent.click(v.getByRole('switch', { name: 'Enable Business check' }));
  fireEvent.click(v.getByRole('button', { name: en.workbench.saveAsMine }));
  await v.findByText(en.workbench.modelNeeded);
  const original = structuredClone(v.create.mock.calls[0][0]);
  fireEvent.click(v.getByRole('button', { name: en.workbench.configureChatModel }));
  await v.findByRole('heading', { name: 'Model management' });
  expect(v.create).toHaveBeenCalledTimes(1);
  fireEvent.click(v.getByRole('button', { name: 'Back to editing' }));
  await waitFor(() => expect((v.getByRole('textbox', { name: en.workbench.customName }) as HTMLInputElement).value).toBe('My inspection Agent'));
  const snapshot = v.states.find(value => value && typeof value === 'object' && 'agentEditorReturn' in value) as { agentEditorReturn: { editing: { document: Record<string, unknown> } } };
  expect(snapshot.agentEditorReturn.editing.document.model_route_refs).toBeUndefined();
  expect(snapshot.agentEditorReturn.editing.document.chat_route_records).toBeUndefined();
  expect(v.getByRole('tab', { name: en.workbench.capabilityTab }).getAttribute('aria-selected')).toBe('true');
  expect(v.queryByRole('combobox')).toBeNull();
  expect(v.create).toHaveBeenCalledTimes(1);
  fireEvent.click(v.getByRole('tab', { name: en.workbench.capabilityTab }));
  expect(v.getByRole('switch', { name: 'Disable Business check' })).toBeTruthy();
  await act(async () => { fireEvent.click(v.getByRole('button', { name: en.workbench.saveAsMine })); });
  expect(v.create).toHaveBeenCalledTimes(2);
  expect(v.create.mock.calls[1][0]).toEqual(original);
  expect(v.save).not.toHaveBeenCalled();
  expect(v.turn).not.toHaveBeenCalled();
});

test('normal navigation to a different template does not restore the previous unsaved input', async () => {
  const v = await mount();
  fireEvent.input(v.getByRole('textbox', { name: en.workbench.customName }), { target: { value: 'Do not copy me' } });
  fireEvent.click(v.getByRole('button', { name: en.workbench.saveAsMine }));
  await v.findByRole('button', { name: en.workbench.configureChatModel });
  fireEvent.click(v.getByRole('button', { name: en.workbench.configureChatModel }));
  await v.findByRole('heading', { name: 'Model management' });
  fireEvent.click(v.getByRole('button', { name: 'Other template' }));
  await waitFor(() => expect((v.getByRole('textbox', { name: en.workbench.customName }) as HTMLInputElement).value).toBe(en.template.coding.codex.name));
  expect(v.create).toHaveBeenCalledTimes(1);
});

test('unrelated failures never offer model configuration as the recovery action', async () => {
  const v = await mount({ code: 'CAPABILITY_UNAVAILABLE', status: 400 });
  fireEvent.click(v.getByRole('button', { name: en.workbench.saveAsMine }));
  await waitFor(() => expect(v.create).toHaveBeenCalledTimes(1));
  expect(v.queryByRole('button', { name: en.workbench.configureChatModel }) === null).toBe(true);
  expect(isAgentModelConfigurationMissing({ code: 'MODEL_ROUTE_NOT_CONFIGURED' })).toBe(true);
  expect(isAgentModelConfigurationMissing({ code: 'CAPABILITY_UNAVAILABLE' })).toBe(false);
});

test('return snapshots exclude model configuration and cannot cross a changed template or preset query', () => {
  const document = { schema_version: '1', model_route_refs: { private: 'model-route' }, chat_route_records: { private: 'credential-ref' },
    enabled_capabilities: [], skill_bindings: [], system_role_provider_overrides: {}, persona: '', instructions: '', starter_prompts: [] };
  const safe = editingDocument(document as unknown as Parameters<typeof editingDocument>[0]);
  expect(JSON.stringify(safe).includes('credential')).toBe(false);
  const snapshot = { version: 1, kind: 'template', templateKey: 'chat.minimal', search: '?template=chat.minimal',
    editing: { displayName: 'Unsaved', document: safe, activeTab: 'settings' } };
  expect(agentEditorReturn({ agentEditorReturn: snapshot }, '?template=chat.minimal')).toEqual(snapshot);
  expect(agentEditorReturn({ agentEditorReturn: snapshot }, '?template=coding.codex')).toBeNull();
  expect(agentEditorReturn({ agentEditorReturn: snapshot }, '?preset=another')).toBeNull();
});

test('creating a before-tool draft returns to the same unsaved Agent edits without saving or sending', async () => {
  const document = createEmptyAgentPresetDocument();
  const original: AgentPresetEditorResponse = { preset: { preset_id: presetId, source: 'user', display_name: 'Saved Agent', bound_target_count: 0,
    current_stable_revision: revision }, draft: { preset_id: presetId, display_name: 'Saved Agent', document, current_revision: revision },
    revision: { reference: revision, document, created_by: 'owner', created_at_ms: 1 } };
  let finish!: (draft: PluginRuntimeDraft) => void;
  const createCheck = spyOn(pluginRuntimeProduct.beforeToolTemplate, 'invoke').mockImplementation(() => new Promise(resolve => { finish = resolve; }));
  const v = await mount(undefined, original);
  fireEvent.click(v.getByRole('switch', { name: 'Enable Business check' }));
  fireEvent.click(v.getByRole('tab', { name: en.workbench.settingsTab }));
  fireEvent.input(v.getByRole('textbox', { name: en.fields.name }), { target: { value: 'Unsaved Agent' } });
  await v.findByRole('heading', { name: 'Unsaved Agent' });
  fireEvent.click(v.getByRole('tab', { name: en.workbench.skillsTab }));
  fireEvent.click(v.getByRole('button', { name: en.middlewareOrder.createBeforeTool }));
  await act(async () => { finish({ id: 'check-draft' } as PluginRuntimeDraft); });
  await v.findByRole('heading', { name: 'Check authoring' });
  fireEvent.click(v.getByRole('button', { name: 'Publish destination' }));
  await v.findByRole('heading', { name: 'Published check' });
  fireEvent.click(v.getByRole('button', { name: 'Back to author' }));
  await v.findByRole('heading', { name: 'Check authoring' });
  fireEvent.click(v.getByRole('button', { name: 'Back to Agent' }));
  await v.findByRole('heading', { name: 'Unsaved Agent' });
  expect(v.getByRole('switch', { name: 'Disable Business check' })).toBeTruthy();
  fireEvent.click(v.getByRole('tab', { name: en.workbench.settingsTab }));
  expect(v.getAllByRole('combobox')).toHaveLength(1);
  expect(createCheck).toHaveBeenCalledTimes(1);
  expect(v.save).not.toHaveBeenCalled();
  expect(v.create).not.toHaveBeenCalled();
  expect(v.turn).not.toHaveBeenCalled();
  fireEvent.click(v.getByRole('button', { name: 'App forward' }));
  await v.findByRole('heading', { name: 'Check authoring' });
  fireEvent.click(v.getByRole('button', { name: 'Back to Agent' }));
  await v.findByRole('heading', { name: 'Saved Agent' });
  expect(v.queryByRole('heading', { name: 'Unsaved Agent' }) === null).toBe(true);
});

test('a changed saved revision cannot receive an older navigation snapshot', async () => {
  const document = createEmptyAgentPresetDocument();
  const original: AgentPresetEditorResponse = { preset: { preset_id: presetId, source: 'user', display_name: 'Saved Agent', bound_target_count: 0,
    current_stable_revision: revision }, draft: { preset_id: presetId, display_name: 'Saved Agent', document, current_revision: revision },
    revision: { reference: revision, document, created_by: 'owner', created_at_ms: 1 } };
  spyOn(pluginRuntimeProduct.beforeToolTemplate, 'invoke').mockResolvedValue({ id: 'check-draft' } as PluginRuntimeDraft);
  const v = await mount(undefined, original);
  fireEvent.click(v.getByRole('tab', { name: en.workbench.settingsTab }));
  fireEvent.change(v.getByRole('textbox', { name: en.fields.name }), { target: { value: 'Old unsaved name' } });
  fireEvent.click(v.getByRole('tab', { name: en.workbench.skillsTab }));
  fireEvent.click(v.getByRole('button', { name: en.middlewareOrder.createBeforeTool }));
  await v.findByRole('heading', { name: 'Check authoring' });
  const newer = { ...revision, revision: 2 };
  v.getEditor.mockResolvedValue({ ...original, preset: { ...original.preset, display_name: 'New saved name', current_stable_revision: newer },
    draft: { ...original.draft, display_name: 'New saved name', current_revision: newer },
    revision: { ...original.revision!, reference: newer } });
  fireEvent.click(v.getByRole('button', { name: 'Back to Agent' }));
  await v.findByRole('heading', { name: 'New saved name' });
  expect(v.getByText(en.workbench.returnChanged)).toBeTruthy();
  expect(v.queryByRole('heading', { name: 'Old unsaved name' }) === null).toBe(true);
  expect(v.save).not.toHaveBeenCalled();
});
