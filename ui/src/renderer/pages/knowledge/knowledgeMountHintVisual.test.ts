import { describe, expect, test } from 'bun:test';
import { readFileSync } from 'node:fs';

const consumersSource = readFileSync(new URL('./KnowledgeConsumersSection.tsx', import.meta.url), 'utf8');
const detailSource = readFileSync(new URL('./KnowledgeDetailPage/index.tsx', import.meta.url), 'utf8');

describe('Knowledge detail mount hint visual style', () => {
  test('keeps mount status in the main pane and rules in a dedicated side rail', () => {
    const titleIndex = consumersSource.indexOf("t('knowledge.detail.use.mountedTitle'");
    const hintIndex = consumersSource.indexOf("t('knowledge.detail.use.mountHint'");

    expect(detailSource.includes('knowledge-use-shell')).toBe(true);
    expect(detailSource.includes('knowledge-use-rules')).toBe(true);
    expect(detailSource.includes("t('knowledge.detail.use.rulesTitle'")).toBe(true);
    expect(titleIndex).toBeGreaterThan(-1);
    expect(hintIndex).toBeGreaterThan(titleIndex);
    expect(detailSource.includes('grid-cols-[minmax(0,1fr)_320px]')).toBe(true);
  });
});
