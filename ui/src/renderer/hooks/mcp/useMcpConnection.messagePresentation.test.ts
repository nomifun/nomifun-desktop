import { readFileSync } from 'node:fs';
import { describe, expect, test } from 'bun:test';

const source = readFileSync(new URL('./useMcpConnection.ts', import.meta.url), 'utf8');

describe('MCP connection check messages', () => {
  test('uses the global Arco message container shared by model health checks', () => {
    expect(source).toMatch(/import\s+\{\s*Message\s*\}\s+from\s+["']@arco-design\/web-react["']/);
    expect(source).not.toMatch(/import\s+\{\s*globalMessageQueue\s*\}\s+from\s+["']\.\/messageQueue["']/);
    expect(source.includes('Message.warning({')).toBe(true);
    expect(source.includes('Message.success({')).toBe(true);
    expect(source.includes('Message.error({')).toBe(true);
  });

  test('never renders a synthetic HTTP unknown status', () => {
    expect(source).not.toMatch(/details\.status\s*\?\?\s*["']unknown["']/);
    expect(source).toMatch(/typeof\s+details\.status\s*!==\s*["']number["']/);
  });

  test('uses typed diagnostics instead of blaming every failure on MCP JSON', () => {
    expect(source).toMatch(/details\.endpoint_scope\s*===\s*["']local["']/);
    expect(source).toMatch(/details\.phase\s*===\s*["']package_bootstrap["']/);
    expect(source).toMatch(/details\.failure_kind\s*===\s*["']process_exited["']/);
    expect(source.includes('Please review the MCP JSON configuration and test again.')).toBe(false);
  });

  test('blocks placeholder configs before invoking the backend', () => {
    expect(source.indexOf('getMcpConfigurationFields(server.transport)')).toBeGreaterThan(-1);
    expect(source.indexOf('getMcpConfigurationFields(server.transport)')).toBeLessThan(
      source.indexOf('mcpService.testMcpConnection.invoke')
    );
  });
});
