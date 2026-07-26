/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { useCallback, useEffect, useRef, useState } from 'react';
import { ipcBridge } from '@/common';
import { isBackendHttpError } from '@/common/adapter/httpBridge';
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
      }
      refresh(
        hasSequenceGap ? 'sequence-gap' : requiresResync ? 'resync' : 'event'
      );
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
  const realtime = createBrowserInventoryRealtimeHandler(refresh);
  const stopInventory = subscribeInventory(realtime.browserEvent);
  const stopLifecycle = subscribeLifecycle(realtime.browserEvent);
  const stopReconnected = subscribeReconnected(realtime.reconnected);

  // This is the first-open reconciliation for the shared emitter: listener
  // registration above initiates/joins the socket before the snapshot starts.
  realtime.connected();

  return () => {
    stopInventory();
    stopLifecycle();
    stopReconnected();
  };
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
    const poll = setInterval(() => void controller.poll(), 30_000);

    return () => {
      clearInterval(poll);
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
