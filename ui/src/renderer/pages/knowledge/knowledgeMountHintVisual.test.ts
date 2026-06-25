import { describe, expect, test } from 'bun:test';
import { readFileSync } from 'node:fs';

const detailSource = readFileSync(new URL('./KnowledgeDetailPage/index.tsx', import.meta.url), 'utf8');

describe('Knowledge detail mount hint visual style', () => {
  test('keeps the mount guidance inline with the mounted heading instead of a standalone row', () => {
    const titleIndex = detailSource.indexOf("t('knowledge.detail.use.mountedTitle'");
    const hintIndex = detailSource.indexOf('knowledge-mount-hint');
    const consumersIndex = detailSource.indexOf('{base ? <KnowledgeConsumersSection');

    expect(detailSource.includes('knowledge-mount-heading')).toBe(true);
    expect(detailSource.includes('knowledge-mount-hint')).toBe(true);
    expect(titleIndex).toBeGreaterThan(-1);
    expect(hintIndex).toBeGreaterThan(titleIndex);
    expect(hintIndex).toBeLessThan(consumersIndex);
    expect(detailSource.includes('knowledge-mount-hint mt-10px')).toBe(false);
    expect(detailSource.includes("className='mt-12px flex items-start gap-8px rd-8px bg-[var(--color-fill-2)] px-12px py-10px'")).toBe(false);
  });
});
