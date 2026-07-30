import { describe, expect, test } from 'bun:test';
import { getMcpApiKeyUrl, getMcpConfigurationFields, supportsMcpOAuthLogin } from './mcpAuthConfig';

describe('MCP auth config detection', () => {
  test('treats AppBuilder API key placeholders as configuration, not OAuth', () => {
    const transport = {
      type: 'sse' as const,
      url: 'http://appbuilder.baidu.com/v2/ai_search/mcp/sse?api_key=AppBuilder API Key',
    };

    expect(getMcpConfigurationFields(transport)).toEqual(['url.api_key']);
    expect(getMcpApiKeyUrl(transport)).toBe('https://appbuilder.baidu.com/console');
    expect(supportsMcpOAuthLogin(transport)).toBe(false);
  });

  test('allows concrete API key URL configs to be tested without OAuth', () => {
    const transport = {
      type: 'sse' as const,
      url: 'http://appbuilder.baidu.com/v2/ai_search/mcp/sse?api_key=sample123',
    };

    expect(getMcpConfigurationFields(transport)).toEqual([]);
    expect(getMcpApiKeyUrl(transport)).toBe('https://appbuilder.baidu.com/console');
    expect(supportsMcpOAuthLogin(transport)).toBe(false);
  });

  test('keeps ordinary URL MCP servers OAuth-capable', () => {
    expect(supportsMcpOAuthLogin({ type: 'sse', url: 'https://example.com/mcp/sse' })).toBe(true);
    expect(getMcpApiKeyUrl({ type: 'sse', url: 'https://example.com/mcp/sse' })).toBeNull();
  });
});
