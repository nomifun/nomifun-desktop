import { describe, expect, test } from 'bun:test';
import {
  buildSkillMarketInstallPrompt,
  filterSkillMarketItems,
  normalizeSkillMarketErrors,
  normalizeSkillMarketItem,
  normalizeSkillMarketItems,
  resolveMarketSyncItems,
  selectMarketSourceWithItems,
  translateMarketDescription,
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
  test('filters by source, search, and shared tags', () => {
    const result = filterSkillMarketItems([item], 'clawhub', 'github', {
      audience: ['developer'],
      scenario: ['coding'],
    });

    expect(result).toEqual([item]);
    expect(filterSkillMarketItems([item], 'skillhub', '', { audience: [], scenario: [] })).toHaveLength(0);
    expect(filterSkillMarketItems([item], 'clawhub', 'missing', { audience: [], scenario: [] })).toHaveLength(0);
    expect(filterSkillMarketItems([item], 'clawhub', '开发', { audience: [], scenario: [] })).toEqual([item]);
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
    const packageItem = {
      ...item,
      id: 'skillhub_packages:tech-test-automation',
      source: 'skillhub_packages' as const,
      url: 'https://skillhub.cn/skillspackage/tech-test-automation',
      install_command: 'skillhub package add tech-test-automation',
    };

    expect(normalizeSkillMarketItems([loopHubItem, mcpItem, mcpWorldItem, pluginItem, packageItem])).toHaveLength(5);
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

  test('builds a draft prompt containing the install command', () => {
    const prompt = buildSkillMarketInstallPrompt(item);
    expect(prompt.includes('请帮我安装这个技能')).toBe(true);
    expect(prompt.includes('openclaw skills install @owner/demo')).toBe(true);
    expect(prompt.includes('https://clawhub.ai/owner/skills/demo')).toBe(true);

    const englishPrompt = buildSkillMarketInstallPrompt(item, 'en-US');
    expect(englishPrompt.includes('ask for confirmation')).toBe(true);
    expect(englishPrompt.includes('Install command:')).toBe(true);
  });

  test('translates common market descriptions for zh display', () => {
    expect(translateMarketDescription('Ranked SkillHub skill from vercel-labs/skills.', item)).toBe(
      '来自 vercel-labs/skills 的 SkillHub 榜单技能。'
    );
    expect(translateMarketDescription('GitHub coding helper', item).includes('开发')).toBe(true);
  });
});
