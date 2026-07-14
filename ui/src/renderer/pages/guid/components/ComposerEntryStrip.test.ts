/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { readFileSync } from 'node:fs';
import { describe, expect, test } from 'bun:test';

const readSource = (url: URL) => readFileSync(url, 'utf8');

const classBlock = (css: string, className: string) => {
  const start = css.indexOf(`.${className} {`);
  expect(start).toBeGreaterThan(-1);
  const end = css.indexOf('\n}', start);
  return css.slice(start, end);
};

describe('Guid composer entry strip polish', () => {
  test('uses a dedicated strip and keeps the skill count inline', () => {
    const source = readSource(new URL('./ComposerEntryStrip.tsx', import.meta.url));

    expect(source.includes('styles.entryStrip')).toBe(true);
    expect(source.includes('styles.entryButton')).toBe(true);
    expect(source.includes('styles.entryCountBadge')).toBe(true);
    expect(source.includes('absolute -top')).toBe(false);
    expect(source.includes('-right-')).toBe(false);
  });

  test('adds spacing between entry controls and the textarea', () => {
    const css = readSource(new URL('../index.module.css', import.meta.url));

    expect(css.includes('.entryStrip')).toBe(true);
    expect(css.includes('margin-bottom: 8px')).toBe(true);
    expect(css.includes('.entryCountBadge')).toBe(true);
  });

  test('keeps the Skills count badge readable when a theme uses a light primary', () => {
    const css = readSource(new URL('../index.module.css', import.meta.url));
    const badge = classBlock(css, 'entryCountBadge');

    expect(badge.includes('background: var(--control-selected-bg)')).toBe(true);
    expect(badge.includes('color: var(--control-selected-fg)')).toBe(true);
    expect(badge.includes('color: #fff')).toBe(false);
  });

  test('does not advertise an unimplemented quick-switch shortcut', () => {
    const source = readSource(new URL('./ComposerEntryStrip.tsx', import.meta.url));
    const css = readSource(new URL('../index.module.css', import.meta.url));

    expect(source.includes('⌘K')).toBe(false);
    expect(source.includes('quickSwitch')).toBe(false);
    expect(css.includes('.entryQuickHint')).toBe(false);
  });

  test('keeps the entry row transparent instead of drawing a full-width background bar', () => {
    const css = readSource(new URL('../index.module.css', import.meta.url));
    const strip = classBlock(css, 'entryStrip');

    expect(strip.includes('background: transparent')).toBe(true);
    expect(strip.includes('background: color-mix')).toBe(false);
    expect(strip.includes('border-radius: 16px')).toBe(false);
  });

  test('exposes a discoverable current-skills summary instead of only a count badge', () => {
    const source = readSource(new URL('./ComposerEntryStrip.tsx', import.meta.url));
    const css = readSource(new URL('../index.module.css', import.meta.url));

    expect(source.includes('activeSkills?: GuidActiveSkill[]')).toBe(true);
    expect(source.includes('Trigger')).toBe(true);
    expect(source.includes('guid.entry.skillsActive')).toBe(true);
    expect(source.includes('guid.skillsPopover.title')).toBe(true);
    expect(source.includes('onAdjustSkills')).toBe(true);
    expect(source.includes('onClick={onAdjustSkills}')).toBe(true);
    expect(source.includes('onInsertSkill')).toBe(false);
    expect(source.includes('onManageSkills')).toBe(false);
    expect(css.includes('.entrySkillPopover')).toBe(true);
    expect(css.includes('.entrySkillCountTrigger')).toBe(true);
    expect(css.includes('.entrySkillCompactRow')).toBe(true);
    expect(css.includes('.entrySkillActions')).toBe(false);
    expect(css.includes('.entrySkillFooter')).toBe(false);
  });

  test('renders the shared collaboration policy before the preset picker', () => {
    const source = readSource(new URL('./ComposerEntryStrip.tsx', import.meta.url));

    expect(source.includes('collaborationPolicyNode?: React.ReactNode')).toBe(true);
    const defaultState = source.slice(source.indexOf('// --- Default state ---'));
    const policyPos = defaultState.indexOf('{collaborationPolicyNode}');
    const presetPickerPos = defaultState.indexOf('onClick={onChoosePreset}');
    expect(policyPos).toBeGreaterThan(-1);
    expect(presetPickerPos).toBeGreaterThan(-1);
    expect(policyPos).toBeLessThan(presetPickerPos);
  });
});
