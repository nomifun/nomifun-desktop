import '../../../../test/setup-dom.ts';
import { act, cleanup, fireEvent, render, waitFor } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, spyOn, test } from 'bun:test';
import { ipcBridge } from '@/common';
import { createInstance } from 'i18next';
import { useState } from 'react';
import { I18nextProvider, initReactI18next } from 'react-i18next';
import { MemoryRouter, useLocation } from 'react-router-dom';
import en from '../../services/i18n/locales/en-US/agentSettings.json';
import common from '../../services/i18n/locales/en-US/common.json';
import AgentResourcePicker, {
  loadAgentResourceInventory,
  optionsForAgentResourceField,
  type AgentResourceInventory,
} from './AgentResourcePicker';
import type { AgentResourceSelectionValue } from '@/renderer/hooks/agent/agentResourceSelection';

const i18n = createInstance();
await i18n.use(initReactI18next).init({ lng: 'en-US', fallbackLng: 'en-US', resources: { 'en-US': { translation: { agentSettings: en, common } } }, interpolation: { escapeValue: false } });

const inventory: AgentResourceInventory = {
  options: {
    companion: [
      { value: 'companion-1', label: 'Mochi' },
      { value: 'companion-2', label: 'Roux' },
    ],
    channel: [
      { value: 'channel-1', label: 'Mochi Telegram', ownerDomain: 'companion', companionId: 'companion-1' },
      { value: 'channel-2', label: 'Roux Lark', ownerDomain: 'companion', companionId: 'companion-2' },
      { value: 'channel-3', label: 'Support Bot', ownerDomain: 'customer_service' },
    ],
    customer: [
      { value: 'customer-1', label: 'Support', channelIds: ['channel-3'], knowledgeBaseIds: ['kb-1'] },
    ],
    knowledge_base: [
      { value: 'kb-1', label: 'Support handbook' },
      { value: 'kb-2', label: 'Private notes' },
    ],
  },
  errors: {},
};

const realFetch = globalThis.fetch;
const restores: (() => void)[] = [];
beforeEach(() => {
  for (const event of [ipcBridge.companion.onCompanionCreated, ipcBridge.companion.onCompanionDeleted]) {
    const spy = spyOn(event, 'on').mockReturnValue(() => {});
    restores.push(() => spy.mockRestore());
  }
});
afterEach(() => { cleanup(); restores.splice(0).forEach((restore) => restore()); globalThis.fetch = realFetch; });

describe('Agent resource picker', () => {
  test('a pointer click opens the companion menu without native label activation closing it', async () => {
    const loader = async () => inventory;
    function Harness() {
      const [value, setValue] = useState<AgentResourceSelectionValue>({});
      return <><AgentResourcePicker requiredKinds={['companion']} capabilityIds={[]} value={value} onChange={setValue} loadInventory={loader} />
        <output data-testid='selection'>{value.companion}</output></>;
    }
    const screen = render(<I18nextProvider i18n={i18n}><MemoryRouter><Harness /></MemoryRouter></I18nextProvider>);
    const select = screen.getByRole('combobox', { name: 'Select Companion' });
    await waitFor(() => expect(select.getAttribute('aria-disabled')).not.toBe('true'));
    // Happy DOM's fireEvent does not implement the browser's native label
    // activation. Reproduce Chromium's second click on the implicit control.
    const target = select.querySelector('.arco-select-view-value-mirror')!;
    const label = target.closest('label');
    fireEvent.mouseDown(target);
    fireEvent.mouseUp(target);
    fireEvent.click(target);
    if (label?.control) fireEvent.click(label.control);
    // Arco closes via a deferred transition; wait past it before choosing.
    await act(async () => { await new Promise((resolve) => setTimeout(resolve, 400)); });
    expect(select.getAttribute('aria-expanded')).toBe('true');
    fireEvent.click(await screen.findByText('Mochi'));
    await waitFor(() => expect(screen.getByTestId('selection').textContent).toBe('companion-1'));
  });

  test('keeps companion selection usable while optional MCP inventory is still loading', async () => {
    let finishMcp!: (value: AgentResourceInventory) => void;
    const mcp = new Promise<AgentResourceInventory>((resolve) => { finishMcp = resolve; });
    const loader = async (kinds: readonly string[]) => kinds.includes('mcp_server') ? mcp : inventory;
    function Harness() {
      const [value, setValue] = useState<AgentResourceSelectionValue>({});
      return <><AgentResourcePicker requiredKinds={['companion']} optionalKinds={['mcp_server', 'channel', 'robot']}
        companionBindings capabilityIds={['mcp.connect', 'mcp.tool_proxy']} value={value} onChange={setValue} loadInventory={loader} />
        <output data-testid='selection'>{value.companion}</output></>;
    }
    const screen = render(<I18nextProvider i18n={i18n}><MemoryRouter><Harness /></MemoryRouter></I18nextProvider>);
    const select = screen.getByRole('combobox', { name: 'Select Companion' });
    await waitFor(() => expect(select.getAttribute('aria-disabled')).not.toBe('true'));
    fireEvent.click(select);
    fireEvent.click(await screen.findByText('Mochi'));
    await waitFor(() => expect(screen.getByTestId('selection').textContent).toBe('companion-1'));
    expect(screen.getByText('Resources ready')).toBeTruthy();
    await act(async () => { finishMcp({ options: { mcp_server: [] }, errors: {} }); });
  });

  test('loads enabled Plugin products with surfaces through the runtime library contract', async () => {
    const calls: string[] = [];
    const plugin = {
      plugin_id: '0190f5fe-7c00-7a00-8000-000000000001', product_revision: 1,
      display_name: 'Workspace panel', description: 'Installed Plugin', kind: 'plugin',
      lifecycle: 'enabled', surface_available: true, updated_at_ms: 1,
      releases: { pointer_revision: 1, active_release_epoch: 1 },
      service_health: { state: 'not_applicable' },
    };
    globalThis.fetch = (async (input) => {
      calls.push(new URL(String(input), 'http://127.0.0.1').pathname);
      return new Response(JSON.stringify({ success: true, data: { library_revision: 1, plugins: [
        plugin,
        { ...plugin, plugin_id: '0190f5fe-7c00-7a00-8000-000000000002', lifecycle: 'disabled' },
        { ...plugin, plugin_id: '0190f5fe-7c00-7a00-8000-000000000003', surface_available: false },
      ] } }), { headers: { 'Content-Type': 'application/json' } });
    }) as typeof fetch;
    const inventory = await loadAgentResourceInventory(['plugin'], new Set(['plugin.surface']));
    expect(calls).toEqual(['/api/plugins/runtimes']);
    expect(inventory.errors).toEqual({});
    expect(inventory.options.plugin).toEqual([
      { value: plugin.plugin_id, label: plugin.display_name, description: plugin.description },
    ]);
  });

  test('filters dependent resources by the selected product owner', () => {
    expect(optionsForAgentResourceField('channel', inventory, { companion: 'companion-1' }, new Set(['companion', 'channel'])).map((option) => option.value)).toEqual(['channel-1']);
    expect(optionsForAgentResourceField('channel', inventory, { customer: 'customer-1' }, new Set(['customer', 'channel'])).map((option) => option.value)).toEqual(['channel-3']);
    expect(optionsForAgentResourceField('knowledge_base', inventory, { customer: 'customer-1' }, new Set(['customer', 'knowledge_base'])).map((option) => option.value)).toEqual(['kb-1']);
  });

  test('uses labeled selects rather than UUID or JSON inputs and accepts one companion choice for memory', async () => {
    const Harness = () => {
      const [value, setValue] = useState<AgentResourceSelectionValue>({ companion: 'companion-1', channel: 'channel-1' });
      return <AgentResourcePicker
        requiredKinds={['companion', 'companion_memory', 'channel']}
        capabilityIds={[]}
        value={value}
        onChange={setValue}
        loadInventory={async () => inventory}
      />;
    };
    const screen = render(<I18nextProvider i18n={i18n}><MemoryRouter><Harness /></MemoryRouter></I18nextProvider>);
    await waitFor(() => expect(screen.getByText('Resources ready')).toBeTruthy());
    expect(screen.container.querySelector('textarea')).toBeNull();
    expect(screen.container.querySelector('input[type="text"]')).toBeNull();
    expect(screen.getByRole('combobox', { name: 'Select Companion' }).textContent?.includes('Mochi')).toBe(true);
    expect(screen.getByRole('combobox', { name: 'Select Channel' }).textContent?.includes('Mochi Telegram')).toBe(true);
  });

  test('offers the product configuration route when a required resource has no options', async () => {
    const Location = () => <span data-testid='location'>{useLocation().pathname}</span>;
    const screen = render(<I18nextProvider i18n={i18n}><MemoryRouter initialEntries={['/guid']}><AgentResourcePicker
      requiredKinds={['knowledge_base']}
      capabilityIds={['knowledge.search']}
      value={{}}
      onChange={() => undefined}
      loadInventory={async () => ({ options: { knowledge_base: [] }, errors: {} })}
    /><Location /></MemoryRouter></I18nextProvider>);

    const configure = (await screen.findByText(en.resources.configure)).closest('button');
    expect(configure).toBeTruthy();
    expect(screen.getByText(en.resources.emptyOptions)).toBeTruthy();
    fireEvent.click(configure!);
    await waitFor(() => expect(screen.getByTestId('location').textContent).toBe('/knowledge'));
  });
});
