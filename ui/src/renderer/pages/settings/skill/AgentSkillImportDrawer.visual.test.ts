/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { readFileSync } from 'node:fs';
import { describe, expect, test } from 'bun:test';

const source = () => readFileSync(new URL('./AgentSkillImportDrawer.tsx', import.meta.url), 'utf8');

describe('AgentSkillImportDrawer visual polish', () => {
  test('uses soft inset surfaces instead of hard theme-sensitive borders', () => {
    const drawer = source();

    expect(drawer.includes('border-border-1')).toBe(false);
    expect(drawer.includes('shadow-[inset_0_0_0_1px_rgba(var(--primary-6),0.10)]')).toBe(true);
    expect(drawer.includes('divide-[rgba(var(--primary-6),0.10)]')).toBe(true);
  });
});
