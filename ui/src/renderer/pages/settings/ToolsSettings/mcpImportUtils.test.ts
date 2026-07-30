import { describe, expect, test } from 'bun:test';
import { toImportableMcpServersFromConfig } from './mcpImportUtils';

describe('MCP import utils', () => {
  test('converts market MCP config into importable servers', () => {
    const servers = toImportableMcpServersFromConfig({
      mcpServers: {
        playwright: {
          command: 'npx @playwright/mcp@latest',
          description: 'Browser automation',
        },
      },
    });

    expect(servers).toHaveLength(1);
    expect(servers[0].name).toBe('playwright');
    expect(servers[0].enabled).toBe(true);
    expect(servers[0].transport).toEqual({
      type: 'stdio',
      command: 'npx',
      args: ['@playwright/mcp@latest'],
      env: {},
    });
    expect(servers[0].original_json.includes('"mcpServers"')).toBe(true);
  });

  test('returns an empty list for invalid market MCP config', () => {
    expect(toImportableMcpServersFromConfig({ command: 'npx demo' })).toEqual([]);
  });

  test('marks URL API key placeholders as requiring configuration', () => {
    const servers = toImportableMcpServersFromConfig({
      mcpServers: {
        appbuilder: {
          type: 'sse',
          url: 'https://example.com/sse?api_key=AppBuilder API Key',
        },
      },
    });

    expect(servers).toHaveLength(1);
    expect(servers[0].enabled).toBe(false);
    expect(servers[0].market_needs_configuration).toBe(true);
    expect(servers[0].market_configuration_fields?.includes('url.api_key')).toBe(true);
  });

  test('keeps concrete URL API key configs enabled', () => {
    const servers = toImportableMcpServersFromConfig({
      mcpServers: {
        appbuilder: {
          type: 'sse',
          url: 'https://example.com/sse?api_key=sample123',
        },
      },
    });

    expect(servers).toHaveLength(1);
    expect(servers[0].enabled).toBe(true);
    expect(servers[0].market_needs_configuration).toBe(false);
  });

  test('marks env placeholders as requiring configuration', () => {
    const servers = toImportableMcpServersFromConfig({
      mcpServers: {
        search: {
          command: 'npx',
          args: ['search-mcp'],
          env: { SEARCH_API_KEY: 'YOUR_API_KEY_HERE' },
        },
      },
    });

    expect(servers).toHaveLength(1);
    expect(servers[0].enabled).toBe(false);
    expect(servers[0].market_configuration_fields?.includes('env.SEARCH_API_KEY')).toBe(true);
  });

  test('keeps concrete HTTP config enabled', () => {
    const servers = toImportableMcpServersFromConfig({
      mcpServers: {
        publicDocs: {
          type: 'http',
          url: 'https://example.com/mcp?mode=public',
        },
      },
    });

    expect(servers).toHaveLength(1);
    expect(servers[0].enabled).toBe(true);
    expect(servers[0].market_needs_configuration).toBe(false);
  });
});
