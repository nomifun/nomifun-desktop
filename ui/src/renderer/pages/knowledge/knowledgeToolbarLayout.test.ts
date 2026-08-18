import { describe, expect, test } from 'bun:test';
import { readFileSync } from 'node:fs';

const toolbarSource = readFileSync(new URL('./KnowledgeTagFilterBar.tsx', import.meta.url), 'utf8');
const toolbarStyles = readFileSync(new URL('./KnowledgeTagFilterBar.module.css', import.meta.url), 'utf8');

describe('Knowledge toolbar layout', () => {
  test('uses a compact width for the short kind selector', () => {
    const kindSelectStart = toolbarSource.indexOf("label={t('knowledge.filter.kindLabel'");
    const tagSelectStart = toolbarSource.indexOf("label={t('knowledge.filter.tagLabel'", kindSelectStart);
    const kindSelect = toolbarSource.slice(kindSelectStart, tagSelectStart);

    expect(kindSelectStart).toBeGreaterThan(-1);
    expect(tagSelectStart).toBeGreaterThan(kindSelectStart);
    expect(kindSelect.includes("minWidthClass='min-w-116px'")).toBe(true);
  });

  test('keeps collapsed action icons dark, unfilled, and tightly spaced', () => {
    expect(toolbarStyles.includes('gap: 0 !important;')).toBe(true);
    expect(toolbarStyles.includes('width: 30px !important;')).toBe(true);
    expect(toolbarStyles.includes('color: var(--color-text-1) !important;')).toBe(true);
    expect(toolbarStyles.match(/background: transparent !important;/g)?.length).toBeGreaterThanOrEqual(2);
  });

  test('does not reflow the whole action group when a popup icon receives focus', () => {
    expect(toolbarStyles.includes('.desktopActions:focus-within')).toBe(false);
    expect(toolbarStyles.includes('.desktopActions:has(.desktopSearchInput:focus)')).toBe(true);
    expect(toolbarStyles.includes('.desktopSearch:focus-within')).toBe(true);
  });
});
