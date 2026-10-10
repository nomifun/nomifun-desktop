import '../../../../test/setup-dom.ts';
import { afterEach, beforeAll, describe, expect, spyOn, test } from 'bun:test';
import { act, cleanup, fireEvent, render, waitFor, within } from '@testing-library/react';
import { useState } from 'react';
import { MemoryRouter, Route, Routes } from 'react-router-dom';
import { createInstance } from 'i18next';
import { I18nextProvider } from 'react-i18next';
import { Message } from '@arco-design/web-react';
import { ipcBridge } from '@/common';
import { mcpService } from '@/common/adapter/ipcBridge';
import type { IMcpServer } from '@/common/config/storage';
import { parseMcpServerId } from '@/common/types/ids';
import * as agentHooks from '@/renderer/hooks/agent/useAgents';
import * as themeContext from '@/renderer/hooks/context/ThemeContext';
import SessionCapabilityPicker, {
  type SessionCapabilityDraft,
  useSessionCapabilityCatalog,
} from '@/renderer/components/chat/SessionCapabilityPicker';
import settings from '@/renderer/services/i18n/locales/en-US/settings.json';
import common from '@/renderer/services/i18n/locales/en-US/common.json';
import conversation from '@/renderer/services/i18n/locales/en-US/conversation.json';
import McpPage from './index';
import { getMcpMarketOrigin } from './McpMarketSettings';

const locale = createInstance();
const restore: Array<() => void> = [];
const serverId = parseMcpServerId('0190f5fe-7c00-7a00-8000-000000000169');
const existingSkillName = 'existing-session-skill';
const clone = <T,>(value: T): T => structuredClone(value);
const track = <T extends { mockRestore: () => void }>(spy: T): T => {
  restore.push(() => spy.mockRestore());
  return spy;
};

beforeAll(async () => {
  await locale.init({
    lng: 'en-US',
    resources: { 'en-US': { translation: { settings, common, conversation } } },
  });
});

afterEach(() => {
  cleanup();
  restore.splice(0).reverse().forEach((dispose) => dispose());
});

function fixture() {
  // Keep the actual market, CRUD, management, catalog and picker mounted;
  // only their backend boundary and unrelated theme/agent consumers are stubbed.
  for (const [storage, key, value] of [
    [localStorage, 'nomifun.mcpMarket.rankings.v1', JSON.stringify({
      items: [{
        id: 'mcpworld:alpha',
        name: 'alpha',
        source: 'mcpworld',
        rank: 1,
        description: '',
        url: 'https://www.mcpworld.com/alpha',
        install_command: 'mcp market add mcpworld:alpha',
      }],
    })],
    [sessionStorage, 'nomifun.mcpMarket.autoSynced.v1', '1'],
  ] as const) {
    const previous = storage.getItem(key);
    storage.setItem(key, value);
    restore.push(() => previous === null ? storage.removeItem(key) : storage.setItem(key, previous));
  }

  let persisted: IMcpServer[] = [];
  let rejectToggleAfterPersist = false;
  let connectionResult: Awaited<ReturnType<typeof mcpService.testMcpConnection.invoke>> = {
    success: true,
    tools: [{ name: 'browse', description: 'Browse a page', input_schema: { type: 'object' } }],
  };
  track(spyOn(ipcBridge.fs.resolveSkillMarketMcpConfig, 'invoke').mockResolvedValue({
    config_json: { mcpServers: { alpha: { command: 'fixture-command', args: ['--fixture'] } } },
  }));
  const importing = track(spyOn(mcpService.importServers, 'invoke').mockImplementation(async ({ servers }) => {
    persisted = servers.map((server) => ({
      ...server,
      mcp_server_id: serverId,
      enabled: false,
      created_at: 1,
      updated_at: 1,
    }));
    return clone(persisted);
  }));
  const listing = track(spyOn(mcpService.listServers, 'invoke').mockImplementation(async () => clone(persisted)));
  const testing = track(spyOn(mcpService.testMcpConnection, 'invoke').mockImplementation(async (request) => {
    const result = clone(connectionResult);
    persisted = persisted.map((server) => server.mcp_server_id !== request.mcp_server_id ? server : {
      ...server,
      last_test_status: result.success ? 'connected' : 'error',
      tools: result.success ? result.tools : undefined,
      last_connected: result.success ? server.updated_at + 1 : undefined,
      updated_at: server.updated_at + 1,
    });
    return result;
  }));
  const toggling = track(spyOn(mcpService.toggleServer, 'invoke').mockImplementation(async ({ mcp_server_id }) => {
    persisted = persisted.map((server) => server.mcp_server_id !== mcp_server_id ? server : {
      ...server,
      enabled: !server.enabled,
      updated_at: server.updated_at + 1,
    });
    if (rejectToggleAfterPersist) {
      rejectToggleAfterPersist = false;
      throw new Error('Fixture response lost after saving toggle');
    }
    return clone(persisted.find((server) => server.mcp_server_id === mcp_server_id)!);
  }));
  track(spyOn(ipcBridge.fs.listAvailableSkills, 'invoke').mockResolvedValue([{
    name: existingSkillName,
    description: 'Existing instructions',
    location: '',
    source: 'custom',
    is_custom: true,
  }]));
  track(spyOn(ipcBridge.fs.listBuiltinAutoSkills, 'invoke').mockResolvedValue([]));
  track(spyOn(agentHooks, 'getAgents').mockResolvedValue([]));
  track(spyOn(themeContext, 'useThemeContext').mockReturnValue({
    theme: 'light', colorScheme: 'default', fontScale: 1,
    setTheme: async () => {}, setColorScheme: async () => {}, setFontScale: async () => {},
  }));
  const errors = track(spyOn(Message, 'error').mockReturnValue(() => {}));
  track(spyOn(Message, 'warning').mockReturnValue(() => {}));
  track(spyOn(Message, 'success').mockReturnValue(() => {}));

  function Session() {
    const { catalog, loading, error, retry } = useSessionCapabilityCatalog();
    const [draft, setDraft] = useState<SessionCapabilityDraft>({
      skillNames: [existingSkillName],
      mcpServerIds: [],
    });
    return <>
      <SessionCapabilityPicker catalog={catalog} loading={loading} loadFailed={Boolean(error)}
        onRetry={retry} draft={draft} onChange={setDraft} applyMode='create' />
      <output data-testid='session-draft'>{JSON.stringify(draft)}</output>
      <output data-testid='session-catalog'>{JSON.stringify(catalog.mcpServers)}</output>
    </>;
  }

  const mount = async () => {
    let view!: ReturnType<typeof render>;
    await act(async () => {
      view = render(<I18nextProvider i18n={locale}>
        <MemoryRouter initialEntries={['/mcp?tab=market']}>
          <Session />
          <Routes><Route path='/mcp' element={<McpPage />} /></Routes>
        </MemoryRouter>
      </I18nextProvider>);
    });
    // A single mount exercises catalog notifications, rather than reopening
    // the conversation or manually requesting fresh data after each action.
    const pickerTrigger = view.getByTestId('session-mcp-trigger');
    const draft = () => JSON.parse(view.getByTestId('session-draft').textContent!) as SessionCapabilityDraft;
    const catalog = () => JSON.parse(view.getByTestId('session-catalog').textContent!) as IMcpServer[];
    const picker = () => within(view.getByTestId('session-mcp-list'));
    const openPicker = () => {
      if (pickerTrigger.getAttribute('aria-expanded') !== 'true') fireEvent.click(pickerTrigger);
    };
    const checkbox = async () => {
      openPicker();
      return await view.findByRole('checkbox', { name: 'alpha' }) as HTMLInputElement;
    };
    const installed = () => within(view.getByTestId('mcp-installed-surface'));
    const check = async () => {
      await act(async () => fireEvent.click(installed().getByTitle(settings.mcpTestConnection)));
    };
    const toggle = async (enabled: boolean) => {
      await act(async () => fireEvent.click(installed().getByRole('switch')));
      await waitFor(() => expect(installed().getByRole('switch').getAttribute('aria-checked')).toBe(String(enabled)));
    };
    const importFromMarket = async () => {
      await act(async () => fireEvent.click(view.getByTestId('btn-add-market-skill-mcpworld-alpha')));
      await act(async () => fireEvent.click(view.getByRole('button', { name: settings.mcpMarket.confirmOk })));
      await view.findByTestId('mcp-installed-surface');
      await installed().findByRole('switch', { name: 'Enable alpha' });
      const item = await checkbox();
      expect(item.disabled).toBe(true);
      expect(picker().getByText(conversation.capabilityPicker.disabled)).toBeTruthy();
      expect(importing).toHaveBeenCalledTimes(1);
      expect(getMcpMarketOrigin(importing.mock.calls[0]![0].servers[0]!)).toBe('mcpworld:alpha');
      expect(persisted[0]!.enabled).toBe(false);
      expect(testing).not.toHaveBeenCalled();
      expect(toggling).not.toHaveBeenCalled();
      expect(draft()).toEqual({ skillNames: [existingSkillName], mcpServerIds: [] });
    };
    return { ...view, draft, catalog, picker, checkbox, installed, check, toggle, importFromMarket, pickerTrigger };
  };

  return {
    mount, importing, listing, testing, toggling, errors,
    failCheck: () => { connectionResult = { success: false, error: 'Fixture connection failed' }; },
    rejectNextToggleAfterPersist: () => { rejectToggleAfterPersist = true; },
    failCatalogReads: () => { listing.mockRejectedValue(new Error('Fixture catalog unavailable')); },
    deferCatalogReads: () => {
      let resolve!: (servers: IMcpServer[]) => void;
      const pending = new Promise<IMcpServer[]>((yes) => { resolve = yes; });
      listing.mockImplementation(() => pending);
      return () => {
        listing.mockImplementation(async () => clone(persisted));
        resolve(clone(persisted));
      };
    },
  };
}

describe('MCP market import to session selection with mounted catalogs', () => {
  test('import stays disabled; check then enable refreshes selection; disabling allows explicit removal', async () => {
    const f = fixture();
    const v = await f.mount();
    await v.importFromMarket();

    await v.check();
    await waitFor(() => expect(v.catalog()[0]?.last_test_status).toBe('connected'));
    expect(f.testing.mock.calls[0]![0].mcp_server_id).toBe(serverId);
    expect(v.catalog()[0]!.tools).toHaveLength(1);
    expect(v.catalog()[0]!.enabled).toBe(false);
    expect((await v.checkbox()).disabled).toBe(true);
    expect(f.toggling).not.toHaveBeenCalled();

    await v.toggle(true);
    await waitFor(async () => expect((await v.checkbox()).disabled).toBe(false));
    expect(v.picker().getByText(conversation.capabilityPicker.available)).toBeTruthy();
    expect(v.draft().mcpServerIds).toEqual([]);
    fireEvent.click(await v.checkbox());
    expect(v.draft()).toEqual({ skillNames: [existingSkillName], mcpServerIds: [serverId] });

    await v.toggle(false);
    await waitFor(() => expect(v.catalog()[0]!.enabled).toBe(false));
    expect(v.picker().getByText(conversation.capabilityPicker.disabled)).toBeTruthy();
    expect(v.draft().mcpServerIds).toEqual([serverId]);
    expect((await v.checkbox()).disabled).toBe(false);
    fireEvent.click(await v.checkbox());
    expect(v.draft()).toEqual({ skillNames: [existingSkillName], mcpServerIds: [] });
    expect((await v.checkbox()).disabled).toBe(true);
    expect(v.getByTestId('session-mcp-trigger')).toBe(v.pickerTrigger);
    expect(f.testing).toHaveBeenCalledTimes(1);
    expect(f.toggling).toHaveBeenCalledTimes(2);
  });

  test('enable before check waits for tool discovery; a failed recheck refreshes the picker and blocks reselection', async () => {
    const f = fixture();
    const v = await f.mount();
    await v.importFromMarket();

    await v.toggle(true);
    await waitFor(() => expect(v.catalog()[0]?.enabled).toBe(true));
    expect((await v.checkbox()).disabled).toBe(true);
    expect(v.picker().getByText(conversation.capabilityPicker.unavailable)).toBeTruthy();
    expect(f.testing).not.toHaveBeenCalled();

    await v.check();
    await waitFor(async () => expect((await v.checkbox()).disabled).toBe(false));
    fireEvent.click(await v.checkbox());
    expect(v.draft().mcpServerIds).toEqual([serverId]);

    f.failCheck();
    await v.check();
    await waitFor(() => expect(v.catalog()[0]?.last_test_status).toBe('error'));
    expect(v.catalog()[0]!.enabled).toBe(true);
    expect(v.picker().getByText(conversation.capabilityPicker.error)).toBeTruthy();
    expect(v.draft()).toEqual({ skillNames: [existingSkillName], mcpServerIds: [serverId] });
    fireEvent.click(await v.checkbox());
    expect((await v.checkbox()).disabled).toBe(true);
    expect(v.draft()).toEqual({ skillNames: [existingSkillName], mcpServerIds: [] });
    expect(f.toggling).toHaveBeenCalledTimes(1);
  });

  test('failed enable leaves a checked-but-disabled import unselectable and can be retried', async () => {
    const f = fixture();
    const v = await f.mount();
    await v.importFromMarket();
    await v.check();
    await waitFor(() => expect(v.catalog()[0]?.last_test_status).toBe('connected'));

    f.toggling.mockRejectedValueOnce(new Error('Fixture enable failed'));
    await act(async () => fireEvent.click(v.installed().getByRole('switch')));
    await waitFor(() => expect(f.errors).toHaveBeenCalled());
    expect(v.installed().getByRole('switch').getAttribute('aria-checked')).toBe('false');
    expect(v.catalog()[0]!.enabled).toBe(false);
    expect((await v.checkbox()).disabled).toBe(true);
    expect(v.draft()).toEqual({ skillNames: [existingSkillName], mcpServerIds: [] });

    await v.toggle(true);
    await waitFor(async () => expect((await v.checkbox()).disabled).toBe(false));
    fireEvent.click(await v.checkbox());
    expect(v.draft().mcpServerIds).toEqual([serverId]);
    expect(f.toggling).toHaveBeenCalledTimes(2);
  });

  test('a lost disable response still reloads the saved disabled state without clearing the selected draft', async () => {
    const f = fixture();
    const v = await f.mount();
    await v.importFromMarket();
    await v.check();
    await v.toggle(true);
    await waitFor(async () => expect((await v.checkbox()).disabled).toBe(false));
    fireEvent.click(await v.checkbox());

    f.rejectNextToggleAfterPersist();
    await act(async () => fireEvent.click(v.installed().getByRole('switch')));
    await waitFor(() => expect(v.catalog()[0]?.enabled).toBe(false));
    expect(f.errors).toHaveBeenCalled();
    expect(v.installed().getByRole('switch').getAttribute('aria-checked')).toBe('false');
    expect(v.picker().getByText(conversation.capabilityPicker.disabled)).toBeTruthy();
    expect(v.draft()).toEqual({ skillNames: [existingSkillName], mcpServerIds: [serverId] });
    expect((await v.checkbox()).disabled).toBe(false);
    fireEvent.click(await v.checkbox());
    expect((await v.checkbox()).disabled).toBe(true);
    fireEvent.click(await v.checkbox());
    expect(v.draft()).toEqual({ skillNames: [existingSkillName], mcpServerIds: [] });
    expect(f.testing).toHaveBeenCalledTimes(1);
  });

  test('failed catalog refresh and pending retry keep stale available choices blocked until a canonical read succeeds', async () => {
    track(spyOn(console, 'error').mockImplementation(() => {}));
    const f = fixture();
    const v = await f.mount();
    await v.importFromMarket();
    await v.check();
    await v.toggle(true);
    await waitFor(async () => expect((await v.checkbox()).disabled).toBe(false));

    f.failCatalogReads();
    await act(async () => fireEvent.click(v.installed().getByRole('switch')));
    await waitFor(() => expect(v.picker().getByText(conversation.capabilityPicker.loadFailed)).toBeTruthy());
    expect(v.catalog()[0]!.enabled).toBe(true);
    expect(v.picker().queryByRole('checkbox', { name: 'alpha' })).toBeNull();
    expect(v.draft()).toEqual({ skillNames: [existingSkillName], mcpServerIds: [] });

    const finishRead = f.deferCatalogReads();
    await act(async () => fireEvent.click(v.picker().getByRole('button', { name: common.retry })));
    expect(v.picker().getByText(conversation.capabilityPicker.loadFailed)).toBeTruthy();
    expect(v.picker().queryByRole('checkbox', { name: 'alpha' })).toBeNull();
    await act(async () => finishRead());

    await waitFor(() => expect(v.catalog()[0]?.enabled).toBe(false));
    expect((await v.checkbox()).disabled).toBe(true);
    expect(v.picker().getByText(conversation.capabilityPicker.disabled)).toBeTruthy();
    expect(v.draft()).toEqual({ skillNames: [existingSkillName], mcpServerIds: [] });
  });
});
