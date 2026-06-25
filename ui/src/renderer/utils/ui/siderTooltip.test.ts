/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { describe, expect, test } from 'bun:test';

import { getSiderTooltipProps } from './siderTooltip';

describe('getSiderTooltipProps', () => {
  const originalWindow = globalThis.window;

  const mockMatchMedia = (matchesByQuery: Record<string, boolean>) =>
    Object.defineProperty(globalThis, 'window', {
      configurable: true,
      value: {
        matchMedia: (query: string) => ({
          matches: matchesByQuery[query] ?? false,
        }),
      },
    });
  const restoreWindow = () =>
    Object.defineProperty(globalThis, 'window', {
      configurable: true,
      value: originalWindow,
    });

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

  test('does not disable enabled hover tooltips because of pointer media queries', () => {
    try {
      mockMatchMedia({
        '(hover: none)': true,
        '(pointer: coarse)': true,
      });

      expect(getSiderTooltipProps(true).disabled).toBe(false);
    } finally {
      restoreWindow();
    }
  });

  test('disables hover tooltips when the caller turns them off', () => {
    expect(getSiderTooltipProps(false).disabled).toBe(true);
  });
});
