import { describe, expect, test } from 'bun:test';
import { readFileSync } from 'node:fs';

const cardStyles = readFileSync(new URL('./KnowledgeCard.module.css', import.meta.url), 'utf8');
const pageStyles = readFileSync(new URL('./KnowledgeListPage/index.module.css', import.meta.url), 'utf8');

function rule(styles: string, selector: string): string {
  const escaped = selector.replace(/[.*+?^${}()|[\]\\]/g, '\\$&');
  const content = styles.match(new RegExp(`${escaped}\\s*\\{([^}]*)\\}`))?.[1];
  expect(content).toBeDefined();
  return content!;
}

describe('Knowledge catalog card geometry', () => {
  test('keeps the approved card width instead of stretching cards into empty grid space', () => {
    expect(rule(pageStyles, '.grid')).toMatch(/grid-template-columns:\s*repeat\(auto-fill,\s*264px\)/);
    expect(rule(cardStyles, '.card')).toMatch(/width:\s*264px/);
  });

  test('keeps the same outer height and description space for populated, empty and tagged cards', () => {
    expect(rule(cardStyles, '.card')).toMatch(/height:\s*107px/);
    expect(rule(cardStyles, '.card')).toMatch(/box-sizing:\s*border-box/);
    expect(rule(cardStyles, '.body')).toMatch(/height:\s*36px/);
    expect(rule(cardStyles, '.body')).toMatch(/overflow:\s*hidden/);
    expect(rule(cardStyles, '.description')).toMatch(/-webkit-line-clamp:\s*2/);
    expect(rule(cardStyles, '.singleLine')).toMatch(/-webkit-line-clamp:\s*1/);
  });
});
