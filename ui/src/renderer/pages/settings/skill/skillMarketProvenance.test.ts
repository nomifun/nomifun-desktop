import type { ISkillMarketItem } from '@/common/adapter/ipcBridge';
import { afterEach, describe, expect, test } from 'bun:test';
import { translateMarketDescription } from './skillMarket';
import {
  INSTALLED_MARKET_KEY,
  readInstalledMarketState,
  reconcileInstalledMarketState,
  recordInstalledMarketItem,
  resolveInstalledMarketPresentation,
  SKILL_MARKET_CACHE_KEY,
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
  localStorage.removeItem(SKILL_MARKET_CACHE_KEY);
  localStorage.removeItem(INSTALLED_MARKET_KEY);
  localStorage.removeItem(LEGACY_INSTALLED_MARKET_KEY);
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
    const presentation = resolveInstalledMarketPresentation(state, 'dev-expert', 'zh-CN');

    expect(state[item.id]?.skill_names).toEqual(['dev-expert']);
    expect(presentation).toEqual({
      marketItemId: item.id,
      source: item.source,
      name: item.name,
      description: translateMarketDescription(item.description, item, 'zh-CN'),
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
    expect(resolveInstalledMarketPresentation(second, 'dev-expert', 'en-US')?.name).toBe(
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

    expect(resolveInstalledMarketPresentation(state, 'dev-expert', 'zh-CN')).toBeUndefined();
    expect(resolveInstalledMarketPresentation(state, 'dev-reviewer', 'zh-CN')).toBeUndefined();
  });
});
