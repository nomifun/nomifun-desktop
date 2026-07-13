/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { readFileSync } from 'node:fs';
import { describe, expect, test } from 'bun:test';

describe('SkillCard interaction ownership', () => {
  test('opens details from the card and reserves tag editing for its explicit action', () => {
    const source = readFileSync(new URL('./SkillCard.tsx', import.meta.url), 'utf8');
    const cardClick = source.indexOf('onClick={() => onOpenDetails(skill)}');
    const footerStop = source.indexOf('onClick={(e) => e.stopPropagation()}');
    const tagClick = source.indexOf('onClick={() => onEditTags(skill)}');

    expect(cardClick).toBeGreaterThanOrEqual(0);
    expect(footerStop).toBeGreaterThan(cardClick);
    expect(tagClick).toBeGreaterThan(footerStop);
    expect(source.includes('e.stopPropagation();\n              onEditTags(skill);')).toBe(true);
  });
});
