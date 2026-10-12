import { describe, expect, test } from 'bun:test';
import {
  filterSkillMarketItems,
  normalizeSkillMarketErrors,
  normalizeSkillMarketItem,
  normalizeSkillMarketItems,
  resolveMarketSyncItems,
  selectMarketSourceWithItems,
} from './skillMarket';

const item = {
  id: 'clawhub:owner/demo',
  source: 'clawhub' as const,
  rank: 1,
  name: 'demo skill',
  description: 'GitHub coding helper',
  url: 'https://clawhub.ai/owner/skills/demo',
  install_command: 'openclaw skills install @owner/demo',
  tags: ['developer', 'coding'],
  audience_tags: ['developer'],
  scenario_tags: ['coding'],
};

describe('skill market helpers', () => {
  test('filters by source and search', () => {
    const result = filterSkillMarketItems([item], 'clawhub', 'github');

    expect(result).toEqual([item]);
    expect(filterSkillMarketItems([item], 'skillhub', '')).toHaveLength(0);
    expect(filterSkillMarketItems([item], 'clawhub', 'missing')).toHaveLength(0);
    expect(filterSkillMarketItems([item], 'clawhub', '开发')).toEqual([]);
  });

  test('rejects unsafe cached commands and URLs', () => {
    expect(
      normalizeSkillMarketItem({
        ...item,
        install_command: 'openclaw skills install @owner/demo; rm -rf ~',
      })
    ).toBeNull();
    expect(normalizeSkillMarketItem({ ...item, url: 'https://example.com/owner/demo' })).toBeNull();
    expect(normalizeSkillMarketItem({ ...item, url: 'https://clawhub.ai:444/owner/demo' })).toBeNull();
    expect(normalizeSkillMarketItems([item, { bad: true }])).toHaveLength(1);
    expect(normalizeSkillMarketErrors(['ok', 1, 'x'.repeat(400)])).toEqual(['ok', 'x'.repeat(240)]);
  });

  test('accepts supported external market sources only with safe add commands', () => {
    const skillHubItem = {
      ...item,
      id: 'skillhub:owner/skills/demo',
      source: 'skillhub' as const,
      url: 'https://skillhub.cn/skills/owner/demo',
      install_command: 'npx skills add @owner/demo',
    };
    const loopHubItem = {
      ...item,
      id: 'loophub:12277',
      source: 'loophub' as const,
      url: 'https://hub.cocoloop.cn/skills/12277',
      install_command: 'loophub skill download https://dl.cocoloop.cn/bss/skills/demo.zip',
    };
    const mcpItem = {
      ...item,
      id: 'skillhub_mcp:playwright',
      source: 'skillhub_mcp' as const,
      url: 'https://skillhub.cn/mcp/playwright',
      install_command: 'mcp market add skillhub:playwright',
    };
    const mcpWorldItem = {
      ...item,
      id: 'mcpworld:c7897f8abf0350fbbf5a7fccc3e79bb8',
      source: 'mcpworld' as const,
      url: 'https://www.mcpworld.com/zh/detail/c7897f8abf0350fbbf5a7fccc3e79bb8',
      install_command: 'mcp market add mcpworld:c7897f8abf0350fbbf5a7fccc3e79bb8',
    };
    const pluginItem = {
      ...item,
      id: 'clawhub_plugins:openclaw/whatsapp',
      source: 'clawhub_plugins' as const,
      url: 'https://clawhub.ai/openclaw/plugins/whatsapp',
      install_command: 'openclaw plugins install clawhub:@openclaw/whatsapp',
    };
    expect(normalizeSkillMarketItems([skillHubItem, loopHubItem, mcpItem, mcpWorldItem, pluginItem])).toHaveLength(5);
    expect(normalizeSkillMarketItem({ ...skillHubItem, url: 'https://www.skills.sh/owner/skills/demo' })).toBeNull();
    expect(normalizeSkillMarketItem({ ...pluginItem, install_command: 'openclaw plugins install @x; rm -rf ~' })).toBeNull();
    expect(normalizeSkillMarketItem({ ...mcpWorldItem, url: 'https://evil.example/zh/detail/demo' })).toBeNull();
  });

  test('keeps cached market items when a sync returns no valid entries', () => {
    expect(resolveMarketSyncItems([item], [])).toEqual([item]);
    expect(resolveMarketSyncItems([], [item])).toEqual([item]);
  });

  test('selects the first configured source that has items when the active source is empty', () => {
    const loopHubItem = {
      ...item,
      id: 'loophub:12277',
      source: 'loophub' as const,
      url: 'https://hub.cocoloop.cn/skills/12277',
      install_command: 'loophub skill download https://dl.cocoloop.cn/bss/skills/demo.zip',
    };

    expect(selectMarketSourceWithItems('clawhub', ['clawhub', 'loophub', 'skillhub'], [loopHubItem])).toBe('loophub');
    expect(selectMarketSourceWithItems('loophub', ['clawhub', 'loophub', 'skillhub'], [loopHubItem])).toBe('loophub');
    expect(selectMarketSourceWithItems('clawhub', ['clawhub', 'loophub', 'skillhub'], [])).toBe('clawhub');
  });

});
