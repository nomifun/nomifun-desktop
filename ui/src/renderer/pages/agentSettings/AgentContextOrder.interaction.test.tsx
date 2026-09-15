import '../../../../test/setup-dom.ts';
import '@arco-design/web-react/lib/_util/react-19-adapter';
import { cleanup, fireEvent, render, within } from '@testing-library/react';
import { afterEach, expect, test } from 'bun:test';
import { createInstance } from 'i18next';
import { useState } from 'react';
import { I18nextProvider, initReactI18next } from 'react-i18next';
import { MemoryRouter } from 'react-router-dom';
import { SWRConfig } from 'swr';
import { asCapabilityId, asPackageId, createEmptyAgentPresetDocument, placeCapability, type AgentPresetDocument, type CapabilityCatalogItem } from '@/common/types/agentPlatform';
import en from '../../services/i18n/locales/en-US/agentSettings.json';
import AgentContextOrder from './AgentContributionOrder';
import AgentPresetEditor from './AgentPresetEditor';
import { asAgentPresetId, asDigestHex, type AgentPresetDraft } from '@/common/types/agentPlatform';

const i18n = createInstance();
await i18n.use(initReactI18next).init({ lng: 'en-US', resources: { 'en-US': { translation: { agentSettings: en } } }, interpolation: { escapeValue: false } });
const item = (id: string, kind = 'context_contributor'): CapabilityCatalogItem => ({
  capability: { id: asCapabilityId(id), version: '1.0.0' }, kind, display_name: id, description: '',
  source_package: { id: asPackageId('test'), version: '1.0.0' }, source_kind: 'managed_local',
  materialization_state: 'materialized', supported_surfaces: ['desktop'], required_runtime_features: [],
  required_resource_kinds: [], required_capabilities: [], conflicting_capabilities: [], action_count: 0, context_contributor_count: 1,
});
const catalog = [item('z'), item('tool', 'tool'), item('a')];
const initial = (): AgentPresetDocument => ({ ...createEmptyAgentPresetDocument(), enabled_capabilities: catalog.map(value => ({ capability: value.capability })) });
function mount(document = initial(), currentCatalog = catalog, disabled = false, kind: 'context' | 'middleware' = 'context') {
  let current = document;
  const Harness = () => {
    const [value, setValue] = useState(document);
    return <AgentContextOrder document={value} catalog={currentCatalog} disabled={disabled} kind={kind} onChange={next => { current = next; setValue(next); }} />;
  };
  return { ...render(<I18nextProvider i18n={i18n}><Harness /></I18nextProvider>), state: () => current };
}
afterEach(cleanup);

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
      draft={draft} catalog={{ capabilities: [...catalog, ...middlewareCatalog], skills: [], roles: [], mcp_tools: [] }} busyAction={null} dirty={draft !== original}
      onDraftChange={setDraft} onSave={() => { saved = draft; }} onStartConversation={() => {}} />;
  };
  const result = render(<I18nextProvider i18n={i18n}><SWRConfig value={{ provider: () => new Map(), fallback: { providers: [] }, revalidateOnMount: false }}><MemoryRouter><Harness /></MemoryRouter></SWRConfig></I18nextProvider>);
  const view = within(result.container);
  fireEvent.click(view.getByRole('tab', { name: en.workbench.settingsTab }));
  fireEvent.click(view.getByRole('button', { name: 'Move z earlier' }));
  fireEvent.click(view.getByRole('button', { name: 'Move middleware m-z earlier' }));
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
  expect(view.getAllByRole('listitem').map(row => row.textContent)).toEqual(['m-a↑↓', 'm-z↑↓']);
  fireEvent.click(view.getByRole('button', { name: 'Move middleware m-z earlier' }));
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
  fireEvent.click(readonly.getByRole('button', { name: 'Move middleware m-z later' }));
  expect(missing.state()).toEqual(ordered);
  expect((readonly.getByRole('button', { name: en.middlewareOrder.reset }) as HTMLButtonElement).disabled).toBe(true);
  const removed = placeCapability(ordered, middlewareCatalog[0].capability, 'none');
  expect(removed.middleware_order).toEqual(['m-a']);
  expect(removed.context_order).toEqual(document.context_order);
  expect(placeCapability(removed, middlewareCatalog[1].capability, 'none').middleware_order).toBeUndefined();
});
