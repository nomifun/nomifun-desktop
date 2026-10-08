import type { ISkillMarketItem, SkillMarketSource } from '@/common/adapter/ipcBridge';
import {
  cleanMarketText,
  normalizeSkillMarketItems,
} from './skillMarket';

/**
 * UI-only provenance for market-installed skills. Runtime identity and Agent
 * context continue to come exclusively from the canonical managed SKILL.md.
 */
export const SKILL_MARKET_CACHE_KEY = 'nomifun.skillMarket.rankings.v4';
const LEGACY_INSTALLED_MARKET_KEY = 'nomifun.skillMarket.installed.v1';
export const INSTALLED_MARKET_KEY = 'nomifun.skillMarket.installed.v2';

type MarketSkillSource = Extract<SkillMarketSource, 'clawhub' | 'loophub' | 'skillhub'>;

export type MarketSkillPresentationSnapshot = {
  name: string;
  description: string;
};

export type InstalledMarketRecord = {
  source: MarketSkillSource;
  skill_names: string[];
  presentation?: MarketSkillPresentationSnapshot;
};

export type InstalledMarketState = Record<string, InstalledMarketRecord>;

const listeners = new Set<() => void>();
let snapshot: InstalledMarketState = {};
let snapshotInputs: (string | null)[] | undefined;
let unpersistedState: InstalledMarketState | undefined;

/** Stable snapshots let all skill surfaces react to presentation changes only. */
export const getInstalledMarketSnapshot = (): InstalledMarketState => {
  if (unpersistedState) return unpersistedState;
  try {
    const inputs = [INSTALLED_MARKET_KEY, LEGACY_INSTALLED_MARKET_KEY, SKILL_MARKET_CACHE_KEY]
      .map((key) => localStorage.getItem(key));
    if (!snapshotInputs || inputs.some((value, index) => value !== snapshotInputs?.[index])) {
      snapshotInputs = inputs;
      snapshot = readInstalledMarketState();
    }
  } catch {
    // Unavailable browser storage must not prevent using the canonical catalog.
  }
  return snapshot;
};

const emitChange = (): void => {
  for (const listener of listeners) listener();
};

const handleStorageChange = (event: StorageEvent): void => {
  if (event.storageArea) {
    try {
      if (event.storageArea !== localStorage) return;
    } catch {
      return;
    }
  }
  if (
    event.key !== null &&
    ![INSTALLED_MARKET_KEY, LEGACY_INSTALLED_MARKET_KEY, SKILL_MARKET_CACHE_KEY].includes(event.key)
  ) return;
  unpersistedState = undefined;
  emitChange();
};

export const subscribeInstalledMarketState = (listener: () => void): (() => void) => {
  if (listeners.size === 0) window.addEventListener('storage', handleStorageChange);
  listeners.add(listener);
  return () => {
    listeners.delete(listener);
    if (listeners.size === 0) window.removeEventListener('storage', handleStorageChange);
  };
};

/** Same-window cache writes do not dispatch the browser's storage event. */
export const notifySkillMarketCacheChanged = (cacheKey: string): void => {
  if (cacheKey !== SKILL_MARKET_CACHE_KEY) return;
  if (unpersistedState) unpersistedState = hydrateFromCurrentCache(unpersistedState);
  emitChange();
};

const isMarketSkillSource = (value: unknown): value is MarketSkillSource =>
  value === 'clawhub' || value === 'loophub' || value === 'skillhub';

const sourceFromMarketId = (id: string): MarketSkillSource | null => {
  const source = id.split(':', 1)[0];
  return isMarketSkillSource(source) ? source : null;
};

const cleanSkillNames = (value: unknown): string[] => {
  if (!Array.isArray(value)) return [];
  return Array.from(
    new Set(
      value
        .filter((name): name is string => typeof name === 'string')
        .map((name) => name.trim())
        .filter((name) => name.length > 0 && name.length <= 255 && !/[\u0000-\u001f\u007f]/.test(name))
    )
  ).slice(0, 32);
};

const presentationFromItem = (item: ISkillMarketItem): MarketSkillPresentationSnapshot => ({
  name: cleanMarketText(item.name, 96),
  description: cleanMarketText(item.description, 220),
});

const parsePresentation = (value: unknown): MarketSkillPresentationSnapshot | undefined => {
  if (!value || typeof value !== 'object' || Array.isArray(value)) return undefined;
  const raw = value as Partial<MarketSkillPresentationSnapshot>;
  const name = cleanMarketText(raw.name, 96);
  if (!name) return undefined;
  return {
    name,
    description: cleanMarketText(raw.description, 220),
  };
};

const parseRecord = (id: string, value: unknown): InstalledMarketRecord | null => {
  if (!value || typeof value !== 'object' || Array.isArray(value) || id.length > 180) return null;
  const raw = value as Partial<InstalledMarketRecord>;
  const source = isMarketSkillSource(raw.source) ? raw.source : sourceFromMarketId(id);
  const skill_names = cleanSkillNames(raw.skill_names);
  if (!source || skill_names.length === 0 || !id.toLowerCase().startsWith(`${source}:`)) return null;
  const presentation = parsePresentation(raw.presentation);
  return {
    source,
    skill_names,
    ...(presentation ? { presentation } : {}),
  };
};

const readCachedMarketItems = (): Map<string, ISkillMarketItem> => {
  try {
    const raw = JSON.parse(localStorage.getItem(SKILL_MARKET_CACHE_KEY) ?? '{}') as { items?: unknown };
    return new Map(normalizeSkillMarketItems(raw.items).map((item) => [item.id, item]));
  } catch {
    return new Map();
  }
};

const hydrateFromCurrentCache = (state: InstalledMarketState): InstalledMarketState => {
  const cachedItems = readCachedMarketItems();
  return Object.fromEntries(
    Object.entries(state).map(([id, record]) => {
      const cached = cachedItems.get(id);
      return [
        id,
        cached && cached.source === record.source
          ? { ...record, presentation: presentationFromItem(cached) }
          : record,
      ];
    })
  );
};

const parseV2State = (value: unknown): InstalledMarketState | null => {
  if (!value || typeof value !== 'object' || Array.isArray(value)) return null;
  const raw = value as { version?: unknown; items?: unknown };
  if (raw.version !== 2 || !raw.items || typeof raw.items !== 'object' || Array.isArray(raw.items)) return null;
  return Object.fromEntries(
    Object.entries(raw.items)
      .slice(0, 512)
      .map(([id, record]) => [cleanMarketText(id, 180), parseRecord(id, record)] as const)
      .filter((entry): entry is readonly [string, InstalledMarketRecord] => Boolean(entry[0] && entry[1]))
      .map(([id, record]) => [id, record])
  );
};

const migrateLegacyState = (): InstalledMarketState => {
  try {
    const value = JSON.parse(localStorage.getItem(LEGACY_INSTALLED_MARKET_KEY) ?? '{}') as unknown;
    if (!value || typeof value !== 'object' || Array.isArray(value)) return {};
    const cachedItems = readCachedMarketItems();
    return Object.fromEntries(
      Object.entries(value)
        .slice(0, 512)
        .map(([id, names]) => {
          const cleanId = cleanMarketText(id, 180);
          const source = sourceFromMarketId(cleanId);
          const skill_names = cleanSkillNames(names);
          const cached = cachedItems.get(cleanId);
          if (!cleanId || !source || skill_names.length === 0) return null;
          return [
            cleanId,
            {
              source,
              skill_names,
              ...(cached && cached.source === source
                ? { presentation: presentationFromItem(cached) }
                : {}),
            } satisfies InstalledMarketRecord,
          ] as const;
        })
        .filter((entry): entry is readonly [string, InstalledMarketRecord] => entry !== null)
    );
  } catch {
    return {};
  }
};

export const readInstalledMarketState = (): InstalledMarketState => {
  if (unpersistedState) return unpersistedState;
  try {
    const persisted = localStorage.getItem(INSTALLED_MARKET_KEY);
    if (persisted !== null) {
      const parsed = parseV2State(JSON.parse(persisted));
      if (parsed) return hydrateFromCurrentCache(parsed);
    }
  } catch {
    // Fall through to the legacy state. A malformed UI cache is not canonical.
  }
  return hydrateFromCurrentCache(migrateLegacyState());
};

export const writeInstalledMarketState = (state: InstalledMarketState): void => {
  try {
    localStorage.setItem(INSTALLED_MARKET_KEY, JSON.stringify({ version: 2, items: state }));
    localStorage.removeItem(LEGACY_INSTALLED_MARKET_KEY);
    unpersistedState = undefined;
  } catch {
    // Presentation provenance is best-effort; the managed Skill Library is canonical.
    unpersistedState = state;
  }
  emitChange();
};

export const recordInstalledMarketItem = (
  state: InstalledMarketState,
  item: ISkillMarketItem,
  skillNames: string[]
): InstalledMarketState => {
  const skill_names = cleanSkillNames(skillNames);
  if (!isMarketSkillSource(item.source) || skill_names.length === 0) return state;
  const claimedNames = new Set(skill_names);
  const next = Object.fromEntries(
    Object.entries(state).filter(
      ([id, record]) => id === item.id || !record.skill_names.some((name) => claimedNames.has(name))
    )
  );
  next[item.id] = {
    source: item.source,
    skill_names,
    presentation: presentationFromItem(item),
  };
  return next;
};

export const reconcileInstalledMarketState = (
  state: InstalledMarketState,
  installedSkillNames: Iterable<string>
): InstalledMarketState => {
  const installed = new Set(installedSkillNames);
  return Object.fromEntries(
    Object.entries(state).filter(([, record]) => record.skill_names.every((name) => installed.has(name)))
  );
};
