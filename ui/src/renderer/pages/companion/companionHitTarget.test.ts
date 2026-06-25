/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { describe, expect, it } from 'vitest';
import { isPointOverCompanionHitTarget } from './companionHitTarget';

type FakeStyle = {
  pointerEvents?: string;
  visibility?: string;
  display?: string;
};

const fakeEl = (rect: DOMRectInit, style: FakeStyle = {}) =>
  ({
    getBoundingClientRect: () => ({
      left: rect.x ?? 0,
      top: rect.y ?? 0,
      right: (rect.x ?? 0) + (rect.width ?? 0),
      bottom: (rect.y ?? 0) + (rect.height ?? 0),
      width: rect.width ?? 0,
      height: rect.height ?? 0,
    }),
    __style: style,
  }) as HTMLElement & { __style: FakeStyle };

describe('isPointOverCompanionHitTarget', () => {
  it('skips hidden chatbar candidates that cannot receive pointer events', () => {
    const hiddenChatbar = fakeEl({ x: 20, y: 160, width: 200, height: 30 }, { pointerEvents: 'none' });

    expect(
      isPointOverCompanionHitTarget(80, 170, [hiddenChatbar], {
        tolerancePx: 8,
        getStyle: (el) => (el as typeof hiddenChatbar).__style,
      })
    ).toBe(false);
  });

  it('counts a visible chatbar candidate even before React reveal state catches up', () => {
    const cssHoverVisibleChatbar = fakeEl({ x: 20, y: 160, width: 200, height: 30 }, { pointerEvents: 'auto' });

    expect(
      isPointOverCompanionHitTarget(80, 170, [cssHoverVisibleChatbar], {
        tolerancePx: 8,
        getStyle: (el) => (el as typeof cssHoverVisibleChatbar).__style,
      })
    ).toBe(true);
  });
});
