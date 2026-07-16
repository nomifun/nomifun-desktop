import { describe, expect, test } from 'bun:test';
import { extensionMcpUiKey, parseExtensionMcpServers } from './extensionCatalog';

describe('parseExtensionMcpServers', () => {
  test('rejects unsafe non-extension keys without hiding valid siblings', () => {
    const result = parseExtensionMcpServers([
      {
        id: 'ext-web-tools-browser',
        name: 'browser',
        description: 'Browser tools contributed by an extension',
        enabled: true,
        transport: {
          type: 'stdio',
          command: 'browser-mcp',
          args: ['--serve'],
        },
        created_at: 100,
        updated_at: 200,
        original_json: '{}',
      },
      {
        id: 42,
        name: 'malformed contribution',
        transport: {
          type: 'stdio',
          command: 'ignored',
        },
      },
      { id: 'constructor', name: 'prototype property' },
      { id: '__proto__', name: 'prototype setter' },
      { id: 'mcp_0190f5fe-7c00-7a00-8000-000000000003', name: 'canonical-looking key' },
      { id: 'ext-', name: 'empty extension suffix' },
      {
        id: 'ext-search-tools-search',
        name: 'search',
        enabled: false,
        transport: {
          type: 'http',
          url: 'https://example.com/mcp',
        },
        original_json: '{"name":"search"}',
      },
      { id: 'ext-no-description', name: 'no description', description: '   ' },
    ]);

    expect(result).toEqual([
      {
        contributionKey: 'ext-web-tools-browser',
        name: 'browser',
        description: 'Browser tools contributed by an extension',
      },
      {
        contributionKey: 'ext-search-tools-search',
        name: 'search',
      },
      {
        contributionKey: 'ext-no-description',
        name: 'no description',
      },
    ]);
  });

  test('keeps the first contribution for each duplicate key', () => {
    const result = parseExtensionMcpServers([
      { id: 'ext-shared-tools', name: 'first', description: 'keep this one' },
      { id: 'ext-shared-tools', name: 'second', description: 'ignore this one' },
      { id: 'ext-safe-constructor', name: 'safe sibling' },
    ]);

    expect(result).toEqual([
      {
        contributionKey: 'ext-shared-tools',
        name: 'first',
        description: 'keep this one',
      },
      {
        contributionKey: 'ext-safe-constructor',
        name: 'safe sibling',
      },
    ]);
  });
});

describe('extensionMcpUiKey', () => {
  test('namespaces legal prototype-looking suffixes away from shared UI state keys', () => {
    const contributions = parseExtensionMcpServers([
      { id: 'ext-constructor', name: 'constructor suffix' },
      { id: 'ext-__proto__', name: 'prototype suffix' },
    ]);

    expect(contributions.map((server) => extensionMcpUiKey(server.contributionKey))).toEqual([
      'extension:ext-constructor',
      'extension:ext-__proto__',
    ]);
  });
});
