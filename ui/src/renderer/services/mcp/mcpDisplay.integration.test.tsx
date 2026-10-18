import { afterEach, beforeAll, beforeEach, describe, expect, mock, spyOn, test } from 'bun:test';
import { act, cleanup, fireEvent, render, waitFor, within } from '@testing-library/react';
import { createInstance } from 'i18next';
import { I18nextProvider } from 'react-i18next';
import { MemoryRouter } from 'react-router-dom';
import { ipcBridge } from '@/common';
import type { ISkillMarketItem } from '@/common/adapter/ipcBridge';
import type { IMcpServer } from '@/common/config/storage';
import { parseMcpServerId } from '@/common/types/ids';
import SessionCapabilityPicker, { type SessionCapabilityDraft } from '@/renderer/components/chat/SessionCapabilityPicker';
import McpMarketSettings from '@/renderer/pages/mcp/McpMarketSettings';
import McpServerHeader from '@/renderer/pages/settings/ToolsSettings/McpServerHeader';
import SkillMarketCard from '@/renderer/pages/settings/skill/SkillMarketCard';
import settings from '@/renderer/services/i18n/locales/en-US/settings.json';
import common from '@/renderer/services/i18n/locales/en-US/common.json';
import { attachMcpMarketOrigin, getMcpMarketSnapshot, MCP_MARKET_CACHE_KEY, notifyMcpMarketCacheChanged } from './mcpMarket';

const locale = createInstance();
let previousCache: string | null;
let previousSnapshot: readonly ISkillMarketItem[];
beforeAll(async () => {
  await locale.init({ lng: 'en-US', resources: { 'en-US': { translation: { settings, common } } } });
});
beforeEach(() => {
  previousCache = localStorage.getItem(MCP_MARKET_CACHE_KEY);
  previousSnapshot = getMcpMarketSnapshot();
  localStorage.removeItem(MCP_MARKET_CACHE_KEY);
  notifyMcpMarketCacheChanged([]);
});
afterEach(() => {
  cleanup();
  if (previousCache === null) localStorage.removeItem(MCP_MARKET_CACHE_KEY);
  else localStorage.setItem(MCP_MARKET_CACHE_KEY, previousCache);
  notifyMcpMarketCacheChanged(previousSnapshot);
});

const item: ISkillMarketItem = {
  id: 'mcpworld:fetch', source: 'mcpworld', rank: 1,
  name: '网页内容抓取服务器', description: '抓取网页并将 HTML 转换为 Markdown。',
  url: 'https://www.mcpworld.com/servers/fetch', install_command: 'mcp market add mcpworld:fetch',
};
const server: IMcpServer = {
  mcp_server_id: parseMcpServerId('0190f5fe-7c00-7a00-8000-000000000069'),
  name: 'fetch', enabled: true,
  transport: { type: 'stdio', command: 'fixture-command' },
  original_json: '{}', created_at: 1, updated_at: 1,
  last_test_status: 'connected', tools: [{ name: 'fetch' }],
};
const noAuto = new Set<string>();

function Surfaces({ current = server, marketItem = item, liveMarket = false, onToggle = () => {}, onTest = () => {}, onSelect = () => {} }: {
  current?: IMcpServer;
  marketItem?: ISkillMarketItem;
  liveMarket?: boolean;
  onToggle?: (value: IMcpServer) => void;
  onTest?: (value: IMcpServer) => void;
  onSelect?: (draft: SessionCapabilityDraft) => void;
}) {
  return <MemoryRouter><I18nextProvider i18n={locale}>
    <section data-testid='market'>
      {liveMarket
        ? <McpMarketSettings mcpServers={[current]} saveMcpServers={async () => {}} />
        : <SkillMarketCard item={marketItem} adding={false} added addedStateLoading={false} onAdd={() => {}} />}
    </section>
    <section data-testid='installed'>
      <McpServerHeader server={current} isTestingConnection={false} onTestConnection={onTest}
        onToggleEnabled={onToggle} onEditServer={() => {}} onDeleteServer={() => {}} />
    </section>
    <SessionCapabilityPicker catalog={{ skills: [], autoSkillNames: noAuto, mcpServers: [current] }}
      draft={{ skillNames: [], mcpServerIds: [] }} onChange={onSelect} applyMode='create' />
  </I18nextProvider></MemoryRouter>;
}

function cacheMarketItem(marketItem: ISkillMarketItem) {
  localStorage.setItem(MCP_MARKET_CACHE_KEY, JSON.stringify({ items: [marketItem] }));
  notifyMcpMarketCacheChanged();
}

describe('MCP display across renderer surfaces', () => {
  test('market, installed header and Session choices share the market title while actions retain canonical identities', async () => {
    const current = { ...server, original_json: attachMcpMarketOrigin(server.original_json, item) };
    const originalJson = current.original_json;
    const onToggle = mock((_value: IMcpServer) => {});
    const onTest = mock((_value: IMcpServer) => {});
    const onSelect = mock((_draft: SessionCapabilityDraft) => {});
    const view = render(<Surfaces current={current} onToggle={onToggle} onTest={onTest} onSelect={onSelect} />);

    expect(within(view.getByTestId('market')).getByText(item.name)).toBeTruthy();
    const installed = within(view.getByTestId('installed'));
    expect(installed.getByText(item.name)).toBeTruthy();
    expect(installed.queryByText(server.name)).toBeNull();
    fireEvent.click(installed.getByRole('switch', { name: `Disable ${item.name}` }));
    fireEvent.click(installed.getByRole('button', { name: settings.mcpTestConnection }));
    expect(onToggle).toHaveBeenCalledTimes(1);
    expect(onToggle.mock.calls[0]?.[0]).toBe(current);
    expect(onTest).toHaveBeenCalledTimes(1);
    expect(onTest.mock.calls[0]?.[0]).toBe(current);

    fireEvent.click(view.getByTestId('session-mcp-trigger'));
    fireEvent.click(await view.findByRole('checkbox', { name: item.name }));
    expect(onSelect).toHaveBeenCalledWith({ skillNames: [], mcpServerIds: [current.mcp_server_id] });
    expect(current.name).toBe('fetch');
    expect(current.original_json).toBe(originalJson);
  });

  test('existing id-only origins follow live market refreshes in already-open installed and Session surfaces', async () => {
    const current = {
      ...server,
      original_json: JSON.stringify({ _nomifun_market: { version: 1, item_id: item.id } }),
    };
    cacheMarketItem(item);
    const view = render(<Surfaces current={current} />);
    fireEvent.click(view.getByTestId('session-mcp-trigger'));
    expect(within(view.getByTestId('installed')).getByText(item.name)).toBeTruthy();
    expect(await view.findByRole('checkbox', { name: item.name })).toBeTruthy();

    const refreshed = { ...item, name: '更新后的网页抓取服务' };
    await act(async () => {
      cacheMarketItem(refreshed);
      view.rerender(<Surfaces current={current} marketItem={refreshed} />);
    });
    for (const id of ['market', 'installed', 'session-mcp-list']) {
      expect(within(view.getByTestId(id)).getAllByText(refreshed.name).length).toBeGreaterThan(0);
      expect(within(view.getByTestId(id)).queryByText(item.name)).toBeNull();
    }
    expect(view.getByRole('checkbox', { name: refreshed.name })).toBeTruthy();

    const otherWindowItem = { ...item, name: '其他窗口更新的抓取服务' };
    await act(async () => {
      localStorage.setItem(MCP_MARKET_CACHE_KEY, JSON.stringify({ items: [otherWindowItem] }));
      window.dispatchEvent(new StorageEvent('storage', { key: MCP_MARKET_CACHE_KEY }));
    });
    expect(within(view.getByTestId('installed')).getByText(otherWindowItem.name)).toBeTruthy();
    expect(view.getByRole('checkbox', { name: otherWindowItem.name })).toBeTruthy();
  });

  test('removing the market cache restores the saved title and leaves manual server names unchanged', async () => {
    const current = { ...server, original_json: attachMcpMarketOrigin(server.original_json, item) };
    const refreshed = { ...item, name: '当前市场抓取服务' };
    cacheMarketItem(refreshed);
    const view = render(<Surfaces current={current} marketItem={refreshed} />);
    fireEvent.click(view.getByTestId('session-mcp-trigger'));
    expect(within(view.getByTestId('installed')).getByText(refreshed.name)).toBeTruthy();
    expect(await view.findByRole('checkbox', { name: refreshed.name })).toBeTruthy();

    await act(async () => {
      localStorage.removeItem(MCP_MARKET_CACHE_KEY);
      notifyMcpMarketCacheChanged();
    });
    expect(within(view.getByTestId('installed')).getByText(item.name)).toBeTruthy();
    expect(view.getByRole('checkbox', { name: item.name })).toBeTruthy();

    await act(async () => {
      cacheMarketItem(item);
      view.rerender(<Surfaces />);
    });
    expect(within(view.getByTestId('installed')).getByText(server.name)).toBeTruthy();
    expect(view.getByRole('checkbox', { name: server.name })).toBeTruthy();
    expect(within(view.getByTestId('installed')).queryByText(item.name)).toBeNull();
  });

  test('a real market refresh keeps mounted consumers in sync when browser cache persistence fails', async () => {
    const current = {
      ...server,
      original_json: JSON.stringify({ _nomifun_market: { version: 1, item_id: item.id } }),
    };
    cacheMarketItem(item);
    const initialCache = localStorage.getItem(MCP_MARKET_CACHE_KEY);
    const autoSyncKey = 'nomifun.mcpMarket.autoSynced.v1';
    const previousAutoSync = sessionStorage.getItem(autoSyncKey);
    sessionStorage.setItem(autoSyncKey, '1');
    const refreshed = { ...item, name: '无需缓存写入的最新抓取服务' };
    const sync = spyOn(ipcBridge.fs.syncSkillMarketRankings, 'invoke').mockResolvedValue({
      fetched_at: 2, items: [refreshed],
    });
    const storage = localStorage;
    const storageDescriptor = Object.getOwnPropertyDescriptor(globalThis, 'localStorage')!;
    const persist = mock((key: string, value: string) => {
      if (key === MCP_MARKET_CACHE_KEY) throw new Error('Storage quota exceeded');
      storage.setItem(key, value);
    });
    // Happy DOM's Storage proxy does not replace methods through spyOn. Keep
    // every real storage operation while failing only this market cache write.
    Object.defineProperty(globalThis, 'localStorage', { configurable: true, value: {
      getItem: storage.getItem.bind(storage), setItem: persist,
      removeItem: storage.removeItem.bind(storage), clear: storage.clear.bind(storage),
      key: storage.key.bind(storage), get length() { return storage.length; },
    } satisfies Storage });
    try {
      const view = render(<Surfaces current={current} liveMarket />);
      fireEvent.click(view.getByTestId('session-mcp-trigger'));
      expect(await view.findByRole('checkbox', { name: item.name })).toBeTruthy();
      const installed = view.getByTestId('installed');
      const installedHeader = installed.firstElementChild;
      const sessionList = view.getByTestId('session-mcp-list');
      expect(within(view.getByTestId('market')).getByText(item.name)).toBeTruthy();
      expect(within(installed).getByText(item.name)).toBeTruthy();

      await act(async () => fireEvent.click(view.getByTestId('btn-sync-mcp-market')));
      await waitFor(() => expect(within(installed).getByText(refreshed.name)).toBeTruthy());
      expect(within(view.getByTestId('market')).getByText(refreshed.name)).toBeTruthy();
      expect(view.getByRole('checkbox', { name: refreshed.name })).toBeTruthy();
      expect(installed.firstElementChild).toBe(installedHeader);
      expect(view.getByTestId('session-mcp-list')).toBe(sessionList);
      expect(localStorage.getItem(MCP_MARKET_CACHE_KEY)).toBe(initialCache);
      expect(sync).toHaveBeenCalledWith({ sources: ['skillhub_mcp', 'mcpworld'] });
      expect(persist.mock.calls.some(([key]) => key === MCP_MARKET_CACHE_KEY)).toBe(true);
    } finally {
      Object.defineProperty(globalThis, 'localStorage', storageDescriptor);
      sync.mockRestore();
      if (previousAutoSync === null) sessionStorage.removeItem(autoSyncKey);
      else sessionStorage.setItem(autoSyncKey, previousAutoSync);
    }
  });
});
