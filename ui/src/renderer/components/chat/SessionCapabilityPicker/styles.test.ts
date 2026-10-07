import { describe, expect, test } from 'bun:test';
import { readFileSync } from 'node:fs';

const css = readFileSync(new URL('./styles.module.css', import.meta.url), 'utf8');
const source = readFileSync(new URL('./index.tsx', import.meta.url), 'utf8');

const rule = (selector: string): string =>
  css.match(new RegExp(`\\.${selector}\\s*\\{([\\s\\S]*?)\\}`))?.[1] ?? '';

describe('SessionCapabilityPicker styles', () => {
  test('uses the shared puzzle icon for the Skills entry', () => {
    expect(source.includes("import { CheckOne, CloseSmall, Lightning, Puzzle, Right } from '@icon-park/react';")).toBe(true);
    expect(source.includes("<Puzzle theme='outline' size={18} fill='currentColor' />")).toBe(true);
    expect(source.includes('MagicHat')).toBe(false);
  });

  test('uses an opaque popup surface with compact, center-aligned rows', () => {
    expect(rule('panel').includes('background: var(--bg-base);')).toBe(true);
    expect(rule('row').includes('min-height: 42px;')).toBe(true);
    expect(rule('row').includes('align-items: center;')).toBe(true);
    expect(rule('row').includes('column-gap: 8px;')).toBe(true);
    expect(rule('checkboxCell').includes('justify-content: center;')).toBe(true);
    expect(rule('copy').includes('gap: 0;')).toBe(true);
    expect(rule('header h3').includes('font-size: 13px;')).toBe(true);
    expect(rule('header h3').includes('font-weight: 500;')).toBe(true);
  });
});
