/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { readFileSync } from 'node:fs';
import { describe, expect, test } from 'bun:test';

const readSource = (url: URL) => readFileSync(url, 'utf8');

describe('Skill tag modal pending draft handling', () => {
  test('commits inline tag drafts before saving skill tags', () => {
    const modal = readSource(new URL('./SkillTagModal.tsx', import.meta.url));
    const picker = readSource(new URL('../PresetSettings/PresetTagPicker.tsx', import.meta.url));

    expect(picker.includes('PresetTagPickerHandle')).toBe(true);
    expect(picker.includes('useImperativeHandle')).toBe(true);
    expect(picker.includes('flushPendingTag')).toBe(true);
    expect(picker.includes('onBlur')).toBe(true);
    expect(picker.includes('commitOnBlur')).toBe(true);
    expect(modal.includes('commitOnBlur')).toBe(true);

    const flushIndex = modal.indexOf('flushPendingTag');
    const saveIndex = modal.indexOf('ipcBridge.fs.setSkillTags.invoke');

    expect(flushIndex).toBeGreaterThanOrEqual(0);
    expect(saveIndex).toBeGreaterThanOrEqual(0);
    expect(flushIndex).toBeLessThan(saveIndex);
  });
});
