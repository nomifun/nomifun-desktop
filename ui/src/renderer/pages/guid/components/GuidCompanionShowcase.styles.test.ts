import { describe, expect, test } from 'bun:test';
import { readFileSync } from 'node:fs';

const css = readFileSync(new URL('./GuidCompanionShowcase.module.css', import.meta.url), 'utf8');

describe('GuidCompanionShowcase styles', () => {
  test('distributes collapsed companions evenly and centers each item', () => {
    expect(css).toContain('.compactStage { display: grid; align-items: center; gap: 8px;');
    expect(css).toContain('.compactTile { display: flex; align-items: center; justify-content: center;');
  });
});
