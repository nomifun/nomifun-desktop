import { describe, expect, test } from 'bun:test';
import { readFileSync } from 'node:fs';
import { createGenerator } from 'unocss';
import unoConfig from '../../../../uno.config';

const listPageSource = readFileSync(new URL('./KnowledgeListPage/index.tsx', import.meta.url), 'utf8');
const emptyStateSource = readFileSync(new URL('./KnowledgeEmptyState.tsx', import.meta.url), 'utf8');
const uno = await createGenerator(unoConfig);

function emptyStateCreateClassBlocks(): string[] {
  const blocks: string[] = [];
  for (const part of emptyStateSource.split("role='button'").slice(1)) {
    const start = part.indexOf('className={[');
    const end = part.indexOf("].join(' ')", start);
    if (start === -1 || end === -1 || !part.slice(0, start).includes('onCreate()')) continue;
    blocks.push(part.slice(start, end));
  }
  return blocks;
}

describe('Knowledge create CTA contrast', () => {
  test('uses one shared primary button for page-level creation', () => {
    const buttons = [...listPageSource.matchAll(/<Button\b[\s\S]*?<\/Button>/g)]
      .map(([button]) => button)
      .filter(button => /onClick=\{\(\)\s*=>\s*openStudio\(\)\}/.test(button));

    expect(buttons).toHaveLength(1);
    expect(buttons[0]).toMatch(/\btype=['"]primary['"]/);
    expect(buttons[0]).not.toContain('text-white');
    expect(listPageSource).not.toContain('border-dashed');
  });

  test('the existing empty-state create CTA retains theme text and a visible focus border', async () => {
    const blocks = emptyStateCreateClassBlocks();
    expect(blocks).toHaveLength(1);

    for (const block of blocks) {
      expect(block).toContain('text-[var(--color-text-1)]');
      expect(block).not.toContain('text-white');
      const utility = block.split(/[\s'`]+/).find(token => token.startsWith('focus-visible:border-'));
      expect(utility).toBeDefined();

      const { css } = await uno.generate(utility!, { preflights: false });
      const declaration = css.match(/border(?:-[a-z]+)?-color\s*:\s*([^;}]+)/)?.[1]?.trim() ?? '';
      expect(declaration).not.toBe('');
      // Uno must not inject slash alpha into comma-separated theme ramps.
      expect(/\/\s*var\(--un-/.test(declaration)).toBe(false);
      expect(['transparent', 'currentColor', 'inherit', 'unset', 'initial']).not.toContain(declaration);
    }
  });
});
