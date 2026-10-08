import type { ISkillMarketItem, SkillMarketSource } from '@/common/adapter/ipcBridge';

export const SKILL_MARKET_SOURCES: SkillMarketSource[] = ['clawhub', 'loophub', 'skillhub'];
export const MCP_MARKET_SOURCES: SkillMarketSource[] = ['skillhub_mcp', 'mcpworld'];

const MARKET_SOURCE_LABELS: Record<SkillMarketSource, string> = {
  clawhub: 'ClawHub',
  loophub: 'LoopHub',
  skillhub: 'SkillHub',
  skillhub_mcp: 'SkillHub MCP',
  mcpworld: 'MCP World',
  clawhub_plugins: 'ClawHub Plugins',
};

const MARKET_SOURCE_URLS: Record<SkillMarketSource, string> = {
  clawhub: 'https://clawhub.ai/',
  loophub: 'https://hub.cocoloop.cn/popular',
  skillhub: 'https://skillhub.cn/skills?sortBy=score',
  skillhub_mcp: 'https://skillhub.cn/mcp',
  mcpworld: 'https://www.mcpworld.com/?category=most_popular',
  clawhub_plugins: 'https://clawhub.ai/plugins',
};

export const marketSourceLabel = (source: SkillMarketSource): string => MARKET_SOURCE_LABELS[source];
export const marketSourceUrl = (source: SkillMarketSource): string => MARKET_SOURCE_URLS[source];

const MAX_NAME_LENGTH = 96;
const MAX_DESCRIPTION_LENGTH = 220;
const MAX_COMMAND_LENGTH = 320;

const isSkillMarketSource = (value: unknown): value is SkillMarketSource =>
  value === 'clawhub' ||
  value === 'skillhub' ||
  value === 'loophub' ||
  value === 'skillhub_mcp' ||
  value === 'mcpworld' ||
  value === 'clawhub_plugins';

export const cleanMarketText = (value: unknown, maxLength = MAX_DESCRIPTION_LENGTH): string => {
  if (typeof value !== 'string') return '';
  return value
    .replace(/[\u0000-\u001f\u007f]/g, ' ')
    .replace(/\s+/g, ' ')
    .trim()
    .slice(0, maxLength);
};

const isSafeMarketUrl = (source: SkillMarketSource, url: string): boolean => {
  try {
    const parsed = new URL(url);
    if (parsed.protocol !== 'https:' || parsed.username || parsed.password || parsed.port) return false;
    if (source === 'clawhub' || source === 'clawhub_plugins') return parsed.hostname === 'clawhub.ai';
    if (source === 'skillhub') return parsed.hostname === 'skillhub.cn';
    if (source === 'loophub') return parsed.hostname === 'hub.cocoloop.cn';
    if (source === 'skillhub_mcp') return parsed.hostname === 'skillhub.cn';
    if (source === 'mcpworld') return parsed.hostname === 'www.mcpworld.com';
    return false;
  } catch {
    return false;
  }
};

const isSafeInstallCommand = (source: SkillMarketSource, value: string): boolean => {
  if (!value || value.length > MAX_COMMAND_LENGTH) return false;
  if (/[\r\n;&|<>`$]/.test(value)) return false;
  if (source === 'clawhub') return value.startsWith('openclaw skills install @');
  if (source === 'skillhub') return value.startsWith('npx skills add ');
  if (source === 'loophub') return value.startsWith('loophub skill download https://dl.cocoloop.cn/bss/skills/');
  if (source === 'skillhub_mcp') return /^mcp market add skillhub:[a-z0-9._-]+$/i.test(value);
  if (source === 'mcpworld') return /^mcp market add mcpworld:[a-z0-9._-]+$/i.test(value);
  if (source === 'clawhub_plugins') return value.startsWith('openclaw plugins install clawhub:@');
  return false;
};

const cleanTagList = (value: unknown): string[] => {
  if (!Array.isArray(value)) return [];
  const seen = new Set<string>();
  return value
    .map((item) => cleanMarketText(item, 40).toLowerCase())
    .filter((item) => /^[a-z0-9_-]+$/.test(item))
    .filter((item) => {
      if (seen.has(item)) return false;
      seen.add(item);
      return true;
    })
    .slice(0, 12);
};

export const normalizeSkillMarketItem = (raw: unknown): ISkillMarketItem | null => {
  if (!raw || typeof raw !== 'object') return null;
  const data = raw as Partial<ISkillMarketItem>;
  if (!isSkillMarketSource(data.source)) return null;

  const url = cleanMarketText(data.url, 260);
  const install_command = cleanMarketText(data.install_command, MAX_COMMAND_LENGTH);
  if (!isSafeMarketUrl(data.source, url) || !isSafeInstallCommand(data.source, install_command)) return null;

  const name = cleanMarketText(data.name, MAX_NAME_LENGTH);
  if (!name) return null;

  return {
    id: cleanMarketText(data.id, 160) || `${data.source}:${name}`,
    source: data.source,
    rank: Number.isFinite(data.rank) ? Number(data.rank) : 0,
    name,
    description: cleanMarketText(data.description, MAX_DESCRIPTION_LENGTH),
    url,
    install_command,
    tags: cleanTagList(data.tags),
    audience_tags: cleanTagList(data.audience_tags),
    scenario_tags: cleanTagList(data.scenario_tags),
    stats: cleanMarketText(data.stats, 60) || undefined,
  };
};

export const normalizeSkillMarketItems = (raw: unknown): ISkillMarketItem[] => {
  if (!Array.isArray(raw)) return [];
  return raw.map(normalizeSkillMarketItem).filter((item): item is ISkillMarketItem => Boolean(item));
};

export const normalizeSkillMarketErrors = (raw: unknown): string[] => {
  if (!Array.isArray(raw)) return [];
  return raw
    .map((item) => cleanMarketText(item, 240))
    .filter(Boolean)
    .slice(0, 4);
};

export const resolveMarketSyncItems = (
  cachedItems: ISkillMarketItem[],
  syncedItems: ISkillMarketItem[]
): ISkillMarketItem[] => (syncedItems.length > 0 ? syncedItems : cachedItems);

export const selectMarketSourceWithItems = (
  activeSource: SkillMarketSource,
  sources: readonly SkillMarketSource[],
  items: readonly ISkillMarketItem[]
): SkillMarketSource => {
  const itemSources = new Set(items.map((item) => item.source));
  if (itemSources.has(activeSource)) return activeSource;
  return sources.find((source) => itemSources.has(source)) ?? activeSource;
};

export const filterSkillMarketItems = (
  items: ISkillMarketItem[],
  source: SkillMarketSource,
  query: string
): ISkillMarketItem[] => {
  const q = query.trim().toLowerCase();
  return items.filter((item) => {
    if (item.source !== source) return false;
    if (q) {
      const haystack = [
        item.name,
        item.description,
        item.tags?.join(' '),
        item.stats,
      ]
        .join(' ')
        .toLowerCase();
      if (!haystack.includes(q)) return false;
    }
    return true;
  });
};
