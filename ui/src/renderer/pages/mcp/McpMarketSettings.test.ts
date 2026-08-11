import { describe, expect, test } from 'bun:test';
import type { IMcpServer } from '@/common/config/storage';
import {
  attachMcpMarketOrigin,
  getMcpMarketOrigin,
  isMcpMarketItemInstalled,
} from './McpMarketSettings';

const marketItem = {
  id: 'skillhub_mcp:playwright',
  name: 'Playwright MCP',
};

const server = (name: string, original_json: string): IMcpServer =>
  ({ name, original_json }) as IMcpServer;

describe('MCP market installed state', () => {
  test('persists exact market provenance inside the server original JSON', () => {
    const original = JSON.stringify({ mcpServers: { browser: { command: 'npx' } } });
    const marked = attachMcpMarketOrigin(original, marketItem.id);
    const installed = server('browser', marked);

    expect(JSON.parse(marked).mcpServers.browser.command).toBe('npx');
    expect(getMcpMarketOrigin(installed)).toBe(marketItem.id);
    expect(isMcpMarketItemInstalled(marketItem, [installed])).toBe(true);
    expect(isMcpMarketItemInstalled(marketItem, [])).toBe(false);
  });

  test('recognizes legacy imports by server name or market slug', () => {
    expect(isMcpMarketItemInstalled(marketItem, [server('playwright', '{}')])).toBe(true);
    expect(isMcpMarketItemInstalled(marketItem, [server('another-server', '{}')])).toBe(false);
    expect(getMcpMarketOrigin(server('broken', '{'))).toBeNull();
  });
});
