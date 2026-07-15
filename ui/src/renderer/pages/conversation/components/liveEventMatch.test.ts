/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { describe, expect, test } from 'bun:test';
import { isLiveEventForTarget } from './liveEventMatch';

describe('isLiveEventForTarget', () => {
  test('matches the same canonical target', () => {
    expect(
      isLiveEventForTarget(
        'conversation',
        'conv_0190f5fe-7c00-7a00-8000-000000000002',
        'conversation',
        'conv_0190f5fe-7c00-7a00-8000-000000000002',
      ),
    ).toBe(true);
  });

  test('does not match another entity or kind', () => {
    expect(
      isLiveEventForTarget(
        'conversation',
        'conv_0190f5fe-7c00-7a00-8000-000000000003',
        'conversation',
        'conv_0190f5fe-7c00-7a00-8000-000000000002',
      ),
    ).toBe(false);
    expect(
      isLiveEventForTarget('terminal', 'term_test_live', 'conversation', 'term_test_live'),
    ).toBe(false);
  });
});
