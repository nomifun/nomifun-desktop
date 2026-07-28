/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { describe, expect, test } from 'bun:test';
import { readFileSync } from 'node:fs';

const source = readFileSync(new URL('./SkillsTab.tsx', import.meta.url), 'utf8');

describe('desktop companion Skill configuration view', () => {
  test('keeps global assignment separate from learned specialties', () => {
    expect(source.includes("t('nomi.skills.configuredTitle'")).toBe(true);
    expect(source.includes("t('nomi.skills.learnedTitle'")).toBe(true);
    expect(source.includes('listAvailableSkills.invoke()')).toBe(true);
    expect(source.includes('listBuiltinAutoSkills.invoke()')).toBe(true);
    expect(source.includes("t('nomi.skills.configMissing'")).toBe(true);
    expect(source.includes('toggleCompanionSkill(')).toBe(true);
  });
});
