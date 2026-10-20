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
import settings from '../../services/i18n/locales/en-US/settings.json';
import AgentResourcePicker, {
  loadAgentResourceInventory,
  optionsForAgentResourceField,
  type AgentResourceInventory,
} from './AgentResourcePicker';
import type { AgentResourceSelectionValue } from '@/renderer/hooks/agent/agentResourceSelection';

const i18n = createInstance();
await i18n.use(initReactI18next).init({ lng: 'en-US', fallbackLng: 'en-US', resources: { 'en-US': { translation: { agentSettings: en, common, settings } } }, interpolation: { escapeValue: false } });

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
  const robotStatus = spyOn(ipcBridge.robot.onStatus, 'on').mockReturnValue(() => {});
  restores.push(() => robotStatus.mockRestore());
});
afterEach(() => { cleanup(); restores.splice(0).forEach((restore) => restore()); globalThis.fetch = realFetch; });

describe('Agent resource picker', () => {
  test('exposes connected Chrome as an explicit Browser binding and preserves it when the connection is released', async () => {
    let connection: { incarnation: string; state: 'connected'; chromium_major: number } | null = { incarnation: 'chrome-1', state: 'connected', chromium_major: 144 };
    const requests: Array<{ method: string; body?: unknown }> = [];
    globalThis.fetch = (async (_input, init) => {
      requests.push({ method: init?.method ?? 'GET', body: init?.body ? JSON.parse(String(init.body)) : undefined });
      if (init?.method === 'DELETE') connection = null;
      return new Response(JSON.stringify({ success: true, data: connection }), { headers: { 'Content-Type': 'application/json' } });
    }) as typeof fetch;
    const ready: boolean[] = [];
    function Harness() {
      const [value, setValue] = useState<AgentResourceSelectionValue>({ browser: 'attached-chrome' });
      return <><AgentResourcePicker requiredKinds={['browser']} capabilityIds={['browser']} value={value} onChange={setValue}
        onAdmissionReadinessChange={(state) => ready.push(state)} />
        <output data-testid='browser-selection'>{value.browser}</output></>;
    }
    const screen = render(<I18nextProvider i18n={i18n}><MemoryRouter><Harness /></MemoryRouter></I18nextProvider>);
    await screen.findByText(en.resources.browser.states.connected);
    await waitFor(() => expect(ready.at(-1)).toBe(true));
    expect(screen.getByRole('combobox', { name: 'Select Browser resource' }).textContent).toContain(en.resources.browser.attached);
    fireEvent.click(screen.getByText(en.resources.browser.disconnect));
    await screen.findByText(en.resources.browser.selectedUnavailable);
    await waitFor(() => expect(ready.at(-1)).toBe(false));
    expect(screen.getByText(en.resources.browser.connectionNeeded)).toBeTruthy();
    expect(screen.queryByText(en.resources.missingCount.replace('{{count}}', '0'))).toBeNull();
    expect(screen.getByTestId('browser-selection').textContent).toBe('attached-chrome');
    expect(requests).toEqual([{ method: 'GET', body: undefined }, { method: 'DELETE', body: { incarnation: 'chrome-1' } }]);
  });

  test('hides automatic Computer and default Browser setup while keeping session admission ready', async () => {
    const permissions = spyOn(ipcBridge.systemPermissions.get, 'invoke');
    restores.push(() => permissions.mockRestore());
    let browserReads = 0;
    globalThis.fetch = (async () => { browserReads += 1; throw new Error('must not preflight Chrome'); }) as typeof fetch;
    const states: boolean[] = [];
    const changes: AgentResourceSelectionValue[] = [];
    const screen = render(<I18nextProvider i18n={i18n}><MemoryRouter><AgentResourcePicker
      requiredKinds={['computer', 'browser']} capabilityIds={['computer', 'browser']}
      actionIds={['browser/navigate']} value={{}} onChange={(value) => changes.push(value)}
      onAdmissionReadinessChange={(ready) => states.push(ready)} hideAutomaticBrowser
    /></MemoryRouter></I18nextProvider>);
    await waitFor(() => expect(states.at(-1)).toBe(true));
    expect(screen.container.querySelector('section')).toBeNull();
    expect(permissions).not.toHaveBeenCalled();
    expect(browserReads).toBe(0);
    expect(changes).toEqual([]);
  });

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

  test('does not block launch when an optional enhancement has no bound resource', async () => {
    const states: boolean[] = [];
    const screen = render(<I18nextProvider i18n={i18n}><MemoryRouter><AgentResourcePicker
      requiredKinds={['robot']} optionalKinds={['robot']} capabilityIds={['robot']}
      actionIds={['robot/vision']} value={{}} onChange={() => undefined}
      onAdmissionReadinessChange={(ready) => states.push(ready)}
      loadInventory={async () => ({ options: { robot: [] }, errors: {} })}
    /></MemoryRouter></I18nextProvider>);
    await waitFor(() => expect(states.at(-1)).toBe(true));
    expect(screen.container.textContent).toContain(common.optional);
    expect(screen.getByText(en.resources.ready)).toBeTruthy();
    expect(screen.getByText(en.resources.emptyOptional)).toBeTruthy();
  });

  test('joins live Robot status and exact Action permissions before offering a device', async () => {
    const list = spyOn(ipcBridge.robot.list, 'invoke').mockResolvedValue([
      {
        robot_id: 'robot-ready', name: 'Mochi bot', companion_id: null,
        board: 'esp32-s3', firmware_version: '1.0.0', last_seen: null, capability_revision: 0,
        created_at: '2026-09-18T00:00:00Z', supported_permissions: ['vision', 'motion'],
        permissions: { vision: true, motion: true, display: false, device_tools: false, proactive_speech: false, continuous_vision: false },
      },
      {
        robot_id: 'robot-blocked', name: 'Roux bot', companion_id: null,
        board: 'esp32-s3', firmware_version: '1.0.0', last_seen: null, capability_revision: 0,
        created_at: '2026-09-18T00:00:00Z', supported_permissions: ['vision'],
        permissions: { vision: true, motion: false, display: false, device_tools: false, proactive_speech: false, continuous_vision: false },
      },
    ]);
    const statuses = spyOn(ipcBridge.robot.statuses, 'invoke').mockResolvedValue([
      { robot_id: 'robot-ready', companion_id: null, phase: 'idle', changed_at: 1 },
      { robot_id: 'robot-blocked', companion_id: null, phase: 'offline', changed_at: 1 },
    ]);
    restores.push(() => list.mockRestore(), () => statuses.mockRestore());

    const loaded = await loadAgentResourceInventory(
      ['robot'],
      new Set(['robot']),
      new Set(['robot/vision', 'robot/motion'])
    );
    expect(loaded.options.robot).toEqual([
      expect.objectContaining({
        value: 'robot-ready', selectable: true, robotPhase: 'idle',
        robotRequiredPermissions: ['vision', 'motion'], robotDisabledPermissions: [],
        robotUnsupportedPermissions: [],
      }),
      expect.objectContaining({
        value: 'robot-blocked', selectable: false, robotPhase: 'offline',
        robotUnsupportedPermissions: ['motion'], robotDisabledPermissions: [],
      }),
    ]);
  });

  test('shows unavailable Robot guidance and only accepts a connected permitted device', async () => {
    const robotInventory: AgentResourceInventory = { options: { robot: [
      { value: 'offline', label: 'Offline bot', description: 'esp32', selectable: false,
        robotPhase: 'offline', robotRequiredPermissions: ['vision'], robotDisabledPermissions: [], robotUnsupportedPermissions: [] },
      { value: 'ready', label: 'Ready bot', description: 'esp32', selectable: true,
        robotPhase: 'idle', robotRequiredPermissions: ['vision'], robotDisabledPermissions: [], robotUnsupportedPermissions: [] },
    ] }, errors: {} };
    function Harness() {
      const [value, setValue] = useState<AgentResourceSelectionValue>({});
      return <><AgentResourcePicker requiredKinds={['robot']} capabilityIds={['robot']}
        actionIds={['robot/vision']} value={value} onChange={setValue}
        loadInventory={async () => robotInventory} />
        <output data-testid='robot-selection'>{value.robot}</output></>;
    }
    const screen = render(<I18nextProvider i18n={i18n}><MemoryRouter><Harness /></MemoryRouter></I18nextProvider>);
    const select = screen.getByRole('combobox', { name: 'Select Robot' });
    await waitFor(() => expect(select.getAttribute('aria-disabled')).not.toBe('true'));
    fireEvent.click(select);
    const offline = await screen.findByText('Offline bot');
    expect(offline.closest('.arco-select-option-disabled')).toBeTruthy();
    expect(screen.getByText(/Offline — Robot actions are unavailable/)).toBeTruthy();
    fireEvent.click(screen.getByText('Ready bot'));
    await waitFor(() => expect(screen.getByTestId('robot-selection').textContent).toBe('ready'));
  });

  test('retains a selected Robot that goes offline without blocking the Agent session', async () => {
    const states: boolean[] = [];
    const robotInventory: AgentResourceInventory = { options: { robot: [
      { value: 'offline', label: 'Offline bot', selectable: false, robotPhase: 'offline',
        robotRequiredPermissions: ['vision'], robotDisabledPermissions: [], robotUnsupportedPermissions: [] },
    ] }, errors: {} };
    const screen = render(<I18nextProvider i18n={i18n}><MemoryRouter><AgentResourcePicker
      requiredKinds={['robot']} capabilityIds={['robot']} actionIds={['robot/vision']}
      value={{ robot: 'offline' }} onChange={() => { throw new Error('live status must not erase the binding'); }}
      onAdmissionReadinessChange={(ready) => states.push(ready)} loadInventory={async () => robotInventory}
    /></MemoryRouter></I18nextProvider>);
    await waitFor(() => expect(screen.getByRole('combobox', { name: 'Select Robot' }).textContent).toContain('Offline bot'));
    await waitFor(() => expect(states.at(-1)).toBe(true));
    expect(screen.getByText(en.resources.ready)).toBeTruthy();
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

  test('shows every Knowledge base selected for the frozen Session', async () => {
    const screen = render(<I18nextProvider i18n={i18n}><MemoryRouter><AgentResourcePicker
      requiredKinds={['knowledge_base']}
      optionalKinds={['knowledge_base']}
      capabilityIds={['knowledge']}
      actionIds={['knowledge/search', 'knowledge/read']}
      value={{ knowledge_bases: ['kb-1', 'kb-2'] }}
      onChange={() => undefined}
      loadInventory={async () => inventory}
    /></MemoryRouter></I18nextProvider>);
    await waitFor(() => expect(screen.getByText('Resources ready')).toBeTruthy());
    const picker = screen.getByRole('combobox', { name: 'Select Knowledge base' });
    expect(picker.textContent?.includes('Support handbook')).toBe(true);
    expect(picker.textContent?.includes('Private notes')).toBe(true);
  });

  test('offers the product configuration route when a required resource has no options', async () => {
    const Location = () => <span data-testid='location'>{useLocation().pathname}</span>;
    const screen = render(<I18nextProvider i18n={i18n}><MemoryRouter initialEntries={['/guid']}><AgentResourcePicker
      requiredKinds={['knowledge_base']}
      capabilityIds={['knowledge/search']}
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
