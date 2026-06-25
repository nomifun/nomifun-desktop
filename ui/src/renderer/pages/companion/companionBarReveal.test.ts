/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { describe, expect, it } from 'vitest';
import { createCompanionBarRevealController } from './companionBarReveal';

type TimerEntry = {
  fn: () => void;
  ms: number;
  active: boolean;
};

const fakeTimers = () => {
  const timers: TimerEntry[] = [];
  return {
    timers,
    setTimeoutFn: (fn: () => void, ms: number) => {
      const entry: TimerEntry = { fn, ms, active: true };
      timers.push(entry);
      return entry;
    },
    clearTimeoutFn: (entry: TimerEntry) => {
      entry.active = false;
    },
    fire: (entry: TimerEntry) => {
      if (entry.active) entry.fn();
    },
  };
};

describe('createCompanionBarRevealController', () => {
  it('keeps the chatbar interactive across a brief transparent gap', () => {
    const calls: boolean[] = [];
    const timers = fakeTimers();
    const controller = createCompanionBarRevealController({
      hideDelayMs: 280,
      setRevealed: (next) => calls.push(next),
      setTimeoutFn: timers.setTimeoutFn,
      clearTimeoutFn: timers.clearTimeoutFn,
    });

    controller.handleHoverChange(true);
    controller.handleHoverChange(false);
    expect(calls).toEqual([true]);
    expect(timers.timers).toHaveLength(1);
    expect(timers.timers[0].ms).toBe(280);

    controller.handleHoverChange(true);
    timers.fire(timers.timers[0]);

    expect(calls).toEqual([true]);
  });

  it('hides the chatbar once the grace window expires', () => {
    const calls: boolean[] = [];
    const timers = fakeTimers();
    const controller = createCompanionBarRevealController({
      hideDelayMs: 280,
      setRevealed: (next) => calls.push(next),
      setTimeoutFn: timers.setTimeoutFn,
      clearTimeoutFn: timers.clearTimeoutFn,
    });

    controller.handleHoverChange(true);
    controller.handleHoverChange(false);
    timers.fire(timers.timers[0]);

    expect(calls).toEqual([true, false]);
  });
});
