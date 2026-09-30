import { describe, expect, test } from 'bun:test';
import { readFileSync } from 'node:fs';

const css = readFileSync(new URL('./GuidCompanionShowcase.module.css', import.meta.url), 'utf8');

describe('GuidCompanionShowcase styles', () => {
  test('distributes collapsed companions evenly inside unified, name-friendly controls', () => {
    expect(css).toContain('.compactStage { display: grid; align-items: center; gap: 8px;');
    expect(css).toContain('.compactTile { display: flex; align-items: center; justify-self: center;');
    expect(css).toContain('.compactTile:hover, .compactTile:focus-within { background: var(--color-fill-2); }');
    expect(css).toContain(".compactTile[data-selected='true']");
    expect(css).toContain('.compactCompanion .companionName { flex: 1 1 auto; min-width: 0; max-width: none;');
    expect(css).not.toContain('max-width: 150px');
  });
});
