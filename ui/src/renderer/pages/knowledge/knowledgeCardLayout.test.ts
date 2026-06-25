import { describe, expect, test } from 'bun:test';
import { readFileSync } from 'node:fs';

const cardSource = readFileSync(new URL('./KnowledgeCard.tsx', import.meta.url), 'utf8');

describe('KnowledgeCard footer layout', () => {
  test('uses a lightweight footer instead of a full-width recessed meta strip', () => {
    expect(cardSource.includes('knowledge-card-footer')).toBe(true);
    expect(cardSource.includes('knowledge-card-meta')).toBe(true);
    expect(cardSource.includes('border-t border-solid border-[var(--color-border-2)]')).toBe(false);
  });

  test('keeps hover actions in footer flow instead of overlaying metadata', () => {
    expect(cardSource.includes('knowledge-card-actions')).toBe(true);
    expect(cardSource.includes('absolute bottom-16px right-16px')).toBe(false);
    expect(cardSource.includes('group-hover:pointer-events-auto')).toBe(true);
  });
});
