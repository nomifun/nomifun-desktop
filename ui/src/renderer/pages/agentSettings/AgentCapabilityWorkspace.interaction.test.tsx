import '../../../../test/setup-dom.ts';
import { useState } from 'react';
import { cleanup, fireEvent, render, within, waitFor } from '@testing-library/react';
import { afterEach, describe, expect, test } from 'bun:test';
import { createInstance } from 'i18next';
import { I18nextProvider, initReactI18next } from 'react-i18next';
import {
  asCapabilityId, asPackageId, createEmptyAgentPresetDocument,
  type AgentPresetDocument, type CapabilityCatalogItem, type OfficialPresetTemplate,
} from '@/common/types/agentPlatform';
import en from '../../services/i18n/locales/en-US/agentSettings.json';
import common from '../../services/i18n/locales/en-US/common.json';
import AgentCapabilityWorkspace from './AgentCapabilityWorkspace';
import OfficialTemplateOverview from './OfficialTemplateOverview';

const testI18n = createInstance();
await testI18n.use(initReactI18next).init({ lng: 'en-US', fallbackLng: 'en-US', resources: { 'en-US': { translation: { agentSettings: en, common } } }, interpolation: { escapeValue: false } });
const item = (id: string, available = true, name = id): CapabilityCatalogItem => ({
  capability: { id: asCapabilityId(id), version: '1.0.0' }, kind: 'tool', display_name: name,
  description: id, source_package: { id: asPackageId('nomifun.example'), version: '1.0.0' },
  source_kind: 'bundled', materialization_state: available ? 'materialized' : 'unavailable',
  supported_surfaces: ['desktop'], required_runtime_features: [], required_resource_kinds: id.startsWith('fs.') ? ['workspace'] : [],
  required_capabilities: [], conflicting_capabilities: [], action_count: 1, context_contributor_count: 0,
});
const read = item('fs.read'), knowledge = item('knowledge.read'), web = item('web.fetch'), unavailable = item('web.search', false);
const selection = (row: CapabilityCatalogItem) => ({ capability: row.capability, action_allowlist: [`${row.capability.id}.invoke`] });
const documentWith = (initial: CapabilityCatalogItem[], onDemand: CapabilityCatalogItem[] = []): AgentPresetDocument => ({ ...createEmptyAgentPresetDocument(), initial_capabilities: initial.map(selection), on_demand_capabilities: onDemand.map(selection) });
function mount(document: AgentPresetDocument, catalog = [read, knowledge, web, unavailable], disabled = false) {
  let current = document;
  const Harness = () => { const [value, setValue] = useState(document); return <AgentCapabilityWorkspace document={value} catalog={catalog} disabled={disabled} onChange={(next) => { current = next; setValue(next); }} />; };
  const result = render(<I18nextProvider i18n={testI18n}><Harness /></I18nextProvider>);
  return { ...result, state: () => current };
}
afterEach(() => cleanup());

describe('Agent capability workspace', () => {
  test('starts with configured capabilities, grouped by task, instead of the whole catalog', () => {
    const screen = mount(documentWith([knowledge], [read]));
    expect(screen.getByRole('button', { name: 'Read the knowledge base' })).toBeTruthy();
    expect(screen.getByRole('button', { name: 'Read files' })).toBeTruthy();
    expect(screen.queryByRole('button', { name: 'Read a webpage' })).toBeNull();
    expect(screen.getByRole('heading', { name: 'Knowledge & memory' })).toBeTruthy();
    expect(screen.getByRole('heading', { name: 'Files & development' })).toBeTruthy();
    expect(screen.queryByText('fs.read', { exact: true })).toBeNull();
  });

  test('filters by search and category without changing the configured scope', () => {
    const screen = mount(documentWith([knowledge], [read]));
    fireEvent.change(screen.getByRole('searchbox', { name: en.workbench.searchConfigured }), { target: { value: 'files' } });
    expect(screen.queryByRole('button', { name: 'Read the knowledge base' })).toBeNull();
    expect(screen.getByRole('button', { name: 'Read files' })).toBeTruthy();
    expect(screen.state().initial_capabilities).toHaveLength(1);
    fireEvent.change(screen.getByRole('searchbox', { name: en.workbench.searchConfigured }), { target: { value: '' } });
    fireEvent.click(screen.getByRole('button', { name: /^Knowledge & memory/ }));
    expect(screen.queryByRole('button', { name: 'Read files' })).toBeNull();
    expect(screen.state().on_demand_capabilities).toHaveLength(1);
  });

  test('stages library choices and adds them together as on-demand capabilities', async () => {
    const screen = mount(documentWith([knowledge], [read]));
    fireEvent.click(screen.getByRole('button', { name: en.workbench.addCapabilities }));
    const body = within(document.body);
    const search = await body.findByRole('searchbox', { name: en.workbench.searchLibrary });
    fireEvent.change(search, { target: { value: 'web.fetch' } });
    fireEvent.click(body.getByRole('checkbox', { name: 'Add Read a webpage' }));
    expect(screen.state().on_demand_capabilities).toHaveLength(1);
    fireEvent.click(body.getByRole('button', { name: 'Add selected (1)' }));
    expect(screen.state().on_demand_capabilities.map((entry) => entry.capability.id)).toEqual(['fs.read', 'web.fetch']);
    expect(screen.state().on_demand_capabilities[0].action_allowlist).toEqual(['fs.read.invoke']);
    expect(screen.state().initial_capabilities).toEqual([selection(knowledge)]);
  });

  test('does not allow unavailable library entries to be selected', async () => {
    const screen = mount(documentWith([read]));
    fireEvent.click(screen.getByRole('button', { name: en.workbench.addCapabilities }));
    const body = within(document.body);
    await body.findByRole('searchbox', { name: en.workbench.searchLibrary });
    expect(body.queryByRole('checkbox', { name: 'Add Search the web' })).toBeNull();
    fireEvent.click(body.getByRole('checkbox', { name: en.workbench.showUnavailable }));
    expect((body.getByRole('checkbox', { name: 'Add Search the web' }) as HTMLInputElement).disabled).toBe(true);
    expect(screen.state().initial_capabilities).toEqual([selection(read)]);
  });

  test('bulk activation preserves exact action restrictions and does not include unselected entries', () => {
    const screen = mount(documentWith([read, knowledge], [web]));
    fireEvent.click(screen.getByRole('checkbox', { name: 'Select Read files' }));
    fireEvent.click(screen.getByRole('checkbox', { name: 'Select Read the knowledge base' }));
    fireEvent.click(screen.getByRole('button', { name: en.capabilities.onDemandShort }));
    expect(screen.state().initial_capabilities).toEqual([]);
    expect(screen.state().on_demand_capabilities).toHaveLength(3);
    expect(screen.state().on_demand_capabilities.find((entry) => entry.capability.id === 'fs.read')?.action_allowlist).toEqual(['fs.read.invoke']);
    expect(screen.state().on_demand_capabilities.find((entry) => entry.capability.id === 'web.fetch')).toEqual(selection(web));
  });

  test('retains missing references visibly and can remove all unavailable entries at once', () => {
    const missing = item('missing.widget');
    const screen = mount(documentWith([read, missing], [unavailable]));
    fireEvent.click(screen.getByRole('button', { name: /Needs attention 2/ }));
    expect(screen.getByText(en.workbench.missingSource)).toBeTruthy();
    fireEvent.click(screen.getByRole('button', { name: en.workbench.removeUnavailable }));
    expect(screen.state().initial_capabilities).toEqual([selection(read)]);
    expect(screen.state().on_demand_capabilities).toEqual([]);
    expect(screen.getByRole('button', { name: 'Read files' })).toBeTruthy();
  });

  test('removing the last capability shows a useful empty state', () => {
    const screen = mount(documentWith([read]));
    fireEvent.click(screen.getByRole('button', { name: 'Remove Read files' }));
    expect(screen.getByRole('heading', { name: en.workbench.emptyTitle })).toBeTruthy();
    expect(screen.state().initial_capabilities).toEqual([]);
  });

  test('opens capability details while keeping IDs out of the normal list', async () => {
    const screen = mount(documentWith([read]));
    fireEvent.click(screen.getByRole('button', { name: 'Read files' }));
    const body = within(document.body);
    await body.findByText('Workspace', { exact: true });
    const technical = body.getByText(common.technical_details).closest('details');
    expect(technical?.open).toBe(false);
    expect(technical?.textContent?.includes('fs.read')).toBe(true);
    expect(technical?.textContent?.includes('nomifun.example@1.0.0')).toBe(true);
  });

  test('paginates long configurations without dropping selections', () => {
    const rows = Array.from({ length: 25 }, (_, index) => item(`example.tool.${index}`, true, `Example ${index}`));
    const screen = mount(documentWith(rows), rows);
    expect(screen.getAllByRole('button', { name: /^Example \d+$/ })).toHaveLength(12);
    const next = screen.container.querySelector('.arco-pagination-item-next');
    expect(next).toBeTruthy();
    fireEvent.click(next!);
    expect(screen.getAllByRole('button', { name: /^Example \d+$/ })).toHaveLength(12);
    expect(screen.queryByRole('button', { name: 'Example 0' })).toBeNull();
    expect(screen.state().initial_capabilities).toHaveLength(25);
  });

  test('disables mutation controls while a save is in progress', () => {
    const screen = mount(documentWith([read]), [read, web], true);
    expect((screen.getByRole('button', { name: 'Remove Read files' }) as HTMLButtonElement).disabled).toBe(true);
    expect((screen.getByRole('button', { name: en.workbench.addCapabilities }) as HTMLButtonElement).disabled).toBe(true);
  });
});

describe('editable official preset', () => {
  test('allows removing unavailable seed capabilities before creating a personal Agent', async () => {
    const template: OfficialPresetTemplate = { template_key: 'assistant.general', seed: { initial_capabilities: [read.capability], on_demand_capabilities: [unavailable.capability], skill_bindings: [], required_resource_kinds: [], required_runtime_features: [] }, role_coverage: { required_capability_categories: [], required_capability_ids: [], required_runtime_features: [], required_resource_kinds: [] }, immutable: true, forkable: true };
    const original = structuredClone(template);
    const saved: Array<{ name: string; document: AgentPresetDocument }> = [];
    const screen = render(<I18nextProvider i18n={testI18n}><OfficialTemplateOverview template={template} catalog={{ capabilities: [read, unavailable], skills: [], mcp_tools: [] }} busy={false} onSave={(name, document) => saved.push({ name, document })} /></I18nextProvider>);
    expect((screen.getByRole('button', { name: en.workbench.saveAsMine }) as HTMLButtonElement).disabled).toBe(true);
    fireEvent.click(screen.getByRole('button', { name: /Needs attention 1/ }));
    fireEvent.click(screen.getByRole('button', { name: en.workbench.removeUnavailable }));
    fireEvent.change(screen.getByRole('textbox', { name: en.workbench.customName }), { target: { value: 'My assistant' } });
    expect(saved).toHaveLength(0);
    fireEvent.click(screen.getByRole('button', { name: en.workbench.saveAsMine }));
    await waitFor(() => expect(saved).toHaveLength(1));
    expect(saved[0].name).toBe('My assistant');
    expect(saved[0].document.initial_capabilities.map((entry) => entry.capability.id)).toEqual(['fs.read']);
    expect(saved[0].document.on_demand_capabilities).toEqual([]);
    expect(template).toEqual(original);
  });
});
