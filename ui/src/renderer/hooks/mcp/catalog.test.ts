import { describe, expect, test } from 'bun:test';
import type { IMcpServer } from '@/common/config/storage';
import { toSessionMcpServer } from './catalog';

const transport: IMcpServer['transport'] = {
  type: 'stdio',
  command: 'npx',
  args: ['-y', '@modelcontextprotocol/server-everything'],
};

describe('toSessionMcpServer', () => {
  test('serializes a catalog integer id as a session string id', () => {
    expect(toSessionMcpServer({ id: 3, name: 'everything', transport })).toEqual({
      id: '3',
      name: 'everything',
      transport,
    });
  });
});
