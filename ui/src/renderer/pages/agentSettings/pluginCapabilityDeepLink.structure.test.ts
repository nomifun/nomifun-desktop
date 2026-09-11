import { describe, expect, test } from 'bun:test';
import { readFileSync } from 'node:fs';

const source = readFileSync(new URL('./AgentCapabilityWorkspace.tsx', import.meta.url), 'utf8');

describe('Plugin capability deep link', () => {
  test('opens the capability picker on the Plugin source and exact capability id', () => {
    expect(source.includes("searchParams.get('source')")).toBe(true);
    expect(source.includes("searchParams.get('capability')")).toBe(true);
    expect(source.includes("setPickerSource('plugin')")).toBe(true);
    expect(source.includes('setPickerOpen(true)')).toBe(true);
  });
});
