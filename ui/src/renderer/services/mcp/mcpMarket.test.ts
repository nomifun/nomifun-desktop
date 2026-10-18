import type { ISkillMarketItem } from '@/common/adapter/ipcBridge';
import type { IMcpServer } from '@/common/config/storage';
import { afterEach, beforeEach, describe, expect, mock, test } from 'bun:test';
import {
  MCP_MARKET_CACHE_KEY,
  attachMcpMarketOrigin,
  getMcpMarketItemServers,
  getMcpMarketOrigin,
  getMcpMarketSnapshot,
  notifyMcpMarketCacheChanged,
  preserveMcpMarketOrigin,
  resolveMcpDisplayName,
  subscribeMcpMarketCache,
} from './mcpMarket';

const item: ISkillMarketItem = {
  id: 'mcpworld:fetch', source: 'mcpworld', rank: 1,
  name: '网页内容抓取服务器', description: 'Fetch a web page.',
  url: 'https://www.mcpworld.com/zh/detail/fetch',
  install_command: 'mcp market add mcpworld:fetch',
};

const server = (original_json = '{}', name = 'fetch', builtin = false): IMcpServer =>
  ({ name, original_json, builtin }) as IMcpServer;

const oldOrigin = (item_id = item.id): string =>
  JSON.stringify({ _nomifun_market: { version: 1, item_id } });

let previousCache: string | null;
let previousSnapshot: readonly ISkillMarketItem[];
beforeEach(() => {
  previousCache = localStorage.getItem(MCP_MARKET_CACHE_KEY);
  previousSnapshot = getMcpMarketSnapshot();
  localStorage.removeItem(MCP_MARKET_CACHE_KEY);
  notifyMcpMarketCacheChanged([]);
});
afterEach(() => {
  if (previousCache === null) localStorage.removeItem(MCP_MARKET_CACHE_KEY);
  else localStorage.setItem(MCP_MARKET_CACHE_KEY, previousCache);
  notifyMcpMarketCacheChanged(previousSnapshot);
});

describe('MCP market presentation', () => {
  test('resolves existing ID-only imports from the same current market title', () => {
    localStorage.setItem(MCP_MARKET_CACHE_KEY, JSON.stringify({ items: [item] }));
    const installed = server(oldOrigin());
    expect(getMcpMarketOrigin(installed)).toBe(item.id);
    expect(resolveMcpDisplayName(installed)).toBe(item.name);
    expect(installed.name).toBe('fetch');
  });

  test('persists a sanitized title with the configuration for offline display', () => {
    const original = JSON.stringify({ mcpServers: { fetch: { command: 'uvx', args: ['mcp-server-fetch'] } } });
    const marked = attachMcpMarketOrigin(original, { ...item, name: '  网页\n内容\u0000抓取  ' });
    expect(JSON.parse(marked).mcpServers.fetch.command).toBe('uvx');
    expect(JSON.parse(marked)._nomifun_market).toEqual({
      version: 1, item_id: item.id, name: '网页 内容 抓取',
    });
    expect(resolveMcpDisplayName(server(marked), [])).toBe('网页 内容 抓取');
    expect(resolveMcpDisplayName(server(attachMcpMarketOrigin('{}', { ...item, name: 'x'.repeat(110) })), [])).toHaveLength(96);
    expect(resolveMcpDisplayName(server(oldOrigin()), [])).toBe('fetch');
  });

  test('a refreshed market title overrides its saved snapshot', () => {
    const installed = server(attachMcpMarketOrigin('{}', item));
    localStorage.setItem(MCP_MARKET_CACHE_KEY, JSON.stringify({ items: [item] }));
    const first = getMcpMarketSnapshot();
    expect(getMcpMarketSnapshot()).toBe(first);
    localStorage.setItem('unrelated-mcp-test', 'value');
    expect(getMcpMarketSnapshot()).toBe(first);
    localStorage.removeItem('unrelated-mcp-test');

    const renamed = { ...item, name: '最新抓取服务器' };
    localStorage.setItem(MCP_MARKET_CACHE_KEY, JSON.stringify({ items: [renamed] }));
    notifyMcpMarketCacheChanged();
    const refreshed = getMcpMarketSnapshot();
    expect(refreshed).not.toBe(first);
    expect(getMcpMarketSnapshot()).toBe(refreshed);
    expect(resolveMcpDisplayName(installed)).toBe(renamed.name);

    localStorage.removeItem(MCP_MARKET_CACHE_KEY);
    expect(resolveMcpDisplayName(installed)).toBe(item.name);
  });

  test('uses runtime names for manual and builtin servers and ignores another market source', () => {
    const skillHub: ISkillMarketItem = {
      ...item, id: 'skillhub_mcp:fetch', source: 'skillhub_mcp', name: '另一个市场的名称',
      url: 'https://skillhub.cn/mcp/fetch', install_command: 'mcp market add skillhub:fetch',
    };
    expect(resolveMcpDisplayName(server(), [item])).toBe('fetch');
    expect(resolveMcpDisplayName(server(attachMcpMarketOrigin('{}', item), 'fetch', true), [item])).toBe('fetch');
    expect(resolveMcpDisplayName(server(oldOrigin()), [skillHub])).toBe('fetch');
    expect(resolveMcpDisplayName(server(oldOrigin()), [{ ...item, source: 'skillhub_mcp' }])).toBe('fetch');
  });

  test('malformed or unsupported provenance falls back to the runtime name', () => {
    for (const original of [
      '{', 'null', '[]', '{}',
      JSON.stringify({ _nomifun_market: 'fetch' }),
      JSON.stringify({ _nomifun_market: { version: 2, item_id: item.id, name: 'wrong' } }),
      JSON.stringify({ _nomifun_market: { version: 1, item_id: 'skillhub:fetch', name: 'wrong' } }),
      JSON.stringify({ _nomifun_market: { version: 1, item_id: '', name: 'wrong' } }),
    ]) {
      expect(getMcpMarketOrigin(server(original))).toBeNull();
      expect(resolveMcpDisplayName(server(original), [item])).toBe('fetch');
    }
  });

  test('connection JSON replacements retain a valid market marker without replacing configuration', () => {
    const installed = server(attachMcpMarketOrigin('{}', item));
    const replacement = JSON.stringify({ mcpServers: { fetch: { url: 'https://example.com/mcp' } } });
    const preserved = preserveMcpMarketOrigin(replacement, installed);
    expect(JSON.parse(preserved).mcpServers).toEqual(JSON.parse(replacement).mcpServers);
    expect(resolveMcpDisplayName(server(preserved), [])).toBe(item.name);

    const explicitlyReplaced = attachMcpMarketOrigin(replacement, { id: 'skillhub_mcp:other', name: 'Explicit title' });
    expect(preserveMcpMarketOrigin(explicitlyReplaced, installed)).toBe(explicitlyReplaced);
    expect(preserveMcpMarketOrigin(replacement, server())).toBe(replacement);
    expect(preserveMcpMarketOrigin('{', installed)).toBe('{');

    for (const config of [
      { fetch: { command: 'uvx' } },
      [{ name: 'fetch', command: 'uvx' }],
    ]) {
      const wrapped = preserveMcpMarketOrigin(JSON.stringify(config), installed);
      expect(JSON.parse(wrapped).mcpServers).toEqual(config);
      expect(resolveMcpDisplayName(server(wrapped), [])).toBe(item.name);
    }
  });
});

describe('MCP market cache subscriptions', () => {
  test('shares newly synced titles when persistence leaves an older ranking cache on disk', () => {
    localStorage.setItem(MCP_MARKET_CACHE_KEY, JSON.stringify({ items: [item] }));
    expect(getMcpMarketSnapshot()[0]?.name).toBe(item.name);

    const refreshed = { ...item, name: '  新同步的\n抓取服务  ' };
    notifyMcpMarketCacheChanged([refreshed, { ...refreshed, id: 'skillhub_mcp:fetch' }]);
    const shared = getMcpMarketSnapshot();
    expect(shared).toHaveLength(1);
    expect(shared[0]?.name).toBe('新同步的 抓取服务');
    expect(resolveMcpDisplayName(server(oldOrigin()))).toBe('新同步的 抓取服务');
    notifyMcpMarketCacheChanged();
    expect(getMcpMarketSnapshot()).toBe(shared);
    expect(JSON.parse(localStorage.getItem(MCP_MARKET_CACHE_KEY)!).items[0].name).toBe(item.name);

    const next = { ...item, name: '后来成功保存的标题' };
    localStorage.setItem(MCP_MARKET_CACHE_KEY, JSON.stringify({ items: [next] }));
    notifyMcpMarketCacheChanged();
    expect(getMcpMarketSnapshot()[0]?.name).toBe(next.name);
  });

  test.each([MCP_MARKET_CACHE_KEY, null])('retains a sync without storage until an authoritative storage event for %s', (key) => {
    const storageDescriptor = Object.getOwnPropertyDescriptor(globalThis, 'localStorage')!;
    Object.defineProperty(globalThis, 'localStorage', {
      configurable: true,
      get: () => { throw new Error('storage unavailable'); },
    });
    try {
      notifyMcpMarketCacheChanged([item]);
      expect(getMcpMarketSnapshot()[0]?.name).toBe(item.name);
      expect(resolveMcpDisplayName(server(oldOrigin()))).toBe(item.name);
    } finally {
      Object.defineProperty(globalThis, 'localStorage', storageDescriptor);
    }
    expect(getMcpMarketSnapshot()[0]?.name).toBe(item.name);
    const unsubscribe = subscribeMcpMarketCache(() => {});
    try {
      window.dispatchEvent(new StorageEvent('storage', { key, storageArea: localStorage }));
      expect(getMcpMarketSnapshot()).toEqual([]);
    } finally {
      unsubscribe();
    }
  });

  test('publishes same-window refreshes and relevant other-window localStorage changes', () => {
    const changed = mock(() => {});
    const unsubscribe = subscribeMcpMarketCache(changed);
    try {
      notifyMcpMarketCacheChanged();
      window.dispatchEvent(new StorageEvent('storage', { key: 'unrelated' }));
      window.dispatchEvent(new StorageEvent('storage', { key: MCP_MARKET_CACHE_KEY, storageArea: sessionStorage }));
      expect(changed).toHaveBeenCalledTimes(1);
      window.dispatchEvent(new StorageEvent('storage', { key: MCP_MARKET_CACHE_KEY, storageArea: localStorage }));
      expect(changed).toHaveBeenCalledTimes(2);
      window.dispatchEvent(new StorageEvent('storage', { key: null, storageArea: localStorage }));
      expect(changed).toHaveBeenCalledTimes(3);
    } finally {
      unsubscribe();
    }
    notifyMcpMarketCacheChanged();
    window.dispatchEvent(new StorageEvent('storage', { key: MCP_MARKET_CACHE_KEY }));
    expect(changed).toHaveBeenCalledTimes(3);
  });

  test('normalizes the existing cache and excludes skills and mismatched source identities', () => {
    localStorage.setItem(MCP_MARKET_CACHE_KEY, JSON.stringify({ items: [
      { ...item, name: '  网页\n内容抓取  ' },
      { ...item, id: 'skillhub_mcp:fetch' },
      {
        ...item, id: 'skillhub:owner/demo', source: 'skillhub',
        url: 'https://skillhub.cn/skills/owner/demo', install_command: 'npx skills add @owner/demo',
      },
    ] }));
    const snapshot = getMcpMarketSnapshot();
    expect(snapshot).toHaveLength(1);
    expect(snapshot[0]?.name).toBe('网页 内容抓取');
  });

  test('fails safely for missing, malformed and inaccessible caches', () => {
    for (const raw of ['{', 'null', '[]', JSON.stringify({ items: 'wrong' })]) {
      localStorage.setItem(MCP_MARKET_CACHE_KEY, raw);
      const empty = getMcpMarketSnapshot();
      expect(empty).toEqual([]);
      expect(getMcpMarketSnapshot()).toBe(empty);
    }
    localStorage.setItem(MCP_MARKET_CACHE_KEY, JSON.stringify({ items: [item] }));
    expect(getMcpMarketSnapshot()).toHaveLength(1);
    const storageDescriptor = Object.getOwnPropertyDescriptor(globalThis, 'localStorage')!;
    Object.defineProperty(globalThis, 'localStorage', {
      configurable: true,
      get: () => { throw new Error('storage unavailable'); },
    });
    try {
      const empty = getMcpMarketSnapshot();
      expect(empty).toEqual([]);
      expect(getMcpMarketSnapshot()).toBe(empty);
      expect(resolveMcpDisplayName(server(attachMcpMarketOrigin('{}', item)))).toBe(item.name);
    } finally {
      Object.defineProperty(globalThis, 'localStorage', storageDescriptor);
    }
  });
});

describe('MCP market installed identity', () => {
  test('prefers exact provenance while keeping legacy names scoped to unmarked user servers', () => {
    const exact = server(oldOrigin(), 'renamed-runtime-key');
    const legacy = server('{}', 'Fetch');
    const foreign = server(oldOrigin('skillhub_mcp:fetch'));
    const builtin = server(oldOrigin(), 'fetch', true);
    expect(getMcpMarketItemServers(item, [exact, legacy, foreign, builtin])).toEqual([exact]);
    expect(getMcpMarketItemServers(item, [legacy, foreign, builtin])).toEqual([legacy]);
    expect(getMcpMarketItemServers(item, [foreign, builtin])).toEqual([]);
    expect(getMcpMarketItemServers({ id: 'skillhub_mcp:some-server', name: 'Localized name' }, [server('{}', 'some_server')])).toHaveLength(1);
  });
});
