import '../../../../test/setup-dom.ts';
import '@arco-design/web-react/lib/_util/react-19-adapter';
import { act, cleanup, fireEvent, render, waitFor, within } from '@testing-library/react';
import { afterEach, beforeEach, expect, mock, spyOn, test } from 'bun:test';
import { createInstance } from 'i18next';
import { useState } from 'react';
import { I18nextProvider, initReactI18next } from 'react-i18next';
import { MemoryRouter, Route, Routes, useParams } from 'react-router-dom';
import { pluginPlatform } from '@/common/adapter/pluginPlatformBridge';
import type { PluginDraftDetail } from '@/common/types/pluginPlatform';
import { SWRConfig } from 'swr';
import { asCapabilityId, asPackageId, createEmptyAgentPresetDocument, placeCapability, type AgentPresetDocument, type CapabilityCatalogItem, type CapabilityModuleCatalogItem } from '@/common/types/agentPlatform';
import en from '../../services/i18n/locales/en-US/agentSettings.json';
import AgentContextOrder from './AgentContributionOrder';
import AgentPresetEditor from './AgentPresetEditor';
import { asAgentPresetId, asDigestHex, type AgentPresetDraft } from '@/common/types/agentPlatform';

const i18n = createInstance();
await i18n.use(initReactI18next).init({ lng: 'en-US', resources: { 'en-US': { translation: { agentSettings: en } } }, interpolation: { escapeValue: false } });
const item = (id: string, kind = 'context_contributor'): CapabilityCatalogItem => ({
  capability: { id: asCapabilityId(id) }, kind, display_name: id, description: '',
  source_package: { id: asPackageId('test'), version: '1.0.0' }, source_kind: 'managed_local',
  materialization_state: 'materialized', supported_surfaces: ['desktop'], required_runtime_features: [],
  required_resource_kinds: [], required_capabilities: [], conflicting_capabilities: [], action_count: 0, context_contributor_count: 1,
});
const catalog = [item('z'), item('tool', 'tool'), item('a')];
const moduleOf = (value: CapabilityCatalogItem): CapabilityModuleCatalogItem => ({
  module: value.capability, display_name: value.display_name, description: value.description,
  source_package: value.source_package, authoring_policy: 'direct', summary_kind: value.kind,
  actions: [], context_schema_refs: value.context_contributor_count ? [`schema://${value.capability.id}`] : [],
  event_schema_refs: [], required_resource_kinds: [], required_host_ports: [],
  required_modules: [], conflicting_modules: [], supported_surfaces: ['desktop'],
});
const initial = (): AgentPresetDocument => ({ ...createEmptyAgentPresetDocument(), enabled_capabilities: catalog.map(value => ({ capability: value.capability })) });
function mount(document = initial(), currentCatalog = catalog, disabled = false, kind: 'context' | 'middleware' = 'context') {
  let current = document;
  const Harness = () => {
    const [value, setValue] = useState(document);
    return <AgentContextOrder document={value} catalog={currentCatalog} disabled={disabled} kind={kind} onChange={next => { current = next; setValue(next); }} />;
  };
  const Creator = () => <div>Draft {useParams().id}</div>;
  return { ...render(<I18nextProvider i18n={i18n}><MemoryRouter><Routes>
    <Route path='/' element={<Harness />} /><Route path='/plugins/create/:id' element={<Creator />} />
  </Routes></MemoryRouter></I18nextProvider>), state: () => current };
}
beforeEach(() => {
  (window as typeof window & { __backendPort?: number }).__backendPort = 11451;
});
afterEach(() => {
  delete (window as typeof window & { __backendPort?: number }).__backendPort;
  cleanup();
  mock.restore();
});

test('personal editor submits order through its existing save action', () => {
  const original: AgentPresetDraft = { preset_id: asAgentPresetId('0190f5fe-7c00-7a00-8000-000000000001'), display_name: 'Ordered Agent', document: initial() };
  const middlewareCatalog = ['m-a', 'm-z'].map(id => ({ ...item(id, 'turn_middleware'), action_count: 1, context_contributor_count: 0 }));
  original.document.enabled_capabilities.push(...middlewareCatalog.map(value => ({ capability: value.capability })));
  original.document.chat_route_records.agent_chat = { schema: 'nomifun.chat-route-record.v1', task: 'agent_chat', failovers: [], primary: {
    model_route_id: 'test-route', model_route_revision: 1, provider_id: 'test-provider', model: 'test-model', protocol: 'openai_chat',
    connection_config_ref: 'test-config', config_revision_digest: asDigestHex('c'.repeat(64)), credential_ref: 'test-credential', features: ['text_input', 'text_output'],
  } };
  let saved: AgentPresetDraft | undefined;
  const Harness = () => {
    const [draft, setDraft] = useState(original);
    return <AgentPresetEditor editor={{ preset: { preset_id: draft.preset_id, display_name: draft.display_name, source: 'user', bound_target_count: 0 }, draft: original }}
      draft={draft} catalog={{ modules: [...catalog, ...middlewareCatalog].map(moduleOf), capabilities: [...catalog, ...middlewareCatalog], skills: [], roles: [], mcp_tools: [] }} busyAction={null} dirty={draft !== original}
      onDraftChange={setDraft} onSave={() => { saved = draft; }} onStartConversation={() => {}} />;
  };
  const result = render(<I18nextProvider i18n={i18n}><SWRConfig value={{ provider: () => new Map(), fallback: { providers: [] }, revalidateOnMount: false }}><MemoryRouter><Harness /></MemoryRouter></SWRConfig></I18nextProvider>);
  const view = within(result.container);
  fireEvent.click(view.getByRole('tab', { name: en.workbench.settingsTab }));
  expect(view.getByRole('textbox', { name: en.fields.name })).toBeTruthy();
  expect(view.getAllByRole('combobox')).toHaveLength(1);
  expect(view.queryByRole('region', { name: en.middlewareOrder.title }) === null).toBe(true);
  expect(view.queryByRole('region', { name: en.contextOrder.title }) === null).toBe(true);
  fireEvent.click(view.getByRole('tab', { name: en.workbench.skillsTab }));
  expect(view.getAllByRole('heading', { level: 3 }).slice(0, 2).map(node => node.textContent)).toEqual([en.middlewareOrder.title, en.contextOrder.title]);
  expect(within(view.getByRole('tabpanel')).getByText(en.sections.skillsMcp)).toBeTruthy();
  expect(view.queryByRole('textbox', { name: en.fields.name }) === null).toBe(true);
  fireEvent.click(view.getByRole('button', { name: 'Move z earlier' }));
  fireEvent.click(view.getByRole('button', { name: 'Move m-z earlier' }));
  fireEvent.click(view.getByRole('button', { name: 'common.save' }));
  expect(saved?.document.context_order).toEqual(['z', 'a']);
  expect(saved?.document.middleware_order).toEqual(['m-z', 'm-a']);
  expect(saved?.document.enabled_capabilities).toEqual(original.document.enabled_capabilities);
});

test('reorders Context only, preserving selections and default reset', () => {
  const original = initial(), result = mount(original), view = within(result.container);
  expect(view.getAllByRole('listitem').map(row => row.textContent)).toEqual(['a↑↓', 'z↑↓']);
  fireEvent.click(view.getByRole('button', { name: 'Move z earlier' }));
  expect(result.state().context_order).toEqual(['z', 'a']);
  expect(result.state().enabled_capabilities).toEqual(original.enabled_capabilities);
  expect(original.context_order).toBeUndefined();
  fireEvent.click(view.getByRole('button', { name: en.contextOrder.reset }));
  expect(result.state().context_order).toBeUndefined();
});

test('retains missing explicit choices and disabled editing does not change them', () => {
  const document = { ...initial(), context_order: [asCapabilityId('z'), asCapabilityId('a')] };
  const result = mount(document, [item('a')], true), view = within(result.container);
  expect(view.getByText(en.contextOrder.missing)).toBeTruthy();
  fireEvent.click(view.getByRole('button', { name: 'Move z later' }));
  expect(result.state()).toEqual(document);
  expect((view.getByRole('button', { name: en.contextOrder.reset }) as HTMLButtonElement).disabled).toBe(true);
});

test('explicit capability removal removes only its order entry', () => {
  const document = { ...initial(), context_order: [asCapabilityId('z'), asCapabilityId('a')] };
  const removed = placeCapability(document, catalog[0].capability, 'none');
  expect(removed.context_order).toEqual(['a']);
  expect(document.context_order).toEqual(['z', 'a']);
  expect(placeCapability(removed, catalog[2].capability, 'none').context_order).toBeUndefined();
});

test('middleware order preserves Context and selection, retains missing choices, resets and removes explicitly', () => {
  const middlewareCatalog = ['m-z', 'm-a', 'unselected'].map(id => ({ ...item(id, 'turn_middleware'), action_count: 1, context_contributor_count: 0 }));
  const document: AgentPresetDocument = { ...initial(), context_order: [asCapabilityId('z')], enabled_capabilities: [
    ...initial().enabled_capabilities, ...middlewareCatalog.slice(0, 2).map(value => ({ capability: value.capability })),
  ] };
  const result = mount(document, [...catalog, ...middlewareCatalog], false, 'middleware'), view = within(result.container);
  expect(view.getAllByRole('listitem').map(row => row.textContent)).toEqual([
    `m-a${en.middlewareOrder.phase.unknown}↑↓`, `m-z${en.middlewareOrder.phase.unknown}↑↓`,
  ]);
  fireEvent.click(view.getByRole('button', { name: 'Move m-z earlier' }));
  expect(result.state().middleware_order).toEqual(['m-z', 'm-a']);
  expect(result.state().context_order).toEqual(document.context_order);
  expect(result.state().enabled_capabilities).toEqual(document.enabled_capabilities);
  const ordered = result.state();
  fireEvent.click(view.getByRole('button', { name: en.middlewareOrder.reset }));
  expect(result.state().middleware_order).toBeUndefined();
  expect(result.state().context_order).toEqual(document.context_order);
  result.unmount();
  const missing = mount(ordered, [middlewareCatalog[1]], true, 'middleware'), readonly = within(missing.container);
  expect(readonly.getByText(en.middlewareOrder.missing)).toBeTruthy();
  fireEvent.click(readonly.getByRole('button', { name: 'Move m-z later' }));
  expect(missing.state()).toEqual(ordered);
  expect((readonly.getByRole('button', { name: en.middlewareOrder.reset }) as HTMLButtonElement).disabled).toBe(true);
  const removed = placeCapability(ordered, middlewareCatalog[0].capability, 'none');
  expect(removed.middleware_order).toEqual(['m-a']);
  expect(removed.context_order).toEqual(document.context_order);
  expect(placeCapability(removed, middlewareCatalog[1].capability, 'none').middleware_order).toBeUndefined();
});

test('execution stages come from the host and unknown selected extensions remain in order', () => {
  const extensions: CapabilityCatalogItem[] = [
    { ...item('model', 'turn_middleware'), middleware_phase: 'before_model' },
    { ...item('check', 'turn_middleware'), middleware_phase: 'before_tool' },
    { ...item('unknown', 'turn_middleware'), action_count: 4 },
    { ...item('future', 'turn_middleware'), middleware_phase: 'future_stage' as CapabilityCatalogItem['middleware_phase'] },
  ];
  const original: AgentPresetDocument = { ...initial(),
    enabled_capabilities: extensions.map(value => ({ capability: value.capability })),
    middleware_order: [asCapabilityId('unknown'), asCapabilityId('check')],
  };
  const result = mount(original, extensions, false, 'middleware'), view = within(result.container);
  expect(view.getAllByRole('listitem').map(row => row.textContent)).toEqual([
    `unknown${en.middlewareOrder.phase.unknown}↑↓`,
    `check${en.middlewareOrder.phase.before_tool}↑↓`,
    `future${en.middlewareOrder.phase.unknown}↑↓`,
    `model${en.middlewareOrder.phase.before_model}↑↓`,
  ]);
  expect(view.getByText(en.middlewareOrder.toolAccess)).toBeTruthy();
  fireEvent.click(view.getByRole('button', { name: 'Move check earlier' }));
  expect(result.state().middleware_order).toEqual(['check', 'unknown', 'future', 'model']);
  expect(result.state().enabled_capabilities).toEqual(original.enabled_capabilities);
});

test('ordinary user creates one check draft from an empty list without saving or changing selection', async () => {
  let finish!: (draft: PluginDraftDetail) => void;
  const create = spyOn(pluginPlatform.drafts.create, 'invoke').mockImplementation(() => new Promise(resolve => { finish = resolve; }));
  const save = spyOn(pluginPlatform.drafts.save, 'invoke');
  const document = createEmptyAgentPresetDocument();
  const result = mount(document, [], false, 'middleware'), view = within(result.container);
  expect(view.getByText(en.middlewareOrder.empty)).toBeTruthy();
  const button = view.getByRole('button', { name: en.middlewareOrder.createBeforeTool });
  fireEvent.click(button); fireEvent.click(button);
  expect(create).toHaveBeenCalledTimes(1);
  expect(create.mock.calls[0]).toEqual([{ template: 'agent.before_tool' }]);
  expect((button as HTMLButtonElement).disabled).toBe(true);
  await act(async () => { finish({ summary: { draft_id: 'check-draft' } } as PluginDraftDetail); });
  expect(view.getByText('Draft check-draft')).toBeTruthy();
  expect(save).not.toHaveBeenCalled();
  expect(result.state()).toEqual(document);
});

test('failed check draft creation is visible and retries only on another explicit click', async () => {
  const create = spyOn(pluginPlatform.drafts.create, 'invoke').mockRejectedValue(new Error('offline'));
  const result = mount(createEmptyAgentPresetDocument(), [], false, 'middleware'), view = within(result.container);
  fireEvent.click(view.getByRole('button', { name: en.middlewareOrder.createBeforeTool }));
  await waitFor(() => expect(view.getByText(en.middlewareOrder.createFailed)).toBeTruthy());
  expect(create).toHaveBeenCalledTimes(1);
  await act(async () => { fireEvent.click(view.getByRole('button', { name: en.middlewareOrder.createBeforeTool })); });
  expect(create).toHaveBeenCalledTimes(2);
  expect(view.queryByText(/Draft /)).toBeNull();
});

test('disabled editor cannot create a check and Context does not offer the execution template', () => {
  const create = spyOn(pluginPlatform.drafts.create, 'invoke');
  const result = mount(createEmptyAgentPresetDocument(), [], true, 'middleware'), view = within(result.container);
  const button = view.getByRole('button', { name: en.middlewareOrder.createBeforeTool });
  expect((button as HTMLButtonElement).disabled).toBe(true);
  fireEvent.click(button);
  expect(create).not.toHaveBeenCalled();
  result.unmount();
  const context = mount();
  expect(within(context.container).queryByRole('button', { name: en.middlewareOrder.createBeforeTool })).toBeNull();
});

test('remote WebUI cannot create a Plugin draft from Agent settings', () => {
  delete (window as typeof window & { __backendPort?: number }).__backendPort;
  const create = spyOn(pluginPlatform.drafts.create, 'invoke');
  const result = mount(createEmptyAgentPresetDocument(), [], false, 'middleware');
  const button = within(result.container).getByRole('button', {
    name: en.middlewareOrder.createBeforeTool,
  });
  expect((button as HTMLButtonElement).disabled).toBe(true);
  fireEvent.click(button);
  expect(create).not.toHaveBeenCalled();
});

test('a late check draft response cannot navigate after the editor unmounts', async () => {
  let finish!: (draft: PluginDraftDetail) => void;
  const create = spyOn(pluginPlatform.drafts.create, 'invoke').mockImplementation(() => new Promise(resolve => { finish = resolve; }));
  const result = mount(createEmptyAgentPresetDocument(), [], false, 'middleware');
  fireEvent.click(within(result.container).getByRole('button', { name: en.middlewareOrder.createBeforeTool }));
  result.unmount();
  const next = mount(createEmptyAgentPresetDocument(), [], false, 'middleware');
  await act(async () => { finish({ summary: { draft_id: 'old-draft' } } as PluginDraftDetail); });
  expect(within(next.container).queryByText('Draft old-draft')).toBeNull();
  expect(within(next.container).getByRole('button', { name: en.middlewareOrder.createBeforeTool })).toBeTruthy();
  expect(create).toHaveBeenCalledTimes(1);
});
