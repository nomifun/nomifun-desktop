/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { describe, expect, test } from 'bun:test';
import { readFileSync } from 'node:fs';

const drawerSource = readFileSync(new URL('./PresetEditDrawer.tsx', import.meta.url), 'utf8');
const controlCss = readFileSync(new URL('../../../styles/theme-control-contract.css', import.meta.url), 'utf8');
const codexNeutralCss = readFileSync(new URL('../DisplaySettings/presets/codex-neutral.css', import.meta.url), 'utf8');

describe('preset skill checkbox selection treatment', () => {
  test('applies the enhanced selected state to every editable preset skill source', () => {
    expect(drawerSource.match(/preset-skill-selection-checkbox/g)?.length).toBe(6);
    expect(controlCss.includes('.arco-checkbox-checked .arco-checkbox-mask')).toBe(true);
    expect(controlCss.includes('.arco-checkbox-mask-icon')).toBe(true);
  });

  test('applies the enhanced selection treatment to preset scope controls', () => {
    expect(drawerSource.match(/preset-scope-selection-checkbox/g)?.length).toBe(4);
    expect(controlCss.includes('.arco-checkbox-mask {')).toBe(true);
  });

  test('keeps the checkmark white against the classic dark black selection fill', () => {
    expect(codexNeutralCss.includes('--enhanced-checkbox-selected-fg: #ffffff;')).toBe(true);
    expect(controlCss.includes('color: var(--enhanced-checkbox-selected-fg, var(--control-selected-fg, var(--color-white))) !important;')).toBe(true);
  });

  test('deduplicates builtin skills that already live in the auto-injected list', () => {
    expect(drawerSource.includes('const builtinAutoSkillNameSet = new Set')).toBe(true);
    expect(drawerSource.includes("skill.source === 'builtin' && !builtinAutoSkillNameSet.has(skill.name)")).toBe(true);
  });

  test('keeps custom skills out of the auto-injected list controls', () => {
    expect(drawerSource.includes('autoInjectedCustomSkillItems')).toBe(false);
    expect(drawerSource.includes('addCustomSkillToAutoInjected')).toBe(false);
    expect(drawerSource.includes('removeCustomSkillFromAutoInjected')).toBe(false);
    expect(drawerSource.includes("data-testid='btn-add-custom-skill-to-auto'")).toBe(false);
    expect(drawerSource.includes("data-testid='btn-remove-custom-skill-from-auto'")).toBe(false);
  });

  test('renders selected custom skills with the auto badge style while keeping builtin auto skills readonly', () => {
    expect(drawerSource.includes('selectedPresetAutoBadge')).toBe(true);
    expect(drawerSource.includes("settings.skillsHub.sourceAuto")).toBe(true);
    expect(drawerSource.includes('badge={selectedPresetAutoBadge(skill.name)}')).toBe(true);
    expect(drawerSource.includes("data-testid='preset-builtin-auto-skill-readonly'")).toBe(true);
    expect(drawerSource.includes('setDisabledBuiltinSkills(')).toBe(false);
  });
});
