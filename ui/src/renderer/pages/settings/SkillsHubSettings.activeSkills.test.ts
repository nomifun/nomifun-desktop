/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { readFileSync } from 'node:fs';
import { describe, expect, test } from 'bun:test';

const readSource = (url: URL) => readFileSync(url, 'utf8');

describe('SkillsHubSettings active skill focus', () => {
  test('does not add a conversation-only activeSkills focus mode to the management page', () => {
    const source = readSource(new URL('./SkillsHubSettings.tsx', import.meta.url));

    expect(source.includes("searchParams.get('activeSkills')")).toBe(false);
    expect(source.includes('activeSkillNames')).toBe(false);
    expect(source.includes('viewingActiveSkills')).toBe(false);
    expect(source.includes('settings.skillsHub.activeSkillsBanner')).toBe(false);
    expect(source.includes('settings.skillsHub.showAllSkills')).toBe(false);
    expect(source.includes('activeSkillNames.has(skill.name)')).toBe(false);
  });
});
