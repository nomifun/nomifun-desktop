import { describe, expect, test } from 'bun:test';
import { readFileSync } from 'node:fs';

const css = readFileSync(new URL('./GuidAgentSelector.module.css', import.meta.url), 'utf8');

const rule = (selector: string): string =>
  css.match(new RegExp(`\\.${selector}\\s*\\{([\\s\\S]*?)\\}`))?.[1] ?? '';

describe('GuidAgentSelector styles', () => {
  test('keeps the homepage trigger compact and centers its icon with the label', () => {
    expect(rule('trigger').includes('align-items: center;')).toBe(true);
    expect(rule('trigger').includes('font-size: 14px;')).toBe(true);
    expect(rule('trigger').includes('font-weight: 600;')).toBe(true);
    expect(rule('trigger').includes('line-height: 20px;')).toBe(true);
    expect(css.includes(".trigger :global(.i-icon) { display: inline-flex; align-items: center; justify-content: center; line-height: 0; }")).toBe(true);
    expect(css.includes(".trigger :global(.i-icon) > svg { display: block; }")).toBe(true);
  });

  test('keeps its popup opaque and aligns compact rows around one center line', () => {
    expect(rule('panel').includes('background: var(--bg-base);')).toBe(true);
    expect(rule('panel').includes('width: min(344px, calc(100vw - 24px));')).toBe(true);
    expect(rule('panel').includes('max-height: min(480px, calc(100vh - 24px));')).toBe(true);
    expect(rule('row').includes('min-height: 34px;')).toBe(true);
    expect(rule('row').includes('align-items: center;')).toBe(true);
    expect(rule('row').includes('gap: 8px;')).toBe(true);
    expect(rule('rowIcon').includes('align-items: center;')).toBe(true);
    expect(rule('rowIcon').includes('line-height: 0;')).toBe(true);
    expect(rule('rowCopy').includes('justify-content: center;')).toBe(true);
    expect(rule('searchWrap').includes('min-height: 32px;')).toBe(true);
    expect(rule('searchInput').includes('padding: 5px 0;')).toBe(true);
    expect(rule('moreButton').includes('width: calc(100% - 20px);')).toBe(true);
    expect(rule('moreButton').includes('min-height: 30px;')).toBe(true);
    expect(rule('templateGrid').includes('display: grid;')).toBe(true);
    expect(rule('templateGrid').includes('grid-template-columns: repeat(2, minmax(0, 1fr));')).toBe(true);
  });
});
