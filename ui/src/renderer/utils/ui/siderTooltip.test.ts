/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { describe, expect, test } from 'bun:test';

import { getSiderTooltipProps } from './siderTooltip';

describe('getSiderTooltipProps', () => {
  test('shows sidebar tooltips immediately on hover', () => {
    const props = getSiderTooltipProps(true) as {
      triggerProps?: {
        mouseEnterDelay?: number;
        mouseLeaveDelay?: number;
      };
    };

    expect(props.triggerProps?.mouseEnterDelay).toBe(0);
    expect(props.triggerProps?.mouseLeaveDelay).toBe(0);
  });

  test('disables hover tooltips when the caller turns them off', () => {
    expect(getSiderTooltipProps(false).disabled).toBe(true);
  });
});
