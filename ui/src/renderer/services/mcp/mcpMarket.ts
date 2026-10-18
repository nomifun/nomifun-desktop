import type { ISkillMarketItem } from '@/common/adapter/ipcBridge';
import type { IMcpServer } from '@/common/config/storage';
import { cleanMarketText, normalizeSkillMarketItems } from '@/renderer/services/skills/skillMarket';

export const MCP_MARKET_CACHE_KEY = 'nomifun.mcpMarket.rankings.v1';
const MCP_MARKET_ORIGIN_KEY = '_nomifun_market';

type McpMarketOrigin = {
  version: 1;
  item_id: string;
  name?: string;
};

const isRecord = (value: unknown): value is Record<string, unknown> =>
  Boolean(value) && typeof value === 'object' && !Array.isArray(value);

const parseOriginalJson = (originalJson: string): Record<string, unknown> | null => {
  try {
    const parsed: unknown = JSON.parse(originalJson);
    return isRecord(parsed) ? parsed : null;
  } catch {
    return null;
  }
};

const isMcpMarketId = (value: unknown): value is string =>
  typeof value === 'string' && value.length <= 160 && /^(?:skillhub_mcp|mcpworld):[a-z0-9._-]+$/i.test(value);

const readOrigin = (server: Pick<IMcpServer, 'original_json'>): McpMarketOrigin | null => {
  const origin = parseOriginalJson(server.original_json)?.[MCP_MARKET_ORIGIN_KEY];
  if (!isRecord(origin) || origin.version !== 1 || !isMcpMarketId(origin.item_id)) return null;
  const name = cleanMarketText(origin.name, 96);
  return { version: 1, item_id: origin.item_id, ...(name ? { name } : {}) };
};

/** Market presentation is separate from the server's immutable runtime name. */
export const attachMcpMarketOrigin = (
  originalJson: string,
  item: Pick<ISkillMarketItem, 'id' | 'name'>
): string => JSON.stringify(
  {
    ...(parseOriginalJson(originalJson) ?? {}),
    [MCP_MARKET_ORIGIN_KEY]: { version: 1, item_id: item.id, name: cleanMarketText(item.name, 96) },
  },
  null,
  2
);

export const getMcpMarketOrigin = (server: Pick<IMcpServer, 'original_json'>): string | null =>
  readOrigin(server)?.item_id ?? null;

/** Replacing a connection JSON should retain its existing market presentation. */
export const preserveMcpMarketOrigin = (
  originalJson: string,
  originalServer: Pick<IMcpServer, 'original_json'>
): string => {
  if (readOrigin({ original_json: originalJson })) return originalJson;
  const origin = readOrigin(originalServer);
  if (!origin) return originalJson;
  try {
    const parsed: unknown = JSON.parse(originalJson);
    if (!isRecord(parsed) && !Array.isArray(parsed)) return originalJson;
    // The editor also accepts a bare server map or array. Enclose those forms
    // so the provenance marker can never be mistaken for another server.
    const original = isRecord(parsed) && Object.prototype.hasOwnProperty.call(parsed, 'mcpServers')
      ? parsed
      : { mcpServers: parsed };
    return JSON.stringify({ ...original, [MCP_MARKET_ORIGIN_KEY]: origin }, null, 2);
  } catch {
    return originalJson;
  }
};

export const normalizeMcpMarketName = (value: string): string =>
  value.trim().toLocaleLowerCase().replace(/[\s_]+/g, '-');

export const getMcpMarketItemServers = (
  item: Pick<ISkillMarketItem, 'id' | 'name'>,
  servers: readonly IMcpServer[]
): IMcpServer[] => {
  const eligible = servers.filter((server) => server.builtin !== true);
  const exact = eligible.filter((server) => getMcpMarketOrigin(server) === item.id);
  if (exact.length > 0) return exact;

  const idSlug = item.id.split(':').slice(1).join(':').split('/').filter(Boolean).at(-1) ?? '';
  const legacyNames = new Set([item.name, idSlug].map(normalizeMcpMarketName).filter(Boolean));
  return eligible.filter((server) => !getMcpMarketOrigin(server) && legacyNames.has(normalizeMcpMarketName(server.name)));
};

const cacheListeners = new Set<() => void>();
let cacheInput: string | null | undefined;
let marketSnapshot: readonly ISkillMarketItem[] = [];

const readCacheInput = (): string | null => {
  try {
    return localStorage.getItem(MCP_MARKET_CACHE_KEY);
  } catch {
    return null;
  }
};

const normalizeMcpMarketItems = (items: unknown): ISkillMarketItem[] =>
  normalizeSkillMarketItems(items).filter(
    (item) => isMcpMarketId(item.id) && item.id.startsWith(`${item.source}:`) &&
      (item.source === 'skillhub_mcp' || item.source === 'mcpworld')
  );

/** Stable snapshots allow useSyncExternalStore to observe the existing ranking cache. */
export const getMcpMarketSnapshot = (): readonly ISkillMarketItem[] => {
  const raw = readCacheInput();
  if (raw === cacheInput) return marketSnapshot;
  cacheInput = raw;
  marketSnapshot = [];
  try {
    const cache: unknown = JSON.parse(raw ?? '{}');
    if (isRecord(cache)) {
      marketSnapshot = normalizeMcpMarketItems(cache.items);
    }
  } catch {
    // A malformed ranking cache never blocks the canonical installed catalog.
  }
  return marketSnapshot;
};

export const notifyMcpMarketCacheChanged = (items?: readonly ISkillMarketItem[]): void => {
  if (items !== undefined) {
    // A successful sync remains shared presentation even when browser cache
    // persistence fails. Reload disk only after its raw value actually changes.
    marketSnapshot = normalizeMcpMarketItems(items);
    cacheInput = readCacheInput();
  }
  for (const listener of cacheListeners) listener();
};

const handleStorageChange = (event: StorageEvent): void => {
  if (event.key !== null && event.key !== MCP_MARKET_CACHE_KEY) return;
  if (event.storageArea) {
    try {
      if (event.storageArea !== localStorage) return;
    } catch {
      return;
    }
  }
  // A storage clear/removal is authoritative even when a failed write left an
  // in-memory snapshot associated with the same missing disk value.
  cacheInput = undefined;
  notifyMcpMarketCacheChanged();
};

export const subscribeMcpMarketCache = (listener: () => void): (() => void) => {
  if (cacheListeners.size === 0) window.addEventListener('storage', handleStorageChange);
  cacheListeners.add(listener);
  return () => {
    cacheListeners.delete(listener);
    if (cacheListeners.size === 0) window.removeEventListener('storage', handleStorageChange);
  };
};

export const resolveMcpDisplayName = (
  server: Pick<IMcpServer, 'name' | 'original_json' | 'builtin'>,
  items: readonly ISkillMarketItem[] = getMcpMarketSnapshot()
): string => {
  if (server.builtin === true) return server.name;
  const origin = readOrigin(server);
  if (!origin) return server.name;
  const item = items.find((entry) => entry.id === origin.item_id && origin.item_id.startsWith(`${entry.source}:`));
  return cleanMarketText(item?.name, 96) || origin.name || server.name;
};
