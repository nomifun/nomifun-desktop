/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { describe, expect, test } from 'bun:test';
import { readFileSync } from 'node:fs';

const readSource = (url: URL) => readFileSync(url, 'utf8');

describe('skill localization integration', () => {
  test('Guid skill surfaces retain and render localized metadata', () => {
    const page = readSource(new URL('../../guid/GuidPage.tsx', import.meta.url));
    const drawer = readSource(new URL('../../guid/components/GuidSkillsDrawer.tsx', import.meta.url));
    const popover = readSource(new URL('../../guid/components/ComposerEntryStrip.tsx', import.meta.url));

    expect(page.includes('name_i18n: s.name_i18n')).toBe(true);
    expect(page.includes('description_i18n: s.description_i18n')).toBe(true);
    expect(drawer.includes('resolveSkillDisplay(skill, localeKey)')).toBe(true);
    expect(popover.includes('resolveSkillDisplay(skill, localeKey)')).toBe(true);
  });

  test('the Agent Workbench reads display metadata from the canonical Skill catalog', () => {
    const editor = readSource(new URL('../../agentSettings/AgentPresetEditor.tsx', import.meta.url));
    expect(editor.includes('catalog.skills.map((skill)')).toBe(true);
    expect(editor.includes('skill.display_name')).toBe(true);
    expect(editor.includes('skill.description')).toBe(true);
  });
});
