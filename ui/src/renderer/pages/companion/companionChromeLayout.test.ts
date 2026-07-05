import { describe, expect, test } from 'bun:test';
import { readFileSync } from 'node:fs';

const companionSource = readFileSync(new URL('./index.tsx', import.meta.url), 'utf8');
const companionCss = readFileSync(new URL('./companion.css', import.meta.url), 'utf8');

describe('desktop companion chrome layout', () => {
  test('anchors unread badge to the figure stage instead of the viewport top', () => {
    const stageIndex = companionSource.indexOf("className='nomi-companion-stage'");
    const badgeIndex = companionSource.indexOf("className='nomi-companion-badge'");
    const figureIndex = companionSource.indexOf('ref={figureHitRef}');

    expect(stageIndex).toBeGreaterThan(-1);
    expect(badgeIndex).toBeGreaterThan(stageIndex);
    expect(figureIndex).toBeGreaterThan(badgeIndex);
  });

  test('defines a positioned figure stage for stable badge and suggestions anchoring', () => {
    expect(companionCss.includes('.nomi-companion-stage')).toBe(true);
    expect(companionCss.includes('position: relative;')).toBe(true);
    expect(companionCss.includes('top: 10px;')).toBe(false);
  });
});
