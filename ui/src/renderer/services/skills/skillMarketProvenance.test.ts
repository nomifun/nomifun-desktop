import type { ISkillMarketItem } from '@/common/adapter/ipcBridge';
import { afterEach, describe, expect, mock, test } from 'bun:test';
import { resolveSkillDisplay } from './skillDisplay';
import {
  INSTALLED_MARKET_KEY,
  getInstalledMarketSnapshot,
  notifySkillMarketCacheChanged,
  readInstalledMarketState,
  reconcileInstalledMarketState,
  recordInstalledMarketItem,
  SKILL_MARKET_CACHE_KEY,
  subscribeInstalledMarketState,
  writeInstalledMarketState,
} from './skillMarketProvenance';

const LEGACY_INSTALLED_MARKET_KEY = 'nomifun.skillMarket.installed.v1';

const item: ISkillMarketItem = {
  id: 'skillhub:author/dev-expert',
  source: 'skillhub',
  rank: 1,
  name: '编程专家.Skill',
  description: 'GitHub coding helper',
  url: 'https://skillhub.cn/skills/author/dev-expert',
  install_command: 'npx skills add @author/dev-expert',
  tags: ['developer'],
  audience_tags: ['developer'],
  scenario_tags: ['coding'],
};

afterEach(() => {
  writeInstalledMarketState({});
  localStorage.removeItem(SKILL_MARKET_CACHE_KEY);
  localStorage.removeItem(INSTALLED_MARKET_KEY);
  localStorage.removeItem(LEGACY_INSTALLED_MARKET_KEY);
});

describe('shared presentation subscriptions', () => {
  test('keeps snapshot identity stable until the exact provenance or market cache changes', () => {
    writeInstalledMarketState(recordInstalledMarketItem({}, item, ['dev-expert']));
    const first = getInstalledMarketSnapshot();
    expect(getInstalledMarketSnapshot()).toBe(first);
    localStorage.setItem('unrelated-setting', 'unchanged');
    expect(getInstalledMarketSnapshot()).toBe(first);
    localStorage.removeItem('unrelated-setting');

    const refreshed = { ...item, name: 'New official title', description: 'New official description' };
    localStorage.setItem(SKILL_MARKET_CACHE_KEY, JSON.stringify({ items: [refreshed] }));
    const next = getInstalledMarketSnapshot();
    expect(next).not.toBe(first);
    expect(next[item.id].presentation?.name).toBe(refreshed.name);
    expect(getInstalledMarketSnapshot()).toBe(next);
  });

  test('publishes installs, refreshes and other-window changes but ignores unrelated storage', () => {
    const changed = mock(() => {});
    const unsubscribe = subscribeInstalledMarketState(changed);
    try {
      writeInstalledMarketState(recordInstalledMarketItem({}, item, ['dev-expert']));
      expect(changed).toHaveBeenCalledTimes(1);
      notifySkillMarketCacheChanged('nomifun.mcpMarket.rankings');
      window.dispatchEvent(new StorageEvent('storage', { key: 'unrelated' }));
      expect(changed).toHaveBeenCalledTimes(1);
      notifySkillMarketCacheChanged(SKILL_MARKET_CACHE_KEY);
      expect(changed).toHaveBeenCalledTimes(2);
      window.dispatchEvent(new StorageEvent('storage', { key: INSTALLED_MARKET_KEY }));
      expect(changed).toHaveBeenCalledTimes(3);
      window.dispatchEvent(new StorageEvent('storage', { key: null }));
      expect(changed).toHaveBeenCalledTimes(4);
    } finally { unsubscribe(); }
    notifySkillMarketCacheChanged(SKILL_MARKET_CACHE_KEY);
    expect(changed).toHaveBeenCalledTimes(4);
  });
});

describe('installed market presentation provenance', () => {
  test('migrates exact v1 identity and preserves the market presentation', () => {
    localStorage.setItem(
      SKILL_MARKET_CACHE_KEY,
      JSON.stringify({ fetched_at: 1, items: [item], errors: [] })
    );
    localStorage.setItem(
      LEGACY_INSTALLED_MARKET_KEY,
      JSON.stringify({ [item.id]: ['dev-expert'] })
    );

    const state = readInstalledMarketState();
    const presentation = resolveSkillDisplay({ name: 'dev-expert', source: 'custom', location: '/skills/dev-expert/SKILL.md' }, 'zh-CN', state);

    expect(state[item.id]?.skill_names).toEqual(['dev-expert']);
    expect(presentation).toEqual({
      name: item.name,
      description: item.description,
    });
  });

  test('keeps one exact market owner for a canonical installed skill', () => {
    const otherOwner: ISkillMarketItem = {
      ...item,
      id: 'skillhub:other/dev-expert',
      name: 'Another Dev Expert',
      url: 'https://skillhub.cn/skills/other/dev-expert',
      install_command: 'npx skills add @other/dev-expert',
    };

    const first = recordInstalledMarketItem({}, item, ['dev-expert']);
    const second = recordInstalledMarketItem(first, otherOwner, ['dev-expert']);

    expect(Object.keys(second)).toEqual([otherOwner.id]);
    expect(resolveSkillDisplay({ name: 'dev-expert', source: 'custom', location: '/skills/dev-expert/SKILL.md' }, 'en-US', second).name).toBe(
      otherOwner.name
    );
  });

  test('drops stale provenance when its canonical skill is no longer installed', () => {
    const state = recordInstalledMarketItem({}, item, ['dev-expert']);

    expect(reconcileInstalledMarketState(state, ['different-skill'])).toEqual({});
    expect(reconcileInstalledMarketState(state, ['dev-expert'])).toEqual(state);
  });

  test('does not assign one package title to every skill from a multi-skill archive', () => {
    const state = recordInstalledMarketItem({}, item, ['dev-expert', 'dev-reviewer']);

    expect(resolveSkillDisplay({ name: 'dev-expert', source: 'custom', location: '/skills/dev-expert/SKILL.md' }, 'zh-CN', state).name).toBe('dev-expert');
    expect(resolveSkillDisplay({ name: 'dev-reviewer', source: 'custom', location: '/skills/dev-reviewer/SKILL.md' }, 'zh-CN', state).name).toBe('dev-reviewer');
  });
});
