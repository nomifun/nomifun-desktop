/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { describe, expect, test } from 'bun:test';
import { readFileSync } from 'node:fs';

const readSource = (url: URL) => readFileSync(url, 'utf8');

describe('skill localization integration', () => {
  test('the session Skill picker displays localized metadata from the available Skill catalog', () => {
    const picker = readSource(new URL('../../../components/chat/SessionCapabilityPicker/index.tsx', import.meta.url));
    const catalog = readSource(new URL('../../../components/chat/SessionCapabilityPicker/useSessionCapabilityCatalog.ts', import.meta.url));
    expect(catalog.includes('ipcBridge.fs.listAvailableSkills.invoke()')).toBe(true);
    expect(picker.includes('catalog.skills.map((skill)')).toBe(true);
    expect(picker.includes('resolveSkillDisplay(skill, i18n.language)')).toBe(true);
    expect(picker.includes('display.name')).toBe(true);
    expect(picker.includes('display.description')).toBe(true);
  });
});
