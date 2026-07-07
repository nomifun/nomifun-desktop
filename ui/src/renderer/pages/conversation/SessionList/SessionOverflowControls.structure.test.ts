/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { describe, expect, test } from 'bun:test';
import { readFileSync } from 'node:fs';

const sessionKindSource = readFileSync(new URL('./SessionKindGroup.tsx', import.meta.url), 'utf8');
const companionGroupSource = readFileSync(new URL('./CompanionSessionGroup.tsx', import.meta.url), 'utf8');

describe('SessionList overflow controls', () => {
  test('uses high-contrast themed text for expandable overflow buttons', () => {
    for (const source of [sessionKindSource, companionGroupSource]) {
      expect(source.includes("t('sessionList.expandDisplay'")).toBe(true);
      expect(source.includes('text-t-secondary transition-colors cursor-pointer')).toBe(true);
      expect(source.includes('text-t-quaternary transition-colors cursor-pointer')).toBe(false);
      expect(source.includes('hover:text-t-primary')).toBe(true);
    }
  });
});
