import { readFileSync } from 'node:fs';
import { describe, expect, test } from 'bun:test';

const source = readFileSync(new URL('./useMcpConnection.ts', import.meta.url), 'utf8');

describe('MCP connection check messages', () => {
  test('uses the global Arco message container shared by model health checks', () => {
    expect(source.includes("import { Message } from '@arco-design/web-react';")).toBe(true);
    expect(source.includes("import { globalMessageQueue } from './messageQueue';")).toBe(false);
    expect(source.includes('Message.warning({')).toBe(true);
    expect(source.includes('Message.success({')).toBe(true);
    expect(source.includes('Message.error({')).toBe(true);
  });
});
