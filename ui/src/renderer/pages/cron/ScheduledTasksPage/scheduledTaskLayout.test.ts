/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { describe, expect, test } from 'bun:test';
import { getScheduledTaskLayout } from './scheduledTaskLayout';

describe('getScheduledTaskLayout', () => {
  test('keeps cards on mobile', () => {
    expect(getScheduledTaskLayout(true)).toBe('card');
  });

  test('uses horizontal rows on desktop', () => {
    expect(getScheduledTaskLayout(false)).toBe('row');
  });
});
