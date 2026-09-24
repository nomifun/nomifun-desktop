import { afterEach, beforeAll, describe, expect, mock, spyOn, test } from 'bun:test';
import { act, cleanup, fireEvent, render, waitFor, within } from '@testing-library/react';
import { createElement as h } from 'react';
import { MemoryRouter, Route, Routes, useLocation, useNavigate } from 'react-router-dom';
import { createInstance } from 'i18next';
import { I18nextProvider } from 'react-i18next';
import { Message } from '@arco-design/web-react';
import { ipcBridge } from '@/common';
import { mcpService, type ISkillMarketMcpConfigResponse } from '@/common/adapter/ipcBridge';
import type { IMcpServer } from '@/common/config/storage';
import { parseMcpServerId } from '@/common/types/ids';
import * as agentHooks from '@/renderer/hooks/agent/useAgents';
import * as themeContext from '@/renderer/hooks/context/ThemeContext';
import settings from '@/renderer/services/i18n/locales/en-US/settings.json';
import common from '@/renderer/services/i18n/locales/en-US/common.json';
import McpPage from './index';
import McpMarketSettings, {
  attachMcpMarketOrigin,
  getMcpMarketItemServers,
  getMcpMarketOrigin,
  isLocalMcpEndpoint,
  isMcpMarketItemInstalled,
} from './McpMarketSettings';

const marketItem = {
  id: 'skillhub_mcp:playwright',
  name: 'Playwright MCP',
};

const server = (name: string, original_json: string): IMcpServer => ({ name, original_json }) as IMcpServer;

describe('MCP market installed state', () => {
  test('distinguishes local-service descriptors from remote MCP endpoints', () => {
    expect(
      isLocalMcpEndpoint({
        type: 'http',
        url: 'http://localhost:18060/mcp',
        headers: {},
      })
    ).toBe(true);
    expect(
      isLocalMcpEndpoint({
        type: 'sse',
        url: 'http://127.0.0.1:8080/sse',
        headers: {},
      })
    ).toBe(true);
    expect(
      isLocalMcpEndpoint({
        type: 'http',
        url: 'https://mcp.example.com/mcp',
        headers: {},
      })
    ).toBe(false);
    expect(isLocalMcpEndpoint({ type: 'stdio', command: 'npx', args: [], env: {} })).toBe(false);
  });

  test('persists exact market provenance inside the server original JSON', () => {
    const original = JSON.stringify({
      mcpServers: { browser: { command: 'npx' } },
    });
    const marked = attachMcpMarketOrigin(original, marketItem.id);
    const installed = server('browser', marked);

    expect(JSON.parse(marked).mcpServers.browser.command).toBe('npx');
    expect(getMcpMarketOrigin(installed)).toBe(marketItem.id);
    expect(isMcpMarketItemInstalled(marketItem, [installed])).toBe(true);
    expect(getMcpMarketItemServers(marketItem, [installed])).toEqual([installed]);
    expect(isMcpMarketItemInstalled(marketItem, [])).toBe(false);
  });

  test('recognizes legacy imports by server name or market slug', () => {
    expect(isMcpMarketItemInstalled(marketItem, [server('playwright', '{}')])).toBe(true);
    expect(isMcpMarketItemInstalled(marketItem, [server('another-server', '{}')])).toBe(false);
    expect(getMcpMarketOrigin(server('broken', '{'))).toBeNull();
  });
});

const locale = createInstance();
const restore: Array<() => void> = [];
beforeAll(async () => {
  await locale.init({
    lng: 'en-US',
    resources: { 'en-US': { translation: { settings, common } } },
  });
});
afterEach(() => {
  cleanup();
  restore
    .splice(0)
    .reverse()
    .forEach((dispose) => dispose());
});

function deferred<T>() {
  let resolve!: (value: T) => void;
  let reject!: (reason: unknown) => void;
  const promise = new Promise<T>((yes, no) => {
    resolve = yes;
    reject = no;
  });
  return { promise, resolve, reject };
}

const resolvedConfig = (name: string): ISkillMarketMcpConfigResponse => ({
  config_json: {
    mcpServers: { [name]: { command: 'fixture-command', args: ['--fixture'] } },
  },
});
const persistedServer = (name = 'alpha'): IMcpServer => ({
  mcp_server_id: parseMcpServerId('0190f5fe-7c00-7a00-8000-000000000069'),
  name,
  enabled: false,
  original_json: '{}',
  created_at: 1,
  updated_at: 1,
  transport: { type: 'stdio', command: 'fixture-command', args: ['--fixture'] },
});

function fixture() {
  // Touch only this market's cache keys; no global fetch/module/i18n mocks.
  for (const [storage, key, value] of [
    [
      localStorage,
      'nomifun.mcpMarket.rankings.v1',
      JSON.stringify({
        items: ['alpha', 'beta'].map((name, rank) => ({
          id: 'mcpworld:' + name,
          name,
          source: 'mcpworld',
          rank: rank + 1,
          description: '',
          url: 'https://www.mcpworld.com/' + name,
          install_command: 'mcp market add mcpworld:' + name,
        })),
      }),
    ],
    [sessionStorage, 'nomifun.mcpMarket.autoSynced.v1', '1'],
  ] as const) {
    const previous = storage.getItem(key);
    storage.setItem(key, value);
    restore.push(() => (previous === null ? storage.removeItem(key) : storage.setItem(key, previous)));
  }
  const resolutions: Array<ReturnType<typeof deferred<ISkillMarketMcpConfigResponse>>> = [];
  const imports: Array<ReturnType<typeof deferred<IMcpServer[]>>> = [];
  const catalog = deferred<IMcpServer[]>();
  const resolving = spyOn(ipcBridge.fs.resolveSkillMarketMcpConfig, 'invoke').mockImplementation(() => {
    const request = deferred<ISkillMarketMcpConfigResponse>();
    resolutions.push(request);
    return request.promise;
  });
  const importing = spyOn(mcpService.importServers, 'invoke').mockImplementation(() => {
    const request = deferred<IMcpServer[]>();
    imports.push(request);
    return request.promise;
  });
  const updating = spyOn(mcpService.updateServer, 'invoke').mockResolvedValue(persistedServer());
  const listing = spyOn(mcpService.listServers, 'invoke').mockImplementation(() => catalog.promise);
  const testing = spyOn(mcpService.testMcpConnection, 'invoke').mockResolvedValue({ success: true });
  const toggling = spyOn(mcpService.toggleServer, 'invoke').mockResolvedValue(persistedServer());
  const errors = spyOn(Message, 'error').mockReturnValue(() => {});
  const warnings = spyOn(Message, 'warning').mockReturnValue(() => {});
  const successes = spyOn(Message, 'success').mockReturnValue(() => {});
  const logging = spyOn(console, 'error').mockImplementation(() => {});
  // The installed editor's unrelated agent and theme consumers stay isolated.
  const agents = spyOn(agentHooks, 'getAgents').mockResolvedValue([]);
  const theme = spyOn(themeContext, 'useThemeContext').mockReturnValue({
    theme: 'light',
    colorScheme: 'default',
    fontScale: 1,
    setTheme: async () => {},
    setColorScheme: async () => {},
    setFontScale: async () => {},
  });
  for (const spy of [
    resolving,
    importing,
    updating,
    listing,
    testing,
    toggling,
    errors,
    warnings,
    successes,
    logging,
    agents,
    theme,
  ]) {
    restore.push(() => spy.mockRestore());
  }
  const save = mock(async (_servers: IMcpServer[] | ((previous: IMcpServer[]) => IMcpServer[])) => {});
  let navigate!: ReturnType<typeof useNavigate>;
  const Probe = () => {
    navigate = useNavigate();
    const location = useLocation();
    return h('output', { 'data-testid': 'path' }, location.pathname + location.search);
  };
  const mount = async (page = false, path = '/mcp?tab=market', installedServers: IMcpServer[] = []) => {
    let view!: ReturnType<typeof render>;
    await act(async () => {
      view = render(
        h(
          I18nextProvider,
          { i18n: locale },
          h(
            MemoryRouter,
            { initialEntries: [path] },
            h(Probe),
            h(
              Routes,
              null,
              h(Route, {
                path: '/mcp',
                element: page
                  ? h(McpPage)
                  : h(McpMarketSettings, {
                      mcpServers: installedServers,
                      saveMcpServers: save,
                    }),
              }),
              h(Route, { path: '*', element: h('div', null, 'Other page') })
            )
          )
        )
      );
    });
    const add = (name: string) => fireEvent.click(view.getByTestId('btn-add-market-skill-mcpworld-' + name));
    const confirm = () => fireEvent.click(view.getByRole('button', { name: settings.mcpMarket.confirmOk }));
    const cancel = () => fireEvent.click(view.getByRole('button', { name: common.cancel }));
    const preview = async () => {
      add('alpha');
      await act(async () => {
        resolutions.at(-1)!.resolve(resolvedConfig('alpha'));
      });
    };
    return { ...view, add, confirm, cancel, preview };
  };
  return {
    mount,
    resolutions,
    imports,
    catalog,
    resolving,
    importing,
    updating,
    testing,
    toggling,
    errors,
    warnings,
    successes,
    save,
    go: (path: string) => navigate(path),
  };
}

describe('MCP market request ownership with the real panel and CRUD hook', () => {
  test('a slower older resolve cannot overwrite the latest preview', async () => {
    const f = fixture();
    const v = await f.mount();
    v.add('alpha');
    v.add('beta');
    await act(async () => {
      f.resolutions[1]!.resolve(resolvedConfig('beta'));
    });
    await act(async () => {
      f.resolutions[0]!.resolve(resolvedConfig('alpha'));
    });
    const dialog = within(v.getByRole('dialog'));
    expect(dialog.queryByText('alpha') === null).toBe(true);
    expect(dialog.getByText('beta')).toBeTruthy();
  });

  test('cancelling the latest preview invalidates older pending resolves', async () => {
    const f = fixture();
    const v = await f.mount();
    v.add('alpha');
    v.add('beta');
    await act(async () => {
      f.resolutions[1]!.resolve(resolvedConfig('beta'));
    });
    v.cancel();
    await act(async () => {
      f.resolutions[0]!.resolve(resolvedConfig('alpha'));
    });
    // Arco retains the closing dialog during its exit animation; no resolved
    // transport may reappear inside it.
    expect(v.queryByText('fixture-command') === null).toBe(true);
    expect(f.importing).not.toHaveBeenCalled();
  });

  test.each(['newer', 'unmount'] as const)('obsolete resolve errors are silent after %s', async (reason) => {
    const f = fixture();
    const v = await f.mount();
    v.add('alpha');
    if (reason === 'unmount') v.unmount();
    else {
      v.add('beta');
      await act(async () => {
        f.resolutions[1]!.resolve(resolvedConfig('beta'));
      });
    }
    await act(async () => {
      f.resolutions[0]!.reject(new Error('fixture offline'));
    });
    expect(f.errors).not.toHaveBeenCalled();
  });

  test('current resolve errors and empty configs release Add for retry', async () => {
    const f = fixture();
    const v = await f.mount();
    v.add('alpha');
    await act(async () => {
      f.resolutions[0]!.reject(new Error('fixture offline'));
    });
    expect(f.errors).toHaveBeenLastCalledWith(settings.mcpMarket.addFailed);
    v.add('alpha');
    await act(async () => {
      f.resolutions[1]!.resolve({ config_json: {} });
    });
    expect(f.errors).toHaveBeenLastCalledWith(settings.mcpMarket.configMissing);
    await v.preview();
    expect(v.getByRole('dialog')).toBeTruthy();
  });

  test('confirmation dispatches once, persists provenance and stays disabled without testing', async () => {
    const f = fixture();
    const v = await f.mount();
    await v.preview();
    act(() => {
      v.confirm();
      v.confirm();
    });
    const count = f.imports.length;
    await act(async () => {
      for (const request of f.imports) request.resolve([persistedServer()]);
    });
    expect(count).toBe(1);
    expect(getMcpMarketOrigin(f.importing.mock.calls[0]![0].servers[0]!)).toBe('mcpworld:alpha');
    const saved = f.save.mock.calls[0]![0];
    expect(typeof saved === 'function' ? saved([])[0]!.enabled : null).toBe(false);
    expect(f.testing).not.toHaveBeenCalled();
    expect(f.toggling).not.toHaveBeenCalled();
    expect(f.warnings).toHaveBeenCalledTimes(1);
    expect(v.getByTestId('path').textContent).toBe('/mcp');
  });

  test('failed installed market entries can replace their config without creating duplicates', async () => {
    const f = fixture();
    const broken = {
      ...persistedServer('alpha'),
      last_test_status: 'error' as const,
      original_json: attachMcpMarketOrigin('{}', 'mcpworld:alpha'),
    };
    const v = await f.mount(false, '/mcp?tab=market', [broken]);

    expect(v.getByTestId('btn-add-market-skill-mcpworld-alpha').textContent).toContain(settings.mcpMarket.repair);
    v.add('alpha');
    await act(async () => {
      f.resolutions[0]!.resolve(resolvedConfig('upstream-renamed'));
    });
    fireEvent.click(v.getByRole('button', { name: settings.mcpMarket.repairOk }));
    await waitFor(() => expect(f.updating).toHaveBeenCalledTimes(1));

    expect(f.importing).not.toHaveBeenCalled();
    expect(f.updating.mock.calls[0]![0].mcp_server_id).toBe(broken.mcp_server_id);
    expect(f.updating.mock.calls[0]![0].data.name).toBe('alpha');
    await waitFor(() => expect(v.getByTestId('path').textContent).toBe('/mcp'));
  });

  test.each(['cancel', 'leave'] as const)('late import after %s cannot notify or navigate', async (action) => {
    const f = fixture();
    const v = await f.mount();
    await v.preview();
    v.confirm();
    if (action === 'cancel') v.cancel();
    else
      await act(async () => {
        f.go('/elsewhere');
      });
    await act(async () => {
      f.imports[0]!.resolve([persistedServer()]);
    });
    expect(f.warnings).not.toHaveBeenCalled();
    expect(v.getByTestId('path').textContent).toBe(action === 'cancel' ? '/mcp?tab=market' : '/elsewhere');
    // Dismissal does not cancel an already dispatched backend write.
    expect(f.save).toHaveBeenCalledTimes(1);
  });

  test('cancel during import keeps Add blocked until the write settles', async () => {
    const f = fixture();
    const v = await f.mount();
    await v.preview();
    v.confirm();
    v.cancel();
    v.add('beta');
    const before = f.resolutions.length;
    await act(async () => {
      f.imports[0]!.resolve([persistedServer()]);
    });
    expect(before).toBe(1);
    v.add('beta');
    expect(f.resolutions).toHaveLength(2);
    await act(async () => {
      f.resolutions[1]!.resolve(resolvedConfig('beta'));
    });
  });

  test('import failure leaves the preview available for retry', async () => {
    const f = fixture();
    const v = await f.mount();
    await v.preview();
    v.confirm();
    await act(async () => {
      f.imports[0]!.reject(new Error('fixture write failed'));
    });
    expect(f.errors).toHaveBeenCalledTimes(1);
    expect(v.getByRole('dialog')).toBeTruthy();
    v.confirm();
    expect(f.imports).toHaveLength(2);
    await act(async () => {
      f.imports[1]!.resolve([persistedServer()]);
    });
    expect(v.getByTestId('path').textContent).toBe('/mcp');
  });
});

describe('MCP page initial catalog boundary', () => {
  test('switching to installed discards market review ownership and preserves other query params', async () => {
    const f = fixture();
    const v = await f.mount(true, '/mcp?tab=market&keep=1');
    await act(async () => {
      f.catalog.resolve([]);
    });
    expect(v.queryByTestId('mcp-page-header-actions')).toBeNull();
    v.add('alpha');
    fireEvent.click(v.getByRole('tab', { name: settings.mcpPage.installedMcpTab }));
    expect(v.getByTestId('mcp-page-header-actions')).toBeTruthy();
    await act(async () => {
      f.resolutions[0]!.resolve(resolvedConfig('alpha'));
    });
    expect(v.getByTestId('path').textContent).toBe('/mcp?keep=1');
    expect(v.queryByRole('dialog') === null).toBe(true);
  });

  test('installed editor is unavailable until the initial GET succeeds', async () => {
    const f = fixture();
    const v = await f.mount(true, '/mcp');
    const prematureAdd = v.queryByRole('button', {
      name: settings.mcpAddServer,
    });
    await act(async () => {
      f.catalog.resolve([]);
    });
    expect(prematureAdd === null).toBe(true);
    const addButton = v.getByRole('button', { name: settings.mcpAddServer });
    expect(within(v.getByTestId('mcp-page-header-actions')).getByRole('button', {
      name: settings.mcpAddServer,
    })).toBe(addButton);
    const installedSurface = within(v.getByTestId('mcp-installed-surface'));
    expect(installedSurface.queryByText(settings.mcpSettings)).toBeNull();
    expect(installedSurface.queryByRole('button', { name: settings.mcpAddServer })).toBeNull();
  });

  test('installed MCP servers paginate ten at a time', async () => {
    const f = fixture();
    const v = await f.mount(true, '/mcp');
    const servers = Array.from({ length: 12 }, (_, index) => ({
      ...persistedServer(`server-${index + 1}`),
      mcp_server_id: parseMcpServerId(`0190f5fe-7c00-7a00-8000-${String(index + 1).padStart(12, '0')}`),
    }));

    await act(async () => {
      f.catalog.resolve(servers);
    });

    const installed = within(v.getByTestId('mcp-installed-surface'));
    expect(installed.getByText('server-1')).toBeTruthy();
    expect(installed.getByText('server-10')).toBeTruthy();
    expect(installed.queryByText('server-11')).toBeNull();

    fireEvent.click(within(v.getByTestId('mcp-server-pagination')).getByText('2'));

    expect(installed.queryByText('server-1')).toBeNull();
    expect(installed.getByText('server-11')).toBeTruthy();
    expect(installed.getByText('server-12')).toBeTruthy();
  });

  test('failed GET keeps installed editing and market Add closed', async () => {
    const f = fixture();
    const v = await f.mount(true, '/mcp');
    await act(async () => {
      f.catalog.reject(new Error('fixture offline'));
    });
    expect(v.queryByRole('button', { name: settings.mcpAddServer }) === null).toBe(true);
    expect(v.getByRole('alert').textContent).toContain('Failed to load MCP servers');
    await act(async () => {
      f.go('/mcp?tab=market');
    });
    v.add('alpha');
    expect(f.resolving).not.toHaveBeenCalled();
  });
});
