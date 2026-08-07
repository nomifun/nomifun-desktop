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

  test('surfaces missing local folders directly on the card', () => {
    expect(cardSource.includes('knowledge-card-root-missing')).toBe(true);
    expect(cardSource.includes('!base.root_exists')).toBe(true);
    expect(cardSource.includes("t('knowledge.card.rootMissing'")).toBe(true);
  });

  test('uses a delete icon for the destructive card action', () => {
    expect(cardSource.includes('MoreOne')).toBe(false);
    expect(cardSource.includes("title={t('knowledge.actions.delete'")).toBe(true);
    expect(cardSource.includes("<Delete theme='outline' size={13} strokeWidth={3} />")).toBe(true);
  });

  test('caps visible tags and exposes the complete tag list in a tooltip', () => {
    expect(cardSource.includes('const MAX_VISIBLE_TAGS = 5')).toBe(true);
    expect(cardSource.includes('resolved.slice(0, MAX_VISIBLE_TAGS)')).toBe(true);
    expect(cardSource.includes('+{overflowCount}')).toBe(true);
    expect(cardSource.includes('<Tooltip content={tooltipContent}')).toBe(true);
  });

  test('strictly clips descriptions to two lines inside compact cards', () => {
    expect(cardSource.includes('max-h-40px min-h-0 flex-1 overflow-hidden')).toBe(true);
    expect(cardSource.includes('WebkitLineClamp: 2')).toBe(true);
  });
});
