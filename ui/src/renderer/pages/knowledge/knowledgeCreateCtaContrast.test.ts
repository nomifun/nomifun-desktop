import { describe, expect, test } from 'bun:test';
import { readFileSync } from 'node:fs';
import { createGenerator } from 'unocss';
import unoConfig from '../../../../uno.config';

const listPageSource = readFileSync(new URL('./KnowledgeListPage/index.tsx', import.meta.url), 'utf8');
const emptyStateSource = readFileSync(new URL('./KnowledgeEmptyState.tsx', import.meta.url), 'utf8');

const uno = await createGenerator(unoConfig);

/**
 * Asserts INTENT, not a literal class name: whatever utility the CTA uses must
 * actually reach the browser as a concrete `border-color`.
 *
 * The historical bug this replaces was `focus-visible:border-[rgb(var(--primary-6))]`,
 * which UnoCSS compiles to `rgb(var(--primary-6) / var(--un-border-opacity))`. The
 * ramp variables are comma-separated triplets, so that expands to
 * `rgb(232, 23, 74 / 1)` — unparseable, and the browser drops the whole
 * declaration, leaving the focus ring invisible. A string assertion could not
 * tell the difference; compiling the class and inspecting the declaration can.
 */
async function resolvedBorderColor(utility: string): Promise<string> {
  const { css } = await uno.generate(utility, { preflights: false });
  const declaration = css.match(/border(?:-[a-z]+)?-color\s*:\s*([^;}]+)/)?.[1]?.trim() ?? '';

  // The utility must not have been dropped on the floor by the generator.
  expect(css.trim()).not.toBe('');
  expect(declaration).not.toBe('');
  // Slash-alpha injected into a comma-triplet var() is exactly the dead form.
  expect(/\/\s*var\(--un-/.test(declaration)).toBe(false);
  // A real contrasting colour, not "keep whatever you inherited".
  expect(['transparent', 'currentColor', 'inherit', 'unset', 'initial'].includes(declaration)).toBe(false);

  return declaration;
}

function classBlockBefore(source: string, marker: string): string {
  const markerIndex = source.indexOf(marker);
  expect(markerIndex).toBeGreaterThan(-1);

  const classStart = source.lastIndexOf('className={[', markerIndex);
  expect(classStart).toBeGreaterThan(-1);

  const classEnd = source.indexOf("].join(' ')", classStart);
  expect(classEnd).toBeGreaterThan(classStart);

  return source.slice(classStart, classEnd);
}

describe('Knowledge create CTA contrast', () => {
  test('list page header create button uses theme text instead of fixed white text', () => {
    const classBlock = classBlockBefore(listPageSource, "t('knowledge.newBase'");

    expect(classBlock.includes('text-white')).toBe(false);
    expect(classBlock.includes('text-[var(--color-text-1)]')).toBe(true);
  });

  test('empty state primary create button uses theme text instead of fixed white text', () => {
    const classBlock = classBlockBefore(emptyStateSource, "t('knowledge.newBase'");

    expect(classBlock.includes('text-white')).toBe(false);
    expect(classBlock.includes('text-[var(--color-text-1)]')).toBe(true);
  });

  test('create buttons show no default border and gain a real focus border colour', async () => {
    const classBlocks = [
      classBlockBefore(listPageSource, "t('knowledge.newBase'"),
      classBlockBefore(emptyStateSource, "t('knowledge.newBase'"),
    ];

    for (const classBlock of classBlocks) {
      expect(classBlock.includes('border-[rgba(var(--primary-6),0.45)]')).toBe(false);
      expect(classBlock.includes('border-transparent')).toBe(true);
      expect(classBlock.includes('focus-visible:outline-none')).toBe(true);

      // Whichever utility spells the focus border, it must compile to a colour
      // the browser can parse — the CTA is otherwise unfocusable to the eye.
      const focusBorderUtility = classBlock
        .split(/[\s'`]+/)
        .find((token) => token.startsWith('focus-visible:border-'));
      expect(focusBorderUtility).toBeDefined();
      await resolvedBorderColor(focusBorderUtility as string);
    }
  });
});
