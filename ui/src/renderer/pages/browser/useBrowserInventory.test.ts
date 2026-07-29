/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import * as bunTest from 'bun:test';
import { describe, expect, test } from 'bun:test';
import { BackendHttpError } from '@/common/adapter/httpBridge';
import type {
  IBrowserInventoryChangedEvent,
  IBrowserOverview,
} from '@/common/browser/browserTypes';
import {
  BROWSER_INVENTORY_EVENT_COALESCE_MS,
  BROWSER_INVENTORY_POLL_INTERVAL_MS,
  BROWSER_REALTIME_LIVENESS_TIMEOUT_MS,
  BROWSER_RETRY_BASE_DELAY_MS,
  BROWSER_RETRY_MAX_DELAY_MS,
  createBrowserInventoryRealtimeHandler,
  createBrowserInventoryRecoveryController,
  createBrowserOverviewRecoveryController,
  startBrowserInventoryFallbackPoll,
  subscribeBrowserInventoryRealtime,
  type BrowserInventoryState,
  type BrowserInventoryRealtimeRefreshReason,
  type BrowserOverviewRecoveryState,
} from './useBrowserInventory';

interface FakeTimerApi {
  useFakeTimers: () => void;
  useRealTimers: () => void;
  clearAllTimers: () => void;
  advanceTimersByTime: (milliseconds: number) => void;
  getTimerCount: () => number;
}

const timers = (bunTest as unknown as { jest: FakeTimerApi }).jest;

const availableOverview = (running = 0): IBrowserOverview => ({
  supported: true,
  enabled: true,
  running_lanes: running,
  queued_lanes: 0,
});

const flushPromises = async () => {
  await Promise.resolve();
  await Promise.resolve();
  await Promise.resolve();
};

const readInventoryState = (
  read: () => BrowserInventoryState | null
): BrowserInventoryState => {
  const state = read();
  if (!state) throw new Error('expected inventory state');
  return state;
};

const readOverviewState = (
  read: () => BrowserOverviewRecoveryState | null
): BrowserOverviewRecoveryState => {
  const state = read();
  if (!state) throw new Error('expected overview state');
  return state;
};

describe('browser inventory recovery', () => {
  test('forces a full refresh on first connect, reconnect, resync, and sequence gaps', () => {
    const reasons: BrowserInventoryRealtimeRefreshReason[] = [];
    const realtime = createBrowserInventoryRealtimeHandler((reason) => {
      reasons.push(reason);
    });

    realtime.connected();
    realtime.browserEvent({ sequence: 10 });
    realtime.browserEvent({ sequence: 11 });
    realtime.browserEvent({ sequence: 14 });
    const resyncEvent: IBrowserInventoryChangedEvent = {
      resync_required: true,
      skipped: 3,
      user_id: null,
      at_ms: 1_700_000_000_000,
    };
    realtime.browserEvent(resyncEvent);
    realtime.reconnected();
    realtime.browserEvent({ sequence: 30 });

    expect(reasons).toEqual([
      'connected',
      'event',
      'event',
      'sequence-gap',
      'resync',
      'reconnected',
      'event',
    ]);
  });

  test('registers all realtime listeners before starting the first snapshot refresh', () => {
    const order: string[] = [];
    let inventory:
      | ((event: { sequence?: number; change_kind?: string }) => void)
      | undefined;
    let lifecycle:
      | ((event: { sequence?: number; change_kind?: string }) => void)
      | undefined;
    let reconnected: (() => void) | undefined;
    const stop = subscribeBrowserInventoryRealtime({
      refresh: (reason) => order.push(`refresh:${reason}`),
      subscribeInventory: (listener) => {
        order.push('subscribe:inventory');
        inventory = listener;
        return () => order.push('stop:inventory');
      },
      subscribeLifecycle: (listener) => {
        order.push('subscribe:lifecycle');
        lifecycle = listener;
        return () => order.push('stop:lifecycle');
      },
      subscribeReconnected: (listener) => {
        order.push('subscribe:reconnected');
        reconnected = listener;
        return () => order.push('stop:reconnected');
      },
    });

    expect(order).toEqual([
      'subscribe:inventory',
      'subscribe:lifecycle',
      'subscribe:reconnected',
      'refresh:connected',
    ]);

    inventory?.({ sequence: 4 });
    lifecycle?.({ sequence: 7 });
    reconnected?.();
    expect(order.slice(-3)).toEqual([
      'refresh:event',
      'refresh:sequence-gap',
      'refresh:reconnected',
    ]);

    stop();
    expect(order.slice(-3)).toEqual([
      'stop:inventory',
      'stop:lifecycle',
      'stop:reconnected',
    ]);
  });

  test('suppresses the duplicate dual-channel delivery of the same sequence', () => {
    const reasons: BrowserInventoryRealtimeRefreshReason[] = [];
    const realtime = createBrowserInventoryRealtimeHandler((reason) => {
      reasons.push(reason);
    });

    realtime.connected();
    // The backend forwards lifecycle-kind payloads on BOTH
    // browser.inventory.changed and browser.lifecycle.changed; the second
    // delivery carries the identical sequence and must not refresh again.
    realtime.browserEvent({ sequence: 10 });
    realtime.browserEvent({ sequence: 10 });
    realtime.browserEvent({ sequence: 11 });
    realtime.browserEvent({ sequence: 11 });
    // Events without a sequence can never be proven duplicates: refresh.
    realtime.browserEvent({});
    realtime.browserEvent({});
    // An explicit resync marker always wins, even on a repeated sequence.
    realtime.browserEvent({ sequence: 11, resync_required: true });

    expect(reasons).toEqual([
      'connected',
      'event',
      'event',
      'event',
      'event',
      'resync',
    ]);
  });

  test('coalesces realtime event bursts into a leading and one trailing refresh', () => {
    timers.useFakeTimers();
    try {
      const order: string[] = [];
      let inventory:
        | ((event: { sequence?: number; change_kind?: string }) => void)
        | undefined;
      const stop = subscribeBrowserInventoryRealtime({
        refresh: (reason) => order.push(`refresh:${reason}`),
        subscribeInventory: (listener) => {
          inventory = listener;
          return () => undefined;
        },
        subscribeLifecycle: () => () => undefined,
        subscribeReconnected: () => () => undefined,
      });
      expect(order).toEqual(['refresh:connected']);

      inventory?.({ sequence: 1 });
      inventory?.({ sequence: 2 });
      inventory?.({ sequence: 3 });
      inventory?.({ sequence: 4 });
      // Leading edge fires immediately; the burst coalesces behind it.
      expect(order).toEqual(['refresh:connected', 'refresh:event']);

      timers.advanceTimersByTime(BROWSER_INVENTORY_EVENT_COALESCE_MS);
      expect(order).toEqual([
        'refresh:connected',
        'refresh:event',
        'refresh:event',
      ]);

      // Quiet period: a single event outside a burst refreshes immediately.
      timers.advanceTimersByTime(BROWSER_INVENTORY_EVENT_COALESCE_MS);
      inventory?.({ sequence: 5 });
      expect(order.at(-1)).toBe('refresh:event');
      expect(order).toHaveLength(4);

      // Urgent reasons bypass and clear any pending coalesced refresh.
      inventory?.({ sequence: 6 });
      inventory?.({ sequence: 9 });
      expect(order.at(-1)).toBe('refresh:sequence-gap');
      const lengthAfterGap = order.length;
      timers.advanceTimersByTime(BROWSER_INVENTORY_EVENT_COALESCE_MS * 4);
      expect(order).toHaveLength(lengthAfterGap);

      stop();
    } finally {
      timers.clearAllTimers();
      timers.useRealTimers();
    }
  });

  test('polls the snapshot fallback while the socket is down and stays quiet while realtime is alive', () => {
    timers.useFakeTimers();
    try {
      let connected = true;
      let lastActivityAt: number | null = null;
      let polls = 0;
      const stop = startBrowserInventoryFallbackPoll({
        poll: () => {
          polls += 1;
        },
        isSocketConnected: () => connected,
        lastRealtimeActivityAt: () => lastActivityAt,
      });

      // Fail safe: a nominally OPEN socket with no recorded activity has
      // never proven the realtime channel works, so the fallback polls.
      timers.advanceTimersByTime(BROWSER_INVENTORY_POLL_INTERVAL_MS);
      expect(polls).toBe(1);

      // Healthy: heartbeats keep arriving between ticks; never poll.
      for (let index = 0; index < 3; index += 1) {
        lastActivityAt = Date.now();
        timers.advanceTimersByTime(BROWSER_INVENTORY_POLL_INTERVAL_MS);
      }
      expect(polls).toBe(1);

      // A visibly closed socket polls immediately, regardless of how recent
      // the last delivered frame was.
      connected = false;
      lastActivityAt = Date.now();
      timers.advanceTimersByTime(BROWSER_INVENTORY_POLL_INTERVAL_MS * 2);
      expect(polls).toBe(3);

      connected = true;
      lastActivityAt = Date.now();
      timers.advanceTimersByTime(BROWSER_INVENTORY_POLL_INTERVAL_MS);
      expect(polls).toBe(3);

      stop();
      connected = false;
      timers.advanceTimersByTime(BROWSER_INVENTORY_POLL_INTERVAL_MS * 2);
      expect(polls).toBe(3);
    } finally {
      timers.clearAllTimers();
      timers.useRealTimers();
    }
  });

  test('restores bounded polling when the socket is wedged half-open (OPEN but silent)', () => {
    timers.useFakeTimers();
    try {
      // The socket never reports a close: a half-open connection after
      // sleep/resume keeps readyState OPEN while nothing arrives anymore.
      const connected = true;
      let lastActivityAt: number | null = null;
      let polls = 0;
      const stop = startBrowserInventoryFallbackPoll({
        poll: () => {
          polls += 1;
        },
        isSocketConnected: () => connected,
        lastRealtimeActivityAt: () => lastActivityAt,
      });

      // Healthy phase: server heartbeats land between poll ticks.
      for (let index = 0; index < 3; index += 1) {
        lastActivityAt = Date.now();
        timers.advanceTimersByTime(BROWSER_INVENTORY_POLL_INTERVAL_MS);
      }
      expect(polls).toBe(0);

      // Wedge: frames stop arriving after one final heartbeat. Silence within
      // the liveness window is still tolerated (a heartbeat may simply be in
      // flight).
      lastActivityAt = Date.now();
      timers.advanceTimersByTime(BROWSER_REALTIME_LIVENESS_TIMEOUT_MS);
      expect(polls).toBe(0);

      // Past the liveness window the channel is treated as dead even though
      // the socket still claims OPEN: staleness stays bounded by the poll.
      timers.advanceTimersByTime(BROWSER_INVENTORY_POLL_INTERVAL_MS * 2);
      expect(polls).toBe(2);

      // Frames resume (e.g. the OS finally surfaced the failure and the
      // bridge reconnected): polling stops again.
      lastActivityAt = Date.now();
      timers.advanceTimersByTime(BROWSER_INVENTORY_POLL_INTERVAL_MS * 2);
      expect(polls).toBe(2);

      stop();
    } finally {
      timers.clearAllTimers();
      timers.useRealTimers();
    }
  });

  test('recovers a direct /browser load after an initial transient failure', async () => {
    timers.useFakeTimers();
    try {
      let overviewCalls = 0;
      let laneCalls = 0;
      let latest: BrowserInventoryState | null = null;
      const controller = createBrowserInventoryRecoveryController({
        invokeOverview: async () => {
          overviewCalls++;
          if (overviewCalls === 1) throw new Error('backend is still starting');
          return availableOverview(1);
        },
        invokeLanes: async () => {
          laneCalls++;
          if (laneCalls === 1) throw new Error('inventory is still starting');
          return [
            {
              lane_id: 'lane-recovered',
              lifecycle_state: 'running',
              tabs: [],
            },
          ];
        },
        onState: (state) => {
          latest = state;
        },
      });

      await controller.start();
      expect(overviewCalls).toBe(1);
      expect(laneCalls).toBe(1);
      let state = readInventoryState(() => latest);
      expect(state.loading).toBe(false);
      expect(state.error?.includes('backend is still starting')).toBe(true);
      expect(timers.getTimerCount()).toBe(1);

      timers.advanceTimersByTime(BROWSER_RETRY_BASE_DELAY_MS - 1);
      await flushPromises();
      expect(overviewCalls).toBe(1);

      timers.advanceTimersByTime(1);
      await flushPromises();
      expect(overviewCalls).toBe(2);
      expect(laneCalls).toBe(2);
      state = readInventoryState(() => latest);
      expect(state.overview?.running_lanes).toBe(1);
      expect(state.lanes.map((lane) => lane.lane_id)).toEqual(['lane-recovered']);
      expect(state.error).toBeNull();
      expect(timers.getTimerCount()).toBe(0);
      controller.dispose();
    } finally {
      timers.clearAllTimers();
      timers.useRealTimers();
    }
  });

  test('treats a definitive unsupported response as gated rather than transient', async () => {
    timers.useFakeTimers();
    try {
      let latest: BrowserInventoryState | null = null;
      const unsupported = new BackendHttpError({
        method: 'GET',
        path: '/api/browser/overview',
        status: 501,
        body: {
          code: 'browser_not_supported',
          message: 'Browser management is not available in this build.',
        },
      });
      const controller = createBrowserInventoryRecoveryController({
        invokeOverview: async () => {
          throw unsupported;
        },
        invokeLanes: async () => {
          throw unsupported;
        },
        onState: (state) => {
          latest = state;
        },
      });

      await controller.start();
      const state = readInventoryState(() => latest);
      expect(state.overview?.supported).toBe(false);
      expect(state.overview?.enabled).toBe(false);
      expect(state.error).toBeNull();
      expect(timers.getTimerCount()).toBe(0);
      controller.dispose();
    } finally {
      timers.clearAllTimers();
      timers.useRealTimers();
    }
  });
});

describe('browser overview entry recovery', () => {
  test('keeps a transient failure retryable and lets realtime recover immediately', async () => {
    timers.useFakeTimers();
    try {
      let calls = 0;
      let latest: BrowserOverviewRecoveryState | null = null;
      const controller = createBrowserOverviewRecoveryController({
        invoke: async () => {
          calls++;
          if (calls === 1) throw new Error('temporary overview failure');
          return availableOverview(2);
        },
        onState: (state) => {
          latest = state;
        },
      });

      await controller.start();
      let state = readOverviewState(() => latest);
      expect(state.availability).toBe('transient-error');
      expect(state.error).toBe('temporary overview failure');
      expect(timers.getTimerCount()).toBe(1);

      await controller.realtime();
      expect(calls).toBe(2);
      state = readOverviewState(() => latest);
      expect(state.availability).toBe('available');
      expect(state.overview?.running_lanes).toBe(2);
      expect(timers.getTimerCount()).toBe(0);

      timers.advanceTimersByTime(BROWSER_RETRY_BASE_DELAY_MS);
      await flushPromises();
      expect(calls).toBe(2);
      controller.dispose();
    } finally {
      timers.clearAllTimers();
      timers.useRealTimers();
    }
  });

  test('uses capped exponential retry delays for repeated transient failures', async () => {
    timers.useFakeTimers();
    try {
      const callTimes: number[] = [];
      const controller = createBrowserOverviewRecoveryController({
        invoke: async () => {
          callTimes.push(Date.now());
          throw new Error('still unavailable');
        },
        onState: () => undefined,
      });

      await controller.start();
      const startedAt = callTimes[0] ?? Date.now();
      const delays = [500, 1_000, 2_000, 4_000, 8_000, 16_000, 30_000, 30_000];
      for (const delay of delays) {
        timers.advanceTimersByTime(delay);
        await flushPromises();
      }

      expect(callTimes.map((time) => time - startedAt)).toEqual([
        0,
        500,
        1_500,
        3_500,
        7_500,
        15_500,
        31_500,
        61_500,
        91_500,
      ]);
      expect(delays.at(-1)).toBe(BROWSER_RETRY_MAX_DELAY_MS);
      expect(timers.getTimerCount()).toBe(1);
      controller.dispose();
    } finally {
      timers.clearAllTimers();
      timers.useRealTimers();
    }
  });

  test('stops retrying when overview explicitly disables the capability', async () => {
    timers.useFakeTimers();
    try {
      let latest: BrowserOverviewRecoveryState | null = null;
      const controller = createBrowserOverviewRecoveryController({
        invoke: async () => ({
          ...availableOverview(),
          enabled: false,
        }),
        onState: (state) => {
          latest = state;
        },
      });

      await controller.start();
      const state = readOverviewState(() => latest);
      expect(state.availability).toBe('unavailable');
      expect(state.overview?.enabled).toBe(false);
      expect(timers.getTimerCount()).toBe(0);
      controller.dispose();
    } finally {
      timers.clearAllTimers();
      timers.useRealTimers();
    }
  });
});
