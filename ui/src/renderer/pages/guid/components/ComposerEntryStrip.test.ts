/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { readFileSync } from 'node:fs';
import { describe, expect, test } from 'bun:test';

const readSource = (url: URL): string => readFileSync(url, 'utf8');

const classBlock = (css: string, className: string) => {
  const start = css.indexOf(`.${className} {`);
  expect(start).toBeGreaterThan(-1);
  const end = css.indexOf('\n}', start);
  return css.slice(start, end);
};

describe('Guid composer entry strip', () => {
  test('stays a transparent per-session control strip', () => {
    const source = readSource(new URL('./ComposerEntryStrip.tsx', import.meta.url));
    const css = readSource(new URL('../index.module.css', import.meta.url));
    const strip = classBlock(css, 'entryStrip');

    expect(source.includes('styles.entryStrip')).toBe(true);
    expect(css.includes('margin-bottom: 8px')).toBe(true);
    expect(strip.includes('background: transparent')).toBe(true);
    expect(strip.includes('background: color-mix')).toBe(false);
    expect(strip.includes('border-radius: 16px')).toBe(false);
  });

  test('does not expose preset-owned Skills or collaboration overrides', () => {
    const source = readSource(new URL('./ComposerEntryStrip.tsx', import.meta.url));

    for (const forbidden of [
      'activeSkills',
      'GuidActiveSkill',
      'onAdjustSkills',
      'skillsEntry',
      'collaborationPolicyNode',
      'guid.entry.skillsActive',
      'guid.skillsPopover',
      'onChoosePreset',
    ]) {
      expect(source.includes(forbidden)).toBe(false);
    }
  });

  test('keeps summon as the only explicit session entry', () => {
    const source = readSource(new URL('./ComposerEntryStrip.tsx', import.meta.url));

    expect(source.includes('onSummonCompanion?: () => void')).toBe(true);
    expect(source.includes('summonedCompanionName')).toBe(true);
    expect(source.includes('conversation.summon.button')).toBe(true);
    for (const retired of [
      'onCreateMiniApp',
      'miniAppActive',
      'miniApps.composer',
      'ApplicationOne',
      'guid-miniapp-',
    ]) {
      expect(source.includes(retired)).toBe(false);
    }

    const summonPos = source.indexOf('{summonEntry}');
    expect(summonPos).toBeGreaterThan(-1);
    expect(source.includes('{miniAppEntry}')).toBe(false);
  });

  test('keeps compact labels responsive without an unimplemented shortcut', () => {
    const source = readSource(new URL('./ComposerEntryStrip.tsx', import.meta.url));
    const css = readSource(new URL('../index.module.css', import.meta.url));

    expect(source.includes('<Tooltip')).toBe(false);
    expect(source.includes('quickSwitch')).toBe(false);
    expect(css.includes('.entryQuickHint')).toBe(false);
    expect(css.includes('container-name: guid-entry-strip')).toBe(true);
    expect(css.includes('@container guid-entry-strip (max-width: 560px)')).toBe(
      true
    );
    expect(css.includes('@media (hover: hover) and (pointer: fine)')).toBe(true);
    expect(css.includes('.entryStrip .entryButton:hover')).toBe(true);
    expect(css.includes('.entryStrip .entryButtonText')).toBe(true);
  });
});
