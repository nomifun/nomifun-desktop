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

/**
 * Every focusable create CTA in a page source, as its class block.
 *
 * This ENUMERATES rather than anchoring on one marker. The previous helper took
 * the first `t('knowledge.newBase'` occurrence and scanned BACKWARDS for
 * `className={[`; once the label was hoisted into a `const newBaseLabel = t(...)`
 * above the JSX, that first occurrence had no class block before it, so the
 * helper returned -1 and both tests died inside it — while the page had
 * meanwhile grown a SECOND create CTA that nothing checked at all.
 *
 * Enumerating means a newly added CTA is covered automatically instead of
 * silently escaping, and no reworded comment or reordered attribute can quietly
 * reduce coverage to zero.
 */
function createCtaClassBlocks(source: string, handler: string): string[] {
  const blocks: string[] = [];
  // Each CTA is a focusable div: role='button' … onClick={… handler …} … className={[ … ].join(' ')}
  for (const part of source.split("role='button'").slice(1)) {
    const classStart = part.indexOf('className={[');
    if (classStart === -1) continue;
    const classEnd = part.indexOf("].join(' ')", classStart);
    if (classEnd === -1) continue;
    // Only the attributes BEFORE the class block belong to this element.
    if (!part.slice(0, classStart).includes(handler)) continue;
    blocks.push(part.slice(classStart, classEnd));
  }
  return blocks;
}

function focusBorderUtility(classBlock: string): string | undefined {
  return classBlock.split(/[\s'`]+/).find((token) => token.startsWith('focus-visible:border-'));
}

const listPageCtas = createCtaClassBlocks(listPageSource, 'openStudio()');
const emptyStateCtas = createCtaClassBlocks(emptyStateSource, 'onCreate()');
/** Everything a keyboard can land on to create a base, pills and cards alike. */
const allCtas = [...listPageCtas, ...emptyStateCtas];
/** The primary tinted pills — the list-page header CTA and the empty-state CTA. */
const pillCtas = allCtas.filter((block) => block.includes('rounded-full'));

describe('Knowledge create CTA contrast', () => {
  test('the create CTAs are actually found, so a green run cannot mean zero coverage', () => {
    // The whole failure mode above was a helper that quietly matched nothing.
    expect(listPageCtas.length).toBe(2); // header pill + add-new dashed card
    expect(emptyStateCtas.length).toBe(1);
    expect(pillCtas.length).toBe(2);
  });

  test('create CTAs use theme text instead of fixed white text', () => {
    for (const classBlock of allCtas) {
      expect(classBlock.includes('text-white')).toBe(false);
    }
    // The tinted pills sit on a translucent primary fill, so they must take the
    // theme's foreground token. The dashed placeholder card is deliberately muted
    // (text-3) and is not held to that token.
    for (const classBlock of pillCtas) {
      expect(classBlock.includes('text-[var(--color-text-1)]')).toBe(true);
    }
  });

  test('pill CTAs show no default border and gain a real focus border colour', async () => {
    for (const classBlock of pillCtas) {
      expect(classBlock.includes('border-[rgba(var(--primary-6),0.45)]')).toBe(false);
      expect(classBlock.includes('border-transparent')).toBe(true);
      expect(classBlock.includes('focus-visible:outline-none')).toBe(true);

      // Whichever utility spells the focus border, it must compile to a colour
      // the browser can parse — the CTA is otherwise unfocusable to the eye.
      const utility = focusBorderUtility(classBlock);
      expect(utility).toBeDefined();
      await resolvedBorderColor(utility as string);
    }
  });

  test('EVERY focusable create CTA has a focus indicator the eye can see', async () => {
    // role='button' + tabIndex=0 puts these in the tab order, but nothing styles
    // a bare focusable div (no global rule matches [role='button']), so a CTA
    // without its own focus-visible utilities is invisible to keyboard users.
    // The add-new dashed card shipped in exactly that state, hidden behind the
    // helper that matched nothing.
    for (const classBlock of allCtas) {
      expect(classBlock.includes('focus-visible:outline-none')).toBe(true);
      const utility = focusBorderUtility(classBlock);
      expect(utility).toBeDefined();
      await resolvedBorderColor(utility as string);
    }
  });
});
