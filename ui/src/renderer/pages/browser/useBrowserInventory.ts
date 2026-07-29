/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { useCallback, useEffect, useRef, useState } from 'react';
import { ipcBridge } from '@/common';
import {
  isBackendHttpError,
  isWsConnected,
  wsLastActivityAt,
} from '@/common/adapter/httpBridge';
import type {
  IBrowserInventoryChangedEvent,
  IBrowserLane,
  IBrowserOverview,
} from '@/common/browser/browserTypes';

const errorMessage = (error: unknown): string =>
  error instanceof Error ? error.message : String(error);

export const BROWSER_RETRY_BASE_DELAY_MS = 500;
export const BROWSER_RETRY_MAX_DELAY_MS = 30_000;

export const browserRetryDelayMs = (consecutiveFailures: number): number => {
  const exponent = Math.min(Math.max(0, consecutiveFailures - 1), 16);
  return Math.min(
    BROWSER_RETRY_MAX_DELAY_MS,
    BROWSER_RETRY_BASE_DELAY_MS * 2 ** exponent
  );
};

export const isBrowserCapabilityUnavailableError = (error: unknown): boolean =>
  isBackendHttpError(error) &&
  (error.status === 404 ||
    error.status === 501 ||
    error.code.toLowerCase() === 'browser_not_supported' ||
    error.code.toLowerCase() === 'browser_disabled');

export const isBrowserCapabilityUnavailable = (
  overview: IBrowserOverview | null,
  unavailable = false
): boolean =>
  unavailable || overview?.supported === false || overview?.enabled === false;

export type BrowserInventoryRealtimeRefreshReason =
  | 'connected'
  | 'event'
  | 'sequence-gap'
  | 'reconnected'
  | 'resync';

export interface BrowserInventoryRealtimeHandler {
  browserEvent: (event: IBrowserInventoryChangedEvent) => void;
  reconnected: () => void;
  connected: () => void;
}

export const createBrowserInventoryRealtimeHandler = (
  refresh: (reason: BrowserInventoryRealtimeRefreshReason) => void
): BrowserInventoryRealtimeHandler => {
  let lastSequence: number | null = null;

  return {
    browserEvent: (event) => {
      const rawEvent = event as IBrowserInventoryChangedEvent & {
        resync_required?: boolean;
        requires_resync?: boolean;
      };
      const sequence =
        typeof event.sequence === 'number' &&
        Number.isSafeInteger(event.sequence) &&
        event.sequence >= 0
          ? event.sequence
          : null;
      const hasSequenceGap =
        sequence !== null &&
        lastSequence !== null &&
        sequence !== lastSequence &&
        sequence !== lastSequence + 1;
      const changeKind = event.change_kind?.trim().toLowerCase();
      const requiresResync =
        rawEvent.resync_required === true ||
        rawEvent.requires_resync === true ||
        changeKind === 'resync_required' ||
        changeKind === 'refresh_required' ||
        changeKind === 'realtime_lagged' ||
        changeKind === 'lagged';

      if (sequence !== null && sequence !== lastSequence) {
        lastSequence = sequence;
        refresh(
          hasSequenceGap ? 'sequence-gap' : requiresResync ? 'resync' : 'event'
        );
        return;
      }
      if (requiresResync) {
        refresh('resync');
        return;
      }
      // The backend forwards lifecycle-kind payloads on both the inventory
      // and lifecycle channels. A repeated sequence is that duplicate
      // delivery, not a new state change: refreshing again is pure waste.
      if (sequence !== null && sequence === lastSequence) return;
      refresh('event');
    },
    reconnected: () => {
      lastSequence = null;
      refresh('reconnected');
    },
    // There is no replay buffer for the shared socket. The first inventory
    // snapshot is therefore deliberately requested after listeners are
    // installed, so an event emitted during initial WebSocket establishment
    // cannot land in a snapshot/subscribe gap.
    connected: () => {
      lastSequence = null;
      refresh('connected');
    },
  };
};

interface BrowserInventoryRealtimeSubscriptionOptions {
  refresh: (reason: BrowserInventoryRealtimeRefreshReason) => void;
  subscribeInventory: (
    listener: (event: IBrowserInventoryChangedEvent) => void
  ) => () => void;
  subscribeLifecycle: (
    listener: (event: IBrowserInventoryChangedEvent) => void
  ) => () => void;
  subscribeReconnected: (listener: () => void) => () => void;
}

/**
 * Ordinary `event` refreshes are coalesced behind a leading-edge refresh so a
 * burst (per-tab CDP churn, telemetry samples, dual-channel deliveries) costs
 * one leading and at most one trailing snapshot instead of one per event.
 * Connect/reconnect/resync/sequence-gap refreshes stay immediate.
 */
export const BROWSER_INVENTORY_EVENT_COALESCE_MS = 1_000;

/**
 * Installs every realtime listener before issuing the initial full snapshot.
 * The shared socket has no replay buffer, so callers must never reverse this
 * order. Its local reconnect signal and event sequence gaps both force the
 * same authoritative overview+lanes refresh.
 */
export const subscribeBrowserInventoryRealtime = ({
  refresh,
  subscribeInventory,
  subscribeLifecycle,
  subscribeReconnected,
}: BrowserInventoryRealtimeSubscriptionOptions): (() => void) => {
  let coalesceTimer: ReturnType<typeof setTimeout> | null = null;
  let trailingQueued = false;

  const clearCoalesce = () => {
    if (coalesceTimer !== null) {
      clearTimeout(coalesceTimer);
      coalesceTimer = null;
    }
    trailingQueued = false;
  };

  const coalescedRefresh = (reason: BrowserInventoryRealtimeRefreshReason) => {
    if (reason !== 'event') {
      // Urgent reasons refresh immediately and supersede any queued trailing
      // refresh — the authoritative snapshot they trigger is already newer.
      clearCoalesce();
      refresh(reason);
      return;
    }
    if (coalesceTimer !== null) {
      trailingQueued = true;
      return;
    }
    refresh(reason);
    coalesceTimer = setTimeout(() => {
      coalesceTimer = null;
      if (trailingQueued) {
        trailingQueued = false;
        refresh('event');
      }
    }, BROWSER_INVENTORY_EVENT_COALESCE_MS);
  };

  const realtime = createBrowserInventoryRealtimeHandler(coalescedRefresh);
  const stopInventory = subscribeInventory(realtime.browserEvent);
  const stopLifecycle = subscribeLifecycle(realtime.browserEvent);
  const stopReconnected = subscribeReconnected(realtime.reconnected);

  // This is the first-open reconciliation for the shared emitter: listener
  // registration above initiates/joins the socket before the snapshot starts.
  realtime.connected();

  return () => {
    clearCoalesce();
    stopInventory();
    stopLifecycle();
    stopReconnected();
  };
};

export const BROWSER_INVENTORY_POLL_INTERVAL_MS = 30_000;

/**
 * The realtime backend emits an application-level `ping` at least every 30s
 * (nomifun-realtime HEARTBEAT_INTERVAL), so a working socket is never silent
 * for long. A socket that has delivered nothing for three heartbeat periods
 * is treated as wedged — a half-open connection after sleep/resume keeps
 * `readyState === OPEN` while every frame is silently lost — and the fallback
 * poll takes over until frames flow again.
 */
export const BROWSER_REALTIME_LIVENESS_TIMEOUT_MS = 90_000;

interface BrowserInventoryFallbackPollOptions {
  poll: () => void;
  isSocketConnected: () => boolean;
  /** Last inbound realtime frame (server heartbeats included), or null. */
  lastRealtimeActivityAt: () => number | null;
  intervalMs?: number;
  livenessTimeoutMs?: number;
  now?: () => number;
}

/**
 * Snapshot polling is a fallback for a wedged realtime channel (half-open
 * socket after sleep/resume, dead backend forwarder), not a supplement to a
 * healthy one: connect, reconnect, sequence-gap, and resync_required already
 * cover every event-visible failure. Liveness is judged by actually delivered
 * frames, never by nominal socket state — a half-open socket still reports
 * OPEN, which is exactly the failure this poll exists to bound. Worst-case
 * inventory staleness is therefore livenessTimeoutMs plus one poll interval.
 */
export const startBrowserInventoryFallbackPoll = ({
  poll,
  isSocketConnected,
  lastRealtimeActivityAt,
  intervalMs = BROWSER_INVENTORY_POLL_INTERVAL_MS,
  livenessTimeoutMs = BROWSER_REALTIME_LIVENESS_TIMEOUT_MS,
  now = Date.now,
}: BrowserInventoryFallbackPollOptions): (() => void) => {
  const timer = setInterval(() => {
    const lastActivityAt = lastRealtimeActivityAt();
    const realtimeAlive =
      isSocketConnected() &&
      lastActivityAt != null &&
      now() - lastActivityAt <= livenessTimeoutMs;
    if (!realtimeAlive) poll();
  }, intervalMs);
  return () => clearInterval(timer);
};

const unavailableOverview = (): IBrowserOverview => ({
  supported: false,
  enabled: false,
  running_lanes: 0,
  queued_lanes: 0,
  total_lanes: 0,
});

export interface BrowserInventoryState {
  lanes: IBrowserLane[];
  overview: IBrowserOverview | null;
  loading: boolean;
  refreshing: boolean;
  error: string | null;
  refresh: () => Promise<void>;
}

export interface BrowserInventoryRecoveryController {
  start: () => Promise<void>;
  refresh: () => Promise<void>;
  realtime: () => Promise<void>;
  poll: () => Promise<void>;
  dispose: () => void;
}

interface BrowserInventoryRecoveryOptions {
  invokeOverview: () => Promise<IBrowserOverview>;
  invokeLanes: () => Promise<IBrowserLane[]>;
  onState: (state: BrowserInventoryState) => void;
}

const initialInventoryState = (): BrowserInventoryState => ({
  lanes: [],
  overview: null,
  loading: true,
  refreshing: false,
  error: null,
  refresh: async () => undefined,
});

export const createBrowserInventoryRecoveryController = ({
  invokeOverview,
  invokeLanes,
  onState,
}: BrowserInventoryRecoveryOptions): BrowserInventoryRecoveryController => {
  let state = initialInventoryState();
  let disposed = false;
  let sequence = 0;
  let hasLoaded = false;
  let consecutiveFailures = 0;
  let retryTimer: ReturnType<typeof setTimeout> | null = null;

  const clearRetry = () => {
    if (retryTimer !== null) {
      clearTimeout(retryTimer);
      retryTimer = null;
    }
  };

  const emit = () => {
    if (!disposed) onState({ ...state, refresh });
  };

  const scheduleRetry = () => {
    clearRetry();
    const delay = browserRetryDelayMs(++consecutiveFailures);
    retryTimer = setTimeout(() => {
      retryTimer = null;
      void load(false);
    }, delay);
  };

  const load = async (resetBackoff: boolean): Promise<void> => {
    if (disposed) return;
    if (resetBackoff) {
      consecutiveFailures = 0;
      clearRetry();
    }

    const currentSequence = ++sequence;
    state = {
      ...state,
      loading: !hasLoaded,
      refreshing: hasLoaded,
    };
    emit();

    const [overviewResult, lanesResult] = await Promise.allSettled([
      invokeOverview(),
      invokeLanes(),
    ]);
    if (disposed || currentSequence !== sequence) return;

    const overviewUnavailable =
      overviewResult.status === 'rejected' &&
      isBrowserCapabilityUnavailableError(overviewResult.reason);
    if (overviewResult.status === 'fulfilled') {
      state = { ...state, overview: overviewResult.value };
    } else if (overviewUnavailable) {
      state = { ...state, overview: unavailableOverview() };
    }
    if (lanesResult.status === 'fulfilled') {
      state = { ...state, lanes: lanesResult.value };
    }

    const capabilityUnavailable =
      overviewUnavailable || isBrowserCapabilityUnavailable(state.overview);
    const transientFailures = capabilityUnavailable
      ? []
      : [overviewResult, lanesResult]
          .filter(
            (result): result is PromiseRejectedResult => result.status === 'rejected'
          )
          .map((result) => errorMessage(result.reason));

    hasLoaded = true;
    state = {
      ...state,
      loading: false,
      refreshing: false,
      error: transientFailures.length > 0 ? transientFailures.join('; ') : null,
    };
    emit();

    if (transientFailures.length > 0) {
      scheduleRetry();
    } else {
      consecutiveFailures = 0;
      clearRetry();
    }
  };

  const refresh = () => load(true);
  const poll = () => {
    clearRetry();
    return load(false);
  };

  return {
    start: refresh,
    refresh,
    realtime: refresh,
    poll,
    dispose: () => {
      disposed = true;
      sequence++;
      clearRetry();
    },
  };
};

export const useBrowserInventory = (): BrowserInventoryState => {
  const [state, setState] = useState<BrowserInventoryState>(initialInventoryState);
  const controllerRef = useRef<BrowserInventoryRecoveryController | null>(null);

  const refresh = useCallback(
    () => controllerRef.current?.refresh() ?? Promise.resolve(),
    []
  );

  useEffect(() => {
    const controller = createBrowserInventoryRecoveryController({
      invokeOverview: () => ipcBridge.browserSession.overview.invoke(),
      invokeLanes: () => ipcBridge.browserSession.lanes.invoke(),
      onState: setState,
    });
    controllerRef.current = controller;

    const stopRealtime = subscribeBrowserInventoryRealtime({
      refresh: () => void controller.realtime(),
      subscribeInventory: (listener) =>
        ipcBridge.browserSession.events.inventoryChanged.on(listener),
      subscribeLifecycle: (listener) =>
        ipcBridge.browserSession.events.lifecycleChanged.on(listener),
      subscribeReconnected: (listener) =>
        ipcBridge.conversation.reconnected.on(listener),
    });
    const stopFallbackPoll = startBrowserInventoryFallbackPoll({
      poll: () => void controller.poll(),
      isSocketConnected: isWsConnected,
      lastRealtimeActivityAt: wsLastActivityAt,
    });

    return () => {
      stopFallbackPoll();
      stopRealtime();
      controller.dispose();
      if (controllerRef.current === controller) controllerRef.current = null;
    };
  }, []);

  return { ...state, refresh };
};

export type BrowserOverviewAvailability =
  | 'loading'
  | 'available'
  | 'transient-error'
  | 'unavailable';

export interface BrowserOverviewRecoveryState {
  overview: IBrowserOverview | null;
  availability: BrowserOverviewAvailability;
  error: string | null;
}

export interface BrowserOverviewRecoveryController {
  start: () => Promise<void>;
  retry: () => Promise<void>;
  realtime: () => Promise<void>;
  dispose: () => void;
}

interface BrowserOverviewRecoveryOptions {
  invoke: () => Promise<IBrowserOverview>;
  onState: (state: BrowserOverviewRecoveryState) => void;
}

export const createBrowserOverviewRecoveryController = ({
  invoke,
  onState,
}: BrowserOverviewRecoveryOptions): BrowserOverviewRecoveryController => {
  let state: BrowserOverviewRecoveryState = {
    overview: null,
    availability: 'loading',
    error: null,
  };
  let disposed = false;
  let sequence = 0;
  let consecutiveFailures = 0;
  let retryTimer: ReturnType<typeof setTimeout> | null = null;

  const clearRetry = () => {
    if (retryTimer !== null) {
      clearTimeout(retryTimer);
      retryTimer = null;
    }
  };

  const emit = () => {
    if (!disposed) onState({ ...state });
  };

  const scheduleRetry = () => {
    clearRetry();
    const delay = browserRetryDelayMs(++consecutiveFailures);
    retryTimer = setTimeout(() => {
      retryTimer = null;
      void load(false);
    }, delay);
  };

  const load = async (resetBackoff: boolean): Promise<void> => {
    if (disposed) return;
    if (resetBackoff) {
      consecutiveFailures = 0;
      clearRetry();
    }

    const currentSequence = ++sequence;
    try {
      const overview = await invoke();
      if (disposed || currentSequence !== sequence) return;
      consecutiveFailures = 0;
      clearRetry();
      state = {
        overview,
        availability: isBrowserCapabilityUnavailable(overview)
          ? 'unavailable'
          : 'available',
        error: null,
      };
      emit();
    } catch (error) {
      if (disposed || currentSequence !== sequence) return;
      if (isBrowserCapabilityUnavailableError(error)) {
        consecutiveFailures = 0;
        clearRetry();
        state = {
          overview: unavailableOverview(),
          availability: 'unavailable',
          error: null,
        };
        emit();
        return;
      }

      state = {
        ...state,
        availability: 'transient-error',
        error: errorMessage(error),
      };
      emit();
      scheduleRetry();
    }
  };

  const retry = () => load(true);

  return {
    start: retry,
    retry,
    realtime: retry,
    dispose: () => {
      disposed = true;
      sequence++;
      clearRetry();
    },
  };
};

export const useBrowserOverview = (): {
  overview: IBrowserOverview | null;
  availability: BrowserOverviewAvailability;
  unavailable: boolean;
  transient: boolean;
  loading: boolean;
  error: string | null;
  retry: () => Promise<void>;
} => {
  const [state, setState] = useState<BrowserOverviewRecoveryState>({
    overview: null,
    availability: 'loading',
    error: null,
  });
  const controllerRef = useRef<BrowserOverviewRecoveryController | null>(null);
  const retry = useCallback(
    () => controllerRef.current?.retry() ?? Promise.resolve(),
    []
  );

  useEffect(() => {
    const controller = createBrowserOverviewRecoveryController({
      invoke: () => ipcBridge.browserSession.overview.invoke(),
      onState: setState,
    });
    controllerRef.current = controller;

    const stopRealtime = subscribeBrowserInventoryRealtime({
      refresh: () => void controller.realtime(),
      subscribeInventory: (listener) =>
        ipcBridge.browserSession.events.inventoryChanged.on(listener),
      subscribeLifecycle: (listener) =>
        ipcBridge.browserSession.events.lifecycleChanged.on(listener),
      subscribeReconnected: (listener) =>
        ipcBridge.conversation.reconnected.on(listener),
    });
    return () => {
      stopRealtime();
      controller.dispose();
      if (controllerRef.current === controller) controllerRef.current = null;
    };
  }, []);

  return {
    ...state,
    unavailable: state.availability === 'unavailable',
    transient: state.availability === 'transient-error',
    loading: state.availability === 'loading',
    retry,
  };
};
