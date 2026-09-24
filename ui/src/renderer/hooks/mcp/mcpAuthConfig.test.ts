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

  test('flags placeholder env values on stdio transports', () => {
    const transport = {
      type: 'stdio' as const,
      command: 'npx',
      args: ['search-mcp'],
      env: { SEARCH_API_KEY: 'YOUR_API_KEY_HERE' },
    };

    expect(getMcpConfigurationFields(transport)).toEqual(['env.SEARCH_API_KEY']);
    expect(getMcpApiKeyUrl(transport)).toBeNull();
    expect(supportsMcpOAuthLogin(transport)).toBe(false);
  });

  test('flags non-secret setup fields, placeholder args, and redacted URL paths', () => {
    expect(
      getMcpConfigurationFields({
        type: 'stdio',
        command: 'npx',
        args: ['-y', 'demo-mcp', '--workspace', '<workspace-id>'],
        env: { MEMOS_USER_ID: 'your-user-id' },
      })
    ).toEqual(['args.3', 'env.MEMOS_USER_ID']);

    expect(
      getMcpConfigurationFields({
        type: 'streamable_http',
        url: 'https://mcp.example.com/xxxxx/mcp',
      })
    ).toEqual(['url']);
  });

  test('flags placeholders in any header or URL query value', () => {
    const fields = getMcpConfigurationFields({
      type: 'http',
      url: 'https://example.com/mcp?workspace=YOUR_WORKSPACE_ID',
      headers: { 'X-Account': '${ACCOUNT_ID}' },
    });

    expect(fields).toEqual(['url.workspace', 'headers.X-Account']);
  });

  test('keeps ordinary URL MCP servers OAuth-capable', () => {
    expect(supportsMcpOAuthLogin({ type: 'sse', url: 'https://example.com/mcp/sse' })).toBe(true);
    expect(getMcpApiKeyUrl({ type: 'sse', url: 'https://example.com/mcp/sse' })).toBeNull();
  });

  test('does not mistake concrete values containing api-key words for placeholders', () => {
    expect(
      getMcpConfigurationFields({
        type: 'http',
        url: 'https://example.com/api-key/mcp',
        headers: { Authorization: 'Bearer production-api-key-123' },
      })
    ).toEqual([]);
  });
});
