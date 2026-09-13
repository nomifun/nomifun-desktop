import { describe, expect, test } from 'bun:test';
import { readFileSync } from 'node:fs';

const source = readFileSync(new URL('./AgentCapabilityWorkspace.tsx', import.meta.url), 'utf8');

describe('Plugin capability deep link', () => {
  test('filters the full catalog inline on a Plugin capability deep link', () => {
    expect(source.includes("searchParams.get('source')")).toBe(true);
    expect(source.includes("searchParams.get('capability')")).toBe(true);
    expect(source.includes("setPluginsOnly(true)")).toBe(true);
    expect(source.includes("setPluginSearch(searchParams.get('capability') ?? '')")).toBe(true);
  });
});
