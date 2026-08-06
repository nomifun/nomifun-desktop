import { describe, expect, test } from 'bun:test';
import { readFileSync } from 'node:fs';

const consumersSource = readFileSync(new URL('./KnowledgeConsumersSection.tsx', import.meta.url), 'utf8');

describe('Knowledge consumers section visual style', () => {
  test('shows a bounded preview list with explicit loading and empty states', () => {
    expect(consumersSource.includes('consumers.slice(0, 3)')).toBe(true);
    expect(consumersSource.includes('knowledge-consumers-row')).toBe(true);
    expect(consumersSource.includes('knowledge-consumers-list-expanded')).toBe(true);
    expect(consumersSource.includes("t('knowledge.consumers.empty'")).toBe(true);
    expect(consumersSource.includes("t('common.loading'")).toBe(true);
    expect(consumersSource.includes('knowledge.consumers.showMore')).toBe(true);
  });

  test('uses a quiet unmount action instead of a destructive delete control', () => {
    expect(consumersSource.includes('knowledge-consumers-remove')).toBe(true);
    expect(consumersSource.includes('Unlink')).toBe(true);
    expect(consumersSource.includes('Delete')).toBe(false);
  });
});
