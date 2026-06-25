import { describe, expect, test } from 'bun:test';
import { readFileSync } from 'node:fs';

const consumersSource = readFileSync(new URL('./KnowledgeConsumersSection.tsx', import.meta.url), 'utf8');

describe('Knowledge consumers section visual style', () => {
  test('uses a soft disclosure list instead of a hard bordered dropdown', () => {
    expect(consumersSource.includes('knowledge-consumers-disclosure')).toBe(true);
    expect(consumersSource.includes('knowledge-consumers-row')).toBe(true);
    expect(consumersSource.includes('rd-8px border border-solid border-border-2 bg-fill-1')).toBe(false);
    expect(consumersSource.includes('border-t border-solid border-border-2')).toBe(false);
    expect(consumersSource.includes('shadow-[inset_0_0_0_1px_rgba(var(--primary-6),0.08)]')).toBe(true);
  });
});
