/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { readFileSync } from 'node:fs';
import { describe, expect, test } from 'bun:test';

const source = readFileSync(new URL('./useAutoScroll.ts', import.meta.url), 'utf8');

describe('useAutoScroll user layout changes', () => {
  test('does not auto-follow resize events caused by a recent pointer interaction inside the list', () => {
    expect(source.includes('USER_LAYOUT_CHANGE_GUARD_MS')).toBe(true);
    expect(source.includes('resizeAutoFollowBlockedUntilRef')).toBe(true);
    expect(source.includes('if (Date.now() < resizeAutoFollowBlockedUntilRef.current) return;')).toBe(true);
    expect(source.includes('resizeAutoFollowBlockedUntilRef.current = Date.now() + USER_LAYOUT_CHANGE_GUARD_MS')).toBe(
      true
    );
  });
});
