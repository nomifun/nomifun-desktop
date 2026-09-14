import '../../../../test/setup-dom.ts';
import '@arco-design/web-react/lib/_util/react-19-adapter';
import { cleanup, fireEvent, render, within, waitFor } from '@testing-library/react';
import { afterEach, describe, expect, test } from 'bun:test';
import { createInstance } from 'i18next';
import { type ReactElement, useState } from 'react';
import { I18nextProvider, initReactI18next } from 'react-i18next';
import { MemoryRouter } from 'react-router-dom';
import {
  asCapabilityId, asPackageId, createEmptyAgentPresetDocument,
  type AgentPresetDocument, type CapabilityCatalogItem,
} from '@/common/types/agentPlatform';
import en from '../../services/i18n/locales/en-US/agentSettings.json';
import common from '../../services/i18n/locales/en-US/common.json';
import AgentCapabilityWorkspace from './AgentCapabilityWorkspace';

const testI18n = createInstance();
await testI18n.use(initReactI18next).init({ lng: 'en-US', fallbackLng: 'en-US', resources: { 'en-US': { translation: { agentSettings: en, common } } }, interpolation: { escapeValue: false } });
const item = (id: string, available = true, name = id): CapabilityCatalogItem => ({
  capability: { id: asCapabilityId(id), version: '1.0.0' }, kind: 'tool', display_name: name,
  description: id, source_package: { id: asPackageId('nomifun.example'), version: '1.0.0' },
  source_kind: 'bundled', materialization_state: available ? 'materialized' : 'unavailable',
  unavailable_code: available ? undefined : 'CAPABILITY_UNAVAILABLE',
  supported_surfaces: ['desktop'], required_runtime_features: [], required_resource_kinds: id.startsWith('fs.') ? ['workspace'] : [],
  required_capabilities: [], conflicting_capabilities: [], action_count: 1, context_contributor_count: 0,
});
const read = item('fs.read'), knowledge = item('knowledge.read'), web = item('web.fetch'), unavailable = item('web.search', false);
const selection = (row: CapabilityCatalogItem) => ({ capability: row.capability, action_allowlist: [`${row.capability.id}.invoke`] });
const documentWith = (enabled: CapabilityCatalogItem[]): AgentPresetDocument => ({ ...createEmptyAgentPresetDocument(), enabled_capabilities: enabled.map(selection) });
const renderInRouter = (ui: ReactElement) => render(<MemoryRouter>{ui}</MemoryRouter>);
function mount(document: AgentPresetDocument, catalog = [read, knowledge, web, unavailable], disabled = false) {
  let current = document;
  const Harness = () => { const [value, setValue] = useState(document); return <AgentCapabilityWorkspace document={value} catalog={catalog} disabled={disabled} onChange={(next) => { current = next; setValue(next); }} />; };
  const result = renderInRouter(<I18nextProvider i18n={testI18n}><Harness /></I18nextProvider>);
  return { ...result, state: () => current };
}
afterEach(() => cleanup());

describe('Agent capability transfer workspace', () => {
  test('shows enabled items and the full catalog together with explicit state', () => {
    const screen = mount(documentWith([read]));
    const enabled = within(screen.getByRole('region', { name: en.workbench.enabledCapabilities }));
    const all = within(screen.getByRole('region', { name: en.workbench.allCapabilities }));
    expect(enabled.getByRole('button', { name: 'View Read files details' })).toBeTruthy();
    expect(enabled.queryByRole('button', { name: 'View Read a webpage details' })).toBeNull();
    expect(all.getByRole('button', { name: 'View Read a webpage details' })).toBeTruthy();
    expect((all.getByRole('checkbox', { name: 'Add Read files' }) as HTMLInputElement).disabled).toBe(true);
    expect(screen.queryByRole('combobox')).toBeNull();
  });

  test('selection does not enable anything until move-in, then updates both panes', async () => {
    const screen = mount(documentWith([read]));
    fireEvent.click(screen.getByRole('checkbox', { name: 'Add Read a webpage' }));
    fireEvent.click(screen.getByRole('checkbox', { name: 'Add Read the knowledge base' }));
    expect(screen.state().enabled_capabilities).toHaveLength(1);
    fireEvent.click(screen.getByRole('button', { name: 'Move in (2)' }));
    await waitFor(() => expect(screen.state().enabled_capabilities).toHaveLength(3));
    expect((screen.getByRole('checkbox', { name: 'Add Read a webpage' }) as HTMLInputElement).disabled).toBe(true);
    fireEvent.click(screen.getByRole('button', { name: en.workbench.undo }));
    await waitFor(() => expect(screen.state().enabled_capabilities).toHaveLength(1));
    expect(screen.state().enabled_capabilities[0].action_allowlist).toEqual(selection(read).action_allowlist);
  });

  test('moves out selected capabilities and preserves unrelated selections', async () => {
    const screen = mount(documentWith([read, knowledge]));
    fireEvent.click(screen.getByRole('checkbox', { name: 'Select Read files' }));
    fireEvent.click(screen.getByRole('button', { name: 'Move out (1)' }));
    await waitFor(() => expect(screen.state().enabled_capabilities.map(row => row.capability.id)).toEqual([knowledge.capability.id]));
    expect((screen.getByRole('checkbox', { name: 'Add Read files' }) as HTMLInputElement).disabled).toBe(false);
  });

  test('changing the catalog filter clears hidden batch selections', async () => {
    const screen = mount(documentWith([read]));
    fireEvent.click(screen.getByRole('checkbox', { name: 'Add Read a webpage' }));
    fireEvent.change(screen.getByRole('searchbox', { name: en.workbench.searchLibrary }), { target: { value: 'knowledge' } });
    await waitFor(() => expect((screen.getByRole('button', { name: 'Move in' }) as HTMLButtonElement).disabled).toBe(true));
    const enabled = within(screen.getByRole('region', { name: en.workbench.enabledCapabilities }));
    expect(enabled.getByRole('button', { name: 'View Read files details' })).toBeTruthy();
    expect(screen.state().enabled_capabilities).toHaveLength(1);
  });

  test('unavailable items remain inspectable but cannot be enabled', () => {
    const screen = mount(documentWith([]));
    expect((screen.getByRole('checkbox', { name: 'Add Search the web' }) as HTMLInputElement).disabled).toBe(true);
    expect(screen.getByRole('button', { name: 'View Search the web details' })).toBeTruthy();
    expect((screen.getByRole('button', { name: 'Enable Search the web' }) as HTMLButtonElement).disabled).toBe(true);
  });

  test('a missing saved capability stays visible and can be disabled', async () => {
    const missing = item('plugin.missing');
    const screen = mount(documentWith([missing]));
    const left = within(screen.getByRole('region', { name: en.workbench.enabledCapabilities }));
    fireEvent.click(left.getByRole('button', { name: /^Disable / }));
    await waitFor(() => expect(screen.state().enabled_capabilities).toEqual([]));
  });
});
