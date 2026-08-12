/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { AUTH_EXPIRED_EVENT, isBackendHttpError } from '@/common/adapter/httpBridge';
import { configService } from '@/common/config/configService';
import {
  BROWSER_DISPLAY_MODE_POLICY_VERSION,
  browserResourcePolicyApi,
  buildBrowserResourcePolicyAdvancedSaveRequest,
  buildBrowserResourcePolicyPresetRequest,
  isBrowserResourcePolicyUnavailableError,
  migrateBrowserDisplayMode,
  type BrowserResourcePolicy,
  type BrowserResourcePolicyAdvanced,
  type BrowserResourcePolicyPreset,
} from '@/common/browser/browserSettings';
import { createBrowserDisplayModeController } from '@/common/browser/browserDisplayModeController';
import {
  resolveBrowserOverviewCapabilities,
  type BrowserDisplayMode,
  type IBrowserOverview,
  type IBrowserOverviewCapabilities,
} from '@/common/browser/browserTypes';
import { ipcBridge } from '@/common';
import NomiScrollArea from '@/renderer/components/base/NomiScrollArea';
import { Alert, Button, Collapse, InputNumber, Message, Modal, Radio, Switch } from '@arco-design/web-react';
import React, { useCallback, useEffect, useRef, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { useNavigate } from 'react-router-dom';
import BasePreferenceRow from './SystemModalContent/PreferenceRow';

const RadioGroup = Radio.Group;
const PreferenceRow: React.FC<
  Omit<React.ComponentProps<typeof BasePreferenceRow>, 'compact'>
> = (props) => (
  <BasePreferenceRow {...props} compact />
);

type BrowserSource = 'managed' | 'system';
type BrowserDisplayModeStatus = 'loading' | 'ready' | 'unavailable' | 'error';
type ResourcePolicyStatus = 'loading' | 'ready' | 'unavailable' | 'error';
type BrowserSettingsError = {
  message?: string;
  fallbackKey: BrowserSettingsErrorKey;
};

const cacheAuthoritativeBrowserDisplayMode = (displayMode: BrowserDisplayMode): void => {
  configService.setLocal('agent.browserUse.displayMode', displayMode);
  configService.setLocal(
    'agent.browserUse.displayModeVersion',
    BROWSER_DISPLAY_MODE_POLICY_VERSION
  );
};
type BrowserSettingsErrorKey =
  | 'common.unknownError'
  | 'settings.browserDisplayModeSaveFailed'
  | 'settings.browserResourcePolicyLoadFailed'
  | 'settings.browserResourcePolicySaveFailed';

export const BROWSER_SETTINGS_OVERVIEW_RETRY_DELAYS_MS = [250, 1_000] as const;

type BrowserSettingsCapabilityLoaderOptions = {
  invoke: () => Promise<IBrowserOverview>;
  onCapabilities: (capabilities: IBrowserOverviewCapabilities) => void;
  retryDelaysMs?: readonly number[];
  schedule?: (callback: () => void | Promise<void>, delayMs: number) => unknown;
  cancelScheduled?: (handle: unknown) => void;
};

export type BrowserSettingsCapabilityLoader = {
  start: () => Promise<void>;
  reload: () => Promise<void>;
  dispose: () => void;
};

const isBrowserOverviewUnavailableError = (error: unknown): boolean =>
  isBackendHttpError(error) &&
  (error.status === 404 ||
    error.status === 501 ||
    error.code.toLowerCase() === 'browser_not_supported' ||
    error.code.toLowerCase() === 'browser_disabled');

const isBrowserDisplayModeUnavailableError = (error: unknown): boolean =>
  isBackendHttpError(error) && (error.status === 404 || error.status === 501);

/**
 * Loads installation-owner capabilities without turning a transient startup
 * failure into a permanent hidden state. Automatic retries are finite, while
 * explicit reload signals are coalesced behind the request already in flight.
 */
export function createBrowserSettingsCapabilityLoader({
  invoke,
  onCapabilities,
  retryDelaysMs = BROWSER_SETTINGS_OVERVIEW_RETRY_DELAYS_MS,
  schedule = (callback, delayMs) => setTimeout(() => void callback(), delayMs),
  cancelScheduled = (handle) => clearTimeout(handle as ReturnType<typeof setTimeout>),
}: BrowserSettingsCapabilityLoaderOptions): BrowserSettingsCapabilityLoader {
  let disposed = false;
  let inFlight = false;
  let reloadQueued = false;
  let retryIndex = 0;
  let retryHandle: unknown;

  const clearRetry = () => {
    if (retryHandle !== undefined) {
      cancelScheduled(retryHandle);
      retryHandle = undefined;
    }
  };

  const load = async (resetRetryBudget: boolean): Promise<void> => {
    if (disposed) return;
    if (inFlight) {
      // A burst of reconnect/reload signals becomes one follow-up request.
      if (resetRetryBudget) reloadQueued = true;
      return;
    }
    if (resetRetryBudget) {
      retryIndex = 0;
      clearRetry();
    }

    inFlight = true;
    try {
      const overview = await invoke();
      if (disposed) return;
      retryIndex = 0;
      clearRetry();
      onCapabilities(resolveBrowserOverviewCapabilities(overview));
    } catch (error) {
      if (disposed) return;
      if (!isBrowserOverviewUnavailableError(error) && retryIndex < retryDelaysMs.length) {
        const delayMs = retryDelaysMs[retryIndex++];
        clearRetry();
        retryHandle = schedule(() => {
          retryHandle = undefined;
          return load(false);
        }, delayMs);
      }
    } finally {
      inFlight = false;
      if (!disposed && reloadQueued) {
        reloadQueued = false;
        await load(true);
      }
    }
  };

  return {
    start: () => load(true),
    reload: () => load(true),
    dispose: () => {
      disposed = true;
      reloadQueued = false;
      clearRetry();
    },
  };
}

export const BROWSER_LOGIN_PROMOTION_POLL_INTERVAL_MS = 2_000;
export const BROWSER_LOGIN_PROMOTION_POLL_MAX_ATTEMPTS = 90;

export type BrowserLoginPromotionStopReason = 'failed' | 'timeout';

type BrowserLoginPromotionPollOptions = {
  /** Lifecycle of the queued login lane, or null when it no longer exists. */
  probeLifecycle: () => Promise<string | null | undefined>;
  /** Foreground the promoted lane; resolves true when a window is confirmed. */
  foreground: () => Promise<boolean>;
  onOpened: () => void;
  onStopped: (reason: BrowserLoginPromotionStopReason) => void;
  schedule?: (callback: () => void | Promise<void>, delayMs: number) => unknown;
  cancelScheduled?: (handle: unknown) => void;
  maxAttempts?: number;
};

/**
 * A queued login open (active:true, message:'queued') has no window yet, and
 * the backend never foregrounds the lane when capacity later frees up. Watch
 * the lane until it runs, then explicitly foreground it — only that success
 * may be announced as an open window. Terminal lane states and a bounded
 * attempt budget both stop the poll so it can never spin forever.
 */
export function startBrowserLoginPromotionPoll({
  probeLifecycle,
  foreground,
  onOpened,
  onStopped,
  schedule = (callback, delayMs) => setTimeout(() => void callback(), delayMs),
  cancelScheduled = (handle) => clearTimeout(handle as ReturnType<typeof setTimeout>),
  maxAttempts = BROWSER_LOGIN_PROMOTION_POLL_MAX_ATTEMPTS,
}: BrowserLoginPromotionPollOptions): () => void {
  let cancelled = false;
  let attempts = 0;
  let handle: unknown;

  const tick = async (): Promise<void> => {
    if (cancelled) return;
    attempts += 1;
    let lifecycle: string | null | undefined;
    try {
      lifecycle = await probeLifecycle();
    } catch {
      lifecycle = undefined; // transient snapshot failure: keep waiting
    }
    if (cancelled) return;

    if (lifecycle === 'failed' || lifecycle === 'stopping' || lifecycle === null) {
      onStopped('failed');
      return;
    }
    if (lifecycle === 'running') {
      let confirmed = false;
      try {
        confirmed = await foreground();
      } catch {
        confirmed = false;
      }
      if (cancelled) return;
      if (confirmed) onOpened();
      else onStopped('failed');
      return;
    }
    if (attempts >= maxAttempts) {
      onStopped('timeout');
      return;
    }
    handle = schedule(tick, BROWSER_LOGIN_PROMOTION_POLL_INTERVAL_MS);
  };

  handle = schedule(tick, BROWSER_LOGIN_PROMOTION_POLL_INTERVAL_MS);

  return () => {
    cancelled = true;
    if (handle !== undefined) {
      cancelScheduled(handle);
      handle = undefined;
    }
  };
}

// ---------------------------------------------------------------------------
// Navigation-surviving promotion watch.
//
// The queued login branch starts this watch and immediately switches the unified
// /browser page from Settings to Lifecycle, destroying the settings pane before
// the poll's first 2s tick. The watch therefore lives at
// module level: component lifecycles never own or cancel it. It ends only by
// its own terminal states (opened / failed / timeout), an explicit
// close-login cancel, or auth loss — so the queued toast's promise ("we'll
// tell you when it opens") is kept regardless of navigation, and no interval
// outlives a logged-out session.
// ---------------------------------------------------------------------------

let activeLoginPromotionStop: (() => void) | null = null;
let detachLoginPromotionAuthListener: (() => void) | null = null;

/** Forget the active watch and its auth listener without stopping the poll. */
function forgetBrowserLoginPromotionWatch(): void {
  activeLoginPromotionStop = null;
  detachLoginPromotionAuthListener?.();
  detachLoginPromotionAuthListener = null;
}

export function hasBrowserLoginPromotionWatch(): boolean {
  return activeLoginPromotionStop !== null;
}

/** Stop the active watch (if any) without firing its callbacks. */
export function cancelBrowserLoginPromotionWatch(): void {
  const stop = activeLoginPromotionStop;
  forgetBrowserLoginPromotionWatch();
  stop?.();
}

/**
 * Start (or replace) the singleton promotion watch. At most one queued
 * Primary sign-in Lane exists per user, so a new queued login supersedes any
 * previous watch instead of stacking polls.
 */
export function beginBrowserLoginPromotionWatch(
  options: BrowserLoginPromotionPollOptions
): void {
  cancelBrowserLoginPromotionWatch();
  activeLoginPromotionStop = startBrowserLoginPromotionPoll({
    ...options,
    onOpened: () => {
      forgetBrowserLoginPromotionWatch();
      options.onOpened();
    },
    onStopped: (reason) => {
      forgetBrowserLoginPromotionWatch();
      options.onStopped(reason);
    },
  });
  // Auth loss (WebUI session expiry / logout redirect) must not leave a lanes
  // poll spinning against 401s for the rest of the attempt budget.
  if (typeof window !== 'undefined' && typeof window.addEventListener === 'function') {
    const onAuthExpired = () => cancelBrowserLoginPromotionWatch();
    window.addEventListener(AUTH_EXPIRED_EVENT, onAuthExpired);
    detachLoginPromotionAuthListener = () =>
      window.removeEventListener(AUTH_EXPIRED_EVENT, onAuthExpired);
  }
}

export type BrowserSecuritySettings = {
  persistentLogin: boolean;
  fullPower: boolean;
};

type BrowserSecuritySettingsKey =
  | 'agent.browserUse.persistentLogin'
  | 'agent.browserUse.fullPower';

const PERSISTENT_LOGIN_KEY = 'agent.browserUse.persistentLogin' as const;
const FULL_POWER_KEY = 'agent.browserUse.fullPower' as const;

const MIB = 1024 * 1024;
const GIB = 1024 * MIB;
const INTEGER_RESOURCE_POLICY_FIELDS = new Set<keyof BrowserResourcePolicyAdvanced>([
  'max_task_memory_bytes',
  'max_task_active_operations',
  'max_task_open_lanes',
  'max_task_tabs',
  'reserved_memory_bytes',
  'max_active_operations',
  'max_open_lanes',
  'max_queued_requests',
  'max_owner_queued_requests',
]);

export const BROWSER_RESOURCE_POLICY_LIMITS = {
  max_memory_ratio: { min: 0.1, max: 0.8 },
  max_task_memory_bytes: { min: 256 * MIB, max: 16 * GIB },
  max_task_active_operations: { min: 1, max: 16 },
  max_task_open_lanes: { min: 1, max: 32 },
  max_task_tabs: { min: 1, max: 64 },
  reserved_memory_bytes: { min: 256 * MIB, max: 512 * GIB },
  max_active_operations: { min: 1, max: 64 },
  max_open_lanes: { min: 1, max: 128 },
  max_queued_requests: { min: 1, max: 256 },
  max_owner_queued_requests: { min: 1, max: 32 },
} as const satisfies Record<keyof BrowserResourcePolicyAdvanced, { min: number; max: number }>;

export type BrowserResourcePolicyValidationError = {
  field: keyof BrowserResourcePolicyAdvanced;
  message: string;
};

export function validateBrowserResourcePolicy(
  policy: BrowserResourcePolicy,
  fallback?: BrowserResourcePolicy
): BrowserResourcePolicyValidationError | null {
  const advanced = policy.advanced;
  const fallbackAdvanced = fallback?.advanced;
  if (!advanced && !fallbackAdvanced) return null;

  for (const field of Object.keys(BROWSER_RESOURCE_POLICY_LIMITS) as (keyof BrowserResourcePolicyAdvanced)[]) {
    const value = advanced?.[field] ?? fallbackAdvanced?.[field];
    if (value === undefined) continue;
    const { min, max } = BROWSER_RESOURCE_POLICY_LIMITS[field];
    if (
      !Number.isFinite(value) ||
      value < min ||
      value > max ||
      (INTEGER_RESOURCE_POLICY_FIELDS.has(field) && !Number.isInteger(value))
    ) {
      return {
        field,
        message: INTEGER_RESOURCE_POLICY_FIELDS.has(field)
          ? `${field} must be an integer between ${min} and ${max}.`
          : `${field} must be between ${min} and ${max}.`,
      };
    }
  }

  const ownerQueue =
    advanced?.max_owner_queued_requests ?? fallbackAdvanced?.max_owner_queued_requests;
  const globalQueue = advanced?.max_queued_requests ?? fallbackAdvanced?.max_queued_requests;
  if (ownerQueue !== undefined && globalQueue !== undefined && ownerQueue > globalQueue) {
    return {
      field: 'max_owner_queued_requests',
      message: 'max_owner_queued_requests cannot exceed max_queued_requests.',
    };
  }

  const taskOperations =
    advanced?.max_task_active_operations ?? fallbackAdvanced?.max_task_active_operations;
  const globalOperations =
    advanced?.max_active_operations ?? fallbackAdvanced?.max_active_operations;
  if (
    taskOperations !== undefined &&
    globalOperations !== undefined &&
    taskOperations > globalOperations
  ) {
    return {
      field: 'max_task_active_operations',
      message: 'max_task_active_operations cannot exceed max_active_operations.',
    };
  }

  const taskLanes = advanced?.max_task_open_lanes ?? fallbackAdvanced?.max_task_open_lanes;
  const globalLanes = advanced?.max_open_lanes ?? fallbackAdvanced?.max_open_lanes;
  if (taskLanes !== undefined && globalLanes !== undefined && taskLanes > globalLanes) {
    return {
      field: 'max_task_open_lanes',
      message: 'max_task_open_lanes cannot exceed max_open_lanes.',
    };
  }

  return null;
}

export function browserSettingsServerErrorMessage(error: unknown): string | undefined {
  if (isBackendHttpError(error)) {
    const body = error.body;
    if (body && typeof body === 'object') {
      const structuredBody = body as {
        message?: unknown;
        error?: unknown;
        details?: unknown;
      };
      for (const candidate of [structuredBody.message, structuredBody.error]) {
        if (typeof candidate === 'string' && candidate.trim()) return candidate;
      }
      if (structuredBody.details && typeof structuredBody.details === 'object') {
        const detailsMessage = (structuredBody.details as { message?: unknown }).message;
        if (typeof detailsMessage === 'string' && detailsMessage.trim()) return detailsMessage;
      }
    }
    if (error.backendMessage.trim()) return error.backendMessage;
  }
  if (error instanceof Error && error.message.trim()) return error.message;
  return undefined;
}

export type BrowserSecuritySettingsStore = {
  get(key: BrowserSecuritySettingsKey): boolean | undefined;
  set(key: BrowserSecuritySettingsKey, value: boolean): Promise<void>;
  setLocal(key: BrowserSecuritySettingsKey, value: boolean): void;
  reload(): Promise<void>;
  isInitialized?: () => boolean;
};

export function readBrowserSecuritySettings(store: BrowserSecuritySettingsStore): BrowserSecuritySettings {
  const storedPersistentLogin = store.get(PERSISTENT_LOGIN_KEY);
  const storedFullPower = store.get(FULL_POWER_KEY);
  return {
    persistentLogin: typeof storedPersistentLogin === 'boolean' ? storedPersistentLogin : true,
    fullPower: typeof storedFullPower === 'boolean' ? storedFullPower : false,
  };
}

export function normalizeBrowserSecuritySettings(
  settings: BrowserSecuritySettings
): BrowserSecuritySettings {
  return {
    persistentLogin: settings.persistentLogin,
    // The backend treats these flags as a security mutex. Keep the renderer
    // fail-closed even when a legacy or hand-written caller asks for both.
    fullPower: settings.persistentLogin ? false : settings.fullPower,
  };
}

export async function persistBrowserSecuritySettingsTransaction(
  store: BrowserSecuritySettingsStore,
  previous: BrowserSecuritySettings,
  next: BrowserSecuritySettings
): Promise<BrowserSecuritySettings> {
  const effectiveNext = normalizeBrowserSecuritySettings(next);
  const changes: [
    BrowserSecuritySettingsKey,
    boolean,
    boolean,
  ][] = [];
  const fullPowerChange: (typeof changes)[number] = [
    FULL_POWER_KEY,
    previous.fullPower,
    effectiveNext.fullPower,
  ];
  const persistentLoginChange: (typeof changes)[number] = [
    PERSISTENT_LOGIN_KEY,
    previous.persistentLogin,
    effectiveNext.persistentLogin,
  ];

  // Keep the intermediate state safe: disable full-power before enabling
  // persistent login, and enable full-power only after persistent login is off.
  if (previous.fullPower !== effectiveNext.fullPower && effectiveNext.fullPower === false) {
    changes.push(fullPowerChange);
  }
  if (previous.persistentLogin !== effectiveNext.persistentLogin) {
    changes.push(persistentLoginChange);
  }
  if (previous.fullPower !== effectiveNext.fullPower && effectiveNext.fullPower === true) {
    changes.push(fullPowerChange);
  }
  const applied: (typeof changes)[number][] = [];

  try {
    for (const change of changes) {
      const [key, , value] = change;
      await store.set(key, value);
      applied.push(change);
    }
    return effectiveNext;
  } catch (error) {
    for (const [key, before] of applied.reverse()) {
      try {
        await store.set(key, before);
      } catch {
        // The authoritative reload below resolves the final persisted state.
      }
    }

    const setLocalSafely = (key: BrowserSecuritySettingsKey, value: boolean) => {
      try {
        store.setLocal(key, value);
      } catch {
        // Keep the original write error as the observable failure.
      }
    };

    try {
      await store.reload();
      if (store.isInitialized && !store.isInitialized()) {
        throw new Error('Browser security settings could not be reloaded.');
      }
      const persisted = readBrowserSecuritySettings(store);
      setLocalSafely(PERSISTENT_LOGIN_KEY, persisted.persistentLogin);
      setLocalSafely(FULL_POWER_KEY, persisted.fullPower);
    } catch {
      setLocalSafely(PERSISTENT_LOGIN_KEY, previous.persistentLogin);
      setLocalSafely(FULL_POWER_KEY, previous.fullPower);
    }

    throw error;
  }
}

const BrowserUseSettingsContent: React.FC = () => {
  const { t } = useTranslation();
  const translationRef = useRef(t);
  translationRef.current = t;
  const navigate = useNavigate();
  const [browserUse, setBrowserUse] = useState(false);
  const [source, setSource] = useState<BrowserSource>('system');
  const [displayMode, setDisplayMode] = useState<BrowserDisplayMode>('headless');
  const [displayModeStatus, setDisplayModeStatus] =
    useState<BrowserDisplayModeStatus>('loading');
  const [displayModeSaving, setDisplayModeSaving] = useState(false);
  const [displayModeError, setDisplayModeError] = useState<string | null>(null);
  const displayModeSavingRef = useRef(false);
  const displayModeControllerRef = useRef(
    createBrowserDisplayModeController({
      get: () => ipcBridge.browserSession.displayMode.get.invoke(),
      put: (next) =>
        ipcBridge.browserSession.displayMode.put.invoke({ display_mode: next }),
    })
  );
  const [persistentLogin, setPersistentLogin] = useState(true);
  const [fullPower, setFullPower] = useState(false);
  const [siteMemory, setSiteMemory] = useState(false);
  const [takeover, setTakeover] = useState(true);
  const [unrestrictedApproval, setUnrestrictedApproval] = useState(false);
  const [visualFallback, setVisualFallback] = useState(false);
  const [resourcePolicy, setResourcePolicy] = useState<BrowserResourcePolicy>({ preset: 'automatic' });
  const [persistedResourcePolicy, setPersistedResourcePolicy] = useState<BrowserResourcePolicy>({
    preset: 'automatic',
  });
  const [resourcePolicyStatus, setResourcePolicyStatus] = useState<ResourcePolicyStatus>('loading');
  const [resourcePolicySaving, setResourcePolicySaving] = useState(false);
  const [resourcePolicyError, setResourcePolicyError] = useState<BrowserSettingsError | null>(null);
  const [securitySettingsSaving, setSecuritySettingsSaving] = useState(false);
  const securitySettingsSavingRef = useRef(false);
  const [loginOpen, setLoginOpen] = useState(false);
  const [loginBusy, setLoginBusy] = useState(false);
  const [browserCapabilities, setBrowserCapabilities] = useState(() =>
    resolveBrowserOverviewCapabilities(null)
  );
  const { canManageBrowserSettings, canManagePrimaryIdentity } = browserCapabilities;

  useEffect(() => {
    const storedPersistentLogin = configService.get('agent.browserUse.persistentLogin') ?? true;
    const storedFullPower = configService.get('agent.browserUse.fullPower') ?? false;

    setBrowserUse(configService.get('agent.browserUse') ?? true);
    const displayModeMigration = migrateBrowserDisplayMode({
      displayMode: configService.get('agent.browserUse.displayMode'),
      displayModeVersion: configService.get('agent.browserUse.displayModeVersion'),
      // Classification input only (legacy-silent installs are 'lineage', not
      // 'default'); the value is never persisted or written back.
      silent: configService.get('agent.browserUse.silent'),
    });
    setDisplayMode(
      displayModeMigration.displayMode === 'external' ? 'external' : 'headless'
    );
    setSource((configService.get('agent.browserUse.source') as BrowserSource) ?? 'system');
    setPersistentLogin(storedPersistentLogin);
    setFullPower(storedPersistentLogin ? false : storedFullPower);
    setSiteMemory(configService.get('agent.browserUse.siteMemory') ?? false);
    // This setting controls approval for irreversible Agent actions. It is not
    // the removed Browser viewer's user-input takeover capability.
    setTakeover(configService.get('agent.browserUse.takeover') ?? true);
    setUnrestrictedApproval(configService.get('agent.browserUse.unrestrictedApproval') ?? false);
    setVisualFallback(configService.get('agent.browserUse.visualFallback') ?? false);

    if (storedPersistentLogin && storedFullPower) {
      securitySettingsSavingRef.current = true;
      setSecuritySettingsSaving(true);
      void persistBrowserSecuritySettingsTransaction(
        configService,
        { persistentLogin: true, fullPower: true },
        { persistentLogin: true, fullPower: false }
      ).catch((error) => {
        const persisted = readBrowserSecuritySettings(configService);
        setPersistentLogin(persisted.persistentLogin);
        setFullPower(persisted.fullPower);
        Message.error(
          browserSettingsServerErrorMessage(error) || translationRef.current('common.unknownError')
        );
      }).finally(() => {
        securitySettingsSavingRef.current = false;
        setSecuritySettingsSaving(false);
      });
    }
  }, []);

  useEffect(() => {
    const capabilityLoader = createBrowserSettingsCapabilityLoader({
      invoke: () => ipcBridge.browserSession.overview.invoke(),
      onCapabilities: setBrowserCapabilities,
    });
    const stopReloadingOnReconnect = ipcBridge.conversation.reconnected.on(() => {
      void capabilityLoader.reload();
    });
    void capabilityLoader.start();

    return () => {
      stopReloadingOnReconnect();
      capabilityLoader.dispose();
    };
  }, []);

  const loadDisplayMode = useCallback(async () => {
    setDisplayModeStatus('loading');
    setDisplayModeError(null);
    const result = await displayModeControllerRef.current.load();
    if (result.kind === 'applied') {
      setDisplayMode(result.displayMode);
      // Keep the renderer cache aligned only after the authoritative backend
      // has answered. Mode plus v2 marker form one trusted lineage boundary;
      // neither cache entry is used as proof that a write succeeded.
      cacheAuthoritativeBrowserDisplayMode(result.displayMode);
      setDisplayModeStatus('ready');
    } else if (result.kind === 'error') {
      const unavailable = isBrowserDisplayModeUnavailableError(result.error);
      setDisplayModeStatus(unavailable ? 'unavailable' : 'error');
      setDisplayModeError(
        unavailable
          ? null
          : (browserSettingsServerErrorMessage(result.error) ?? null)
      );
    }
  }, []);

  useEffect(() => {
    if (!canManageBrowserSettings) return;
    void loadDisplayMode();
  }, [canManageBrowserSettings, loadDisplayMode]);

  useEffect(
    () => () => displayModeControllerRef.current.dispose(),
    []
  );

  useEffect(() => {
    if (!canManageBrowserSettings) return;
    return ipcBridge.conversation.reconnected.on(() => {
      void loadDisplayMode();
    });
  }, [canManageBrowserSettings, loadDisplayMode]);

  const loadResourcePolicy = useCallback(async () => {
    setResourcePolicyStatus('loading');
    try {
      const policy = await browserResourcePolicyApi.get();
      setResourcePolicy(policy);
      setPersistedResourcePolicy(policy);
      setResourcePolicyStatus('ready');
      setResourcePolicyError(null);
    } catch (error) {
      const unavailable = isBrowserResourcePolicyUnavailableError(error);
      setResourcePolicyStatus(unavailable ? 'unavailable' : 'error');
      setResourcePolicyError(
        unavailable
          ? null
          : {
              message: browserSettingsServerErrorMessage(error),
              fallbackKey: 'settings.browserResourcePolicyLoadFailed',
            }
      );
    }
  }, []);

  useEffect(() => {
    if (!canManageBrowserSettings) return;
    void loadResourcePolicy();
  }, [canManageBrowserSettings, loadResourcePolicy]);

  // Reflect whether the managed Primary sign-in Lane is already open.
  useEffect(() => {
    if (!canManagePrimaryIdentity) {
      setLoginOpen(false);
      return;
    }
    let cancelled = false;
    ipcBridge.browserLogin.status
      .invoke()
      .then((res) => {
        if (!cancelled && res) setLoginOpen(!!res.active);
      })
      .catch(() => {});
    return () => {
      cancelled = true;
    };
  }, [canManagePrimaryIdentity]);

  // The queued promotion watch is intentionally NOT cancelled on unmount:
  // starting it is immediately followed by navigate('/browser'), which switches
  // to Lifecycle and unmounts this settings pane. The module-level singleton keeps the promise
  // made by the queued toast; it ends via its own terminal states,
  // close-login, or auth loss.
  const watchQueuedLoginPromotion = useCallback((laneId: string | undefined) => {
    beginBrowserLoginPromotionWatch({
      probeLifecycle: async () => {
        // No lane id means there is nothing to promote: stop as failed.
        if (!laneId) return null;
        const lanes = await ipcBridge.browserSession.lanes.invoke();
        return lanes.find((lane) => lane.lane_id === laneId)?.lifecycle_state ?? null;
      },
      foreground: async () => {
        const result = (await ipcBridge.browserSession.foregroundLane.invoke({
          lane_id: laneId ?? '',
        })) as { foregrounded?: boolean } | undefined;
        return result?.foregrounded === true;
      },
      onOpened: () => {
        Message.info(translationRef.current('settings.browserLoginOpenedHint'));
      },
      onStopped: (reason) => {
        setLoginOpen(false);
        Message.error(
          translationRef.current(
            reason === 'timeout'
              ? 'settings.browserLoginQueuedTimeout'
              : 'settings.browserLoginFailed'
          )
        );
      },
    });
  }, []);

  // Toggle the managed Primary sign-in Lane for the selected browser source.
  const handleLoginToggle = useCallback(async () => {
    if (!canManagePrimaryIdentity || loginBusy) return;
    setLoginBusy(true);
    try {
      if (loginOpen) {
        cancelBrowserLoginPromotionWatch();
        const res = await ipcBridge.browserLogin.close.invoke();
        setLoginOpen(res ? !!res.active : false);
      } else {
        const res = await ipcBridge.browserLogin.open.invoke({ source });
        setLoginOpen(res ? !!res.active : false);
        if (res && !res.active && (res.message || '').startsWith('launch_failed')) {
          Message.error(t('settings.browserLoginFailed'));
        } else if (res && res.active && res.message === 'queued') {
          // Queued: the lane is waiting for capacity and no window exists
          // yet. Announce the wait honestly and watch for promotion; only a
          // confirmed foreground after promotion reports an open window.
          Message.info(t('settings.browserLoginQueuedHint'));
          watchQueuedLoginPromotion(res.lane_id);
          navigate('/browser');
        } else if (res && res.active) {
          Message.info(t('settings.browserLoginOpenedHint'));
          navigate('/browser');
        }
      }
    } catch {
      Message.error(t('settings.browserLoginFailed'));
    } finally {
      setLoginBusy(false);
    }
  }, [
    canManagePrimaryIdentity,
    loginBusy,
    loginOpen,
    navigate,
    source,
    t,
    watchQueuedLoginPromotion,
  ]);

  const persistBoolean = useCallback(
    (key: Parameters<typeof configService.set>[0], checked: boolean, revert: () => void) => {
      configService.set(key, checked).catch(() => {
        revert();
        configService.setLocal(key, !checked);
      });
    },
    []
  );

  const handleBrowserUseChange = useCallback(
    (checked: boolean) => {
      setBrowserUse(checked);
      persistBoolean('agent.browserUse', checked, () => setBrowserUse(!checked));
    },
    [persistBoolean]
  );

  // Browser source only selects the executable. Both choices remain isolated,
  // Hub-owned instances; routine Agent work stays headless.
  const sourceRef = useRef(source);
  sourceRef.current = source;
  const handleSourceChange = useCallback(
    (value: string) => {
      const next: BrowserSource = value === 'system' ? 'system' : 'managed';
      // Capture the pre-change value outside the state updater: React may
      // replay updaters, and a network write with rollback inside one can
      // issue duplicate PUTs or roll back to a mid-transition value.
      const previous = sourceRef.current;
      if (next === previous) return;
      setSource(next);
      configService.set('agent.browserUse.source', next).catch(() => {
        setSource(previous);
        configService.setLocal('agent.browserUse.source', previous);
      });
    },
    []
  );

  // The display mode is a trusted user preference for the application-level
  // default visibility policy. It is enforced by the backend Host launch
  // policy; Agent tool JSON has no path into it, and neither option restores
  // the removed embedded viewer or user takeover.
  const handleDisplayModeChange = useCallback(
    async (value: string) => {
      if (
        displayModeStatus !== 'ready' ||
        displayModeSaving ||
        displayModeSavingRef.current
      ) {
        return;
      }
      const next: BrowserDisplayMode = value === 'external' ? 'external' : 'headless';
      if (next === displayMode) return;
      displayModeSavingRef.current = true;
      setDisplayModeSaving(true);
      setDisplayModeError(null);
      try {
        const result = await displayModeControllerRef.current.save(next);
        if (result.kind === 'applied') {
          setDisplayMode(result.displayMode);
          setDisplayModeStatus('ready');
          cacheAuthoritativeBrowserDisplayMode(result.displayMode);
          Message.success(translationRef.current('settings.browserDisplayModeSaved'));
          if (result.verificationError) {
            const message = browserSettingsServerErrorMessage(result.verificationError);
            setDisplayModeError(message ?? null);
            Message.error(
              message ||
                translationRef.current('settings.browserDisplayModeLoadFailedWithDetails', {
                  error: translationRef.current('common.unknownError'),
                })
            );
          }
        } else if (result.kind === 'rejected') {
          const message = result.nonPersistent
            ? browserSettingsServerErrorMessage(result.error)
            : result.unconfirmed
              ? translationRef.current('settings.browserDisplayModeUnconfirmed')
              : browserSettingsServerErrorMessage(result.error);
          setDisplayMode(result.displayMode);
          setDisplayModeStatus('ready');
          setDisplayModeError(message ?? null);
          // A live GET reconciles what the Hub currently runs, but a rejected
          // PUT does not prove that value was persisted. Never promote a
          // rejected result into the trusted mode+v2 fallback cache.
          Message.error(
            message || translationRef.current('settings.browserDisplayModeSaveFailed')
          );
        } else if (result.kind === 'unknown') {
          const message = browserSettingsServerErrorMessage(result.error);
          setDisplayModeStatus('error');
          setDisplayModeError(message ?? null);
          Message.error(
            message || translationRef.current('settings.browserDisplayModeSaveFailed')
          );
        }
      } finally {
        displayModeSavingRef.current = false;
        setDisplayModeSaving(false);
      }
    },
    [displayMode, displayModeSaving, displayModeStatus]
  );

  const persistResourcePolicy = useCallback(
    async (next: BrowserResourcePolicy, revertOnFailure?: BrowserResourcePolicy) => {
      setResourcePolicySaving(true);
      setResourcePolicyError(null);
      try {
        const saved = await browserResourcePolicyApi.put(next);
        setResourcePolicy(saved);
        setPersistedResourcePolicy(saved);
        setResourcePolicyStatus('ready');
      } catch (error) {
        if (revertOnFailure) {
          setResourcePolicy(revertOnFailure);
        }
        if (isBrowserResourcePolicyUnavailableError(error)) {
          setResourcePolicyStatus('unavailable');
        } else {
          const message = browserSettingsServerErrorMessage(error);
          setResourcePolicyError({
            message,
            fallbackKey: 'settings.browserResourcePolicySaveFailed',
          });
          Message.error(message || t('settings.browserResourcePolicySaveFailed'));
        }
      } finally {
        setResourcePolicySaving(false);
      }
    },
    [t]
  );

  const handleResourcePolicyPresetChange = useCallback(
    (value: string) => {
      setResourcePolicyError(null);
      const nextPreset: BrowserResourcePolicyPreset =
        value === 'resource_saving' || value === 'high_concurrency' ? value : 'automatic';
      const previous = resourcePolicy;
      // Advanced values that merely echo the fetched server state must not be
      // sent back: the backend applies them as overrides after the preset
      // transition, turning the switch into a label-only no-op.
      const request = buildBrowserResourcePolicyPresetRequest(
        nextPreset,
        previous,
        persistedResourcePolicy
      );
      const validationError = validateBrowserResourcePolicy(request, persistedResourcePolicy);
      if (validationError) {
        Message.error(validationError.message);
        return;
      }
      setResourcePolicy({ ...previous, preset: nextPreset });
      void persistResourcePolicy(request, previous);
    },
    [persistResourcePolicy, persistedResourcePolicy, resourcePolicy]
  );

  const handleResourcePolicyAdvancedChange = useCallback(
    (field: keyof BrowserResourcePolicyAdvanced, value: number | undefined) => {
      setResourcePolicyError(null);
      setResourcePolicy((previous) => {
        const advanced = { ...previous.advanced };
        if (typeof value === 'number' && Number.isFinite(value)) {
          advanced[field] = value;
        } else {
          delete advanced[field];
        }
        return {
          ...previous,
          advanced: Object.keys(advanced).length > 0 ? advanced : undefined,
        };
      });
    },
    []
  );

  const handleSaveResourcePolicyAdvanced = useCallback(() => {
    // Edited advanced values resolve the preset to 'custom': only then does
    // the backend honor an explicit reserved_memory_bytes instead of raising
    // it to the 20%-of-total floor that protects the named presets.
    const request = buildBrowserResourcePolicyAdvancedSaveRequest(
      resourcePolicy,
      persistedResourcePolicy
    );
    const validationError = validateBrowserResourcePolicy(request, persistedResourcePolicy);
    if (validationError) {
      Message.error(validationError.message);
      return;
    }
    void persistResourcePolicy(request);
  }, [persistResourcePolicy, persistedResourcePolicy, resourcePolicy]);

  const persistSecuritySettings = useCallback(
    async (next: BrowserSecuritySettings) => {
      if (securitySettingsSavingRef.current) return;
      const previous = { persistentLogin, fullPower };
      securitySettingsSavingRef.current = true;
      setPersistentLogin(next.persistentLogin);
      setFullPower(next.fullPower);
      setSecuritySettingsSaving(true);
      try {
        await persistBrowserSecuritySettingsTransaction(configService, previous, next);
      } catch (error) {
        const persisted = readBrowserSecuritySettings(configService);
        setPersistentLogin(persisted.persistentLogin);
        setFullPower(persisted.fullPower);
        Message.error(browserSettingsServerErrorMessage(error) || t('common.unknownError'));
      } finally {
        securitySettingsSavingRef.current = false;
        setSecuritySettingsSaving(false);
      }
    },
    [fullPower, persistentLogin, t]
  );

  const handlePersistentLoginChange = useCallback(
    (checked: boolean) => {
      void persistSecuritySettings({
        persistentLogin: checked,
        fullPower: checked ? false : fullPower,
      });
    },
    [fullPower, persistSecuritySettings]
  );

  const handleFullPowerChange = useCallback(
    (checked: boolean) => {
      void persistSecuritySettings({
        persistentLogin,
        fullPower: checked,
      });
    },
    [persistSecuritySettings, persistentLogin]
  );

  const handleSiteMemoryChange = useCallback(
    (checked: boolean) => {
      setSiteMemory(checked);
      persistBoolean('agent.browserUse.siteMemory', checked, () => setSiteMemory(!checked));
    },
    [persistBoolean]
  );

  const handleTakeoverChange = useCallback(
    (checked: boolean) => {
      setTakeover(checked);
      persistBoolean('agent.browserUse.takeover', checked, () => setTakeover(!checked));
    },
    [persistBoolean]
  );

  const handleUnrestrictedApprovalChange = useCallback(
    (checked: boolean) => {
      if (!checked) {
        setUnrestrictedApproval(false);
        persistBoolean('agent.browserUse.unrestrictedApproval', false, () => setUnrestrictedApproval(true));
        return;
      }

      Modal.confirm({
        title: t('settings.browserUnrestrictedApprovalConfirmTitle'),
        content: t('settings.browserUnrestrictedApprovalConfirmContent'),
        okText: t('settings.browserUnrestrictedApprovalConfirmOk'),
        onOk: () => {
          setUnrestrictedApproval(true);
          persistBoolean('agent.browserUse.unrestrictedApproval', true, () => setUnrestrictedApproval(false));
        },
      });
    },
    [persistBoolean, t]
  );

  const handleVisualFallbackChange = useCallback(
    (checked: boolean) => {
      setVisualFallback(checked);
      persistBoolean('agent.browserUse.visualFallback', checked, () => setVisualFallback(!checked));
    },
    [persistBoolean]
  );

  const fullPowerDisabled = !browserUse || persistentLogin || securitySettingsSaving;
  const resourcePolicyDisabled = resourcePolicyStatus !== 'ready' || resourcePolicySaving;
  const resourcePolicyPlaceholder = t('settings.browserResourcePolicyBackendDefault');
  const resourcePolicyValidationError = validateBrowserResourcePolicy(resourcePolicy, persistedResourcePolicy);
  const resourcePolicyErrorMessage = resourcePolicyError
    ? resourcePolicyError.message || t(resourcePolicyError.fallbackKey)
    : null;

  return (
    <div className='flex flex-col h-full min-h-0 w-full overflow-hidden'>
      <NomiScrollArea className='flex-1 min-h-0 pb-8px scrollbar-hide'>
        <div className='space-y-10px'>
          <section className='space-y-6px'>
            <h2 className='m-0 px-2px text-13px font-600 text-t-secondary'>
              {t('settings.browserUseSection')}
            </h2>
            <div className='box-border px-12px md:px-16px py-12px bg-2 rd-12px border border-solid border-[var(--color-border-2)]'>
              {/*
                Separator recipe used by every panel in this file. `divide-y` emits only a width and
                this project ships no border reset, so the style stays `none` unless `divide-solid` is
                present. `divide-solid` styles all four sides, so `divide-x-0` is needed to stop the
                unset left/right widths falling back to the CSS initial `medium` (~3px). The old
                `divide-border-2` emitted nothing at all: there is no theme colour named `border`.
              */}
              <div className='w-full flex flex-col divide-y divide-x-0 divide-solid divide-[var(--color-border-2)]'>
              <PreferenceRow label={t('settings.browserUse')} description={t('settings.browserUseDesc')}>
                <Switch checked={browserUse} onChange={handleBrowserUseChange} />
              </PreferenceRow>
              <PreferenceRow label={t('settings.browserSource')} description={t('settings.browserSourceDesc')}>
                <RadioGroup type='button' value={source} disabled={!browserUse} onChange={handleSourceChange}>
                  <Radio value='managed'>{t('settings.browserSourceManaged')}</Radio>
                  <Radio value='system'>{t('settings.browserSourceSystem')}</Radio>
                </RadioGroup>
              </PreferenceRow>
              {canManageBrowserSettings && (
                <>
                  <PreferenceRow
                    label={t('settings.browserDisplayMode')}
                    description={t('settings.browserDisplayModeDesc')}
                  >
                    <RadioGroup
                      type='button'
                      value={displayMode}
                      disabled={displayModeStatus !== 'ready' || displayModeSaving}
                      onChange={(value) => void handleDisplayModeChange(value)}
                    >
                      <Radio value='headless'>{t('settings.browserDisplayModeHeadless')}</Radio>
                      <Radio value='external'>{t('settings.browserDisplayModeExternal')}</Radio>
                    </RadioGroup>
                  </PreferenceRow>
                  {(displayModeStatus === 'unavailable' ||
                    displayModeStatus === 'error' ||
                    displayModeError) && (
                    <Alert
                      className='my-8px'
                      type='warning'
                      showIcon
                      content={
                        displayModeError
                          ? t('settings.browserDisplayModeLoadFailedWithDetails', {
                              error: displayModeError,
                            })
                          : t('settings.browserDisplayModeUnavailable')
                      }
                    />
                  )}
                </>
              )}
              {canManagePrimaryIdentity && (
                <PreferenceRow
                  label={t('settings.browserLogin')}
                  description={t('settings.browserLoginDesc')}
                >
                  <Button
                    size='small'
                    loading={loginBusy}
                    disabled={!browserUse}
                    onClick={handleLoginToggle}
                  >
                    {loginOpen ? t('settings.browserLoginClose') : t('settings.browserLoginOpen')}
                  </Button>
                </PreferenceRow>
              )}
              <PreferenceRow
                label={t('settings.browserPersistentLogin')}
                description={t('settings.browserPersistentLoginDesc')}
              >
                <Switch
                  checked={persistentLogin}
                  loading={securitySettingsSaving}
                  disabled={!browserUse || securitySettingsSaving}
                  onChange={handlePersistentLoginChange}
                />
              </PreferenceRow>
              <PreferenceRow
                label={t('settings.browserFullPower')}
                description={
                  persistentLogin
                    ? t('settings.browserFullPowerDisabledByPersistentLogin')
                    : t('settings.browserFullPowerDesc')
                }
              >
                <Switch checked={fullPower} disabled={fullPowerDisabled} onChange={handleFullPowerChange} />
              </PreferenceRow>
              <PreferenceRow label={t('settings.browserSiteMemory')} description={t('settings.browserSiteMemoryDesc')}>
                <Switch checked={siteMemory} disabled={!browserUse} onChange={handleSiteMemoryChange} />
              </PreferenceRow>
              <PreferenceRow label={t('settings.browserTakeover')} description={t('settings.browserTakeoverDesc')}>
                <Switch checked={takeover} disabled={!browserUse} onChange={handleTakeoverChange} />
              </PreferenceRow>
              <PreferenceRow
                label={t('settings.browserUnrestrictedApproval')}
                description={t('settings.browserUnrestrictedApprovalDesc')}
              >
                <Switch
                  checked={unrestrictedApproval}
                  disabled={!browserUse}
                  onChange={handleUnrestrictedApprovalChange}
                />
              </PreferenceRow>
              <PreferenceRow
                label={t('settings.browserVisualFallback')}
                description={t('settings.browserVisualFallbackDesc')}
              >
                <Switch checked={visualFallback} disabled={!browserUse} onChange={handleVisualFallbackChange} />
              </PreferenceRow>
              </div>
            </div>
          </section>

          {canManageBrowserSettings && (
            <section className='space-y-6px'>
              <h2 className='m-0 px-2px text-13px font-600 text-t-secondary'>
                {t('settings.browserResourcePolicySection')}
              </h2>
              <div className='box-border px-12px md:px-16px py-12px bg-2 rd-12px border border-solid border-[var(--color-border-2)] space-y-8px'>
                <div className='w-full flex flex-col divide-y divide-x-0 divide-solid divide-[var(--color-border-2)]'>
                <PreferenceRow
                  label={t('settings.browserResourcePolicy')}
                  description={t('settings.browserResourcePolicyDesc')}
                >
                  <RadioGroup
                    type='button'
                    value={resourcePolicy.preset}
                    disabled={resourcePolicyDisabled}
                    onChange={handleResourcePolicyPresetChange}
                  >
                    <Radio value='automatic'>{t('settings.browserResourcePolicyAutomatic')}</Radio>
                    <Radio value='resource_saving'>{t('settings.browserResourcePolicySaving')}</Radio>
                    <Radio value='high_concurrency'>{t('settings.browserResourcePolicyHighConcurrency')}</Radio>
                  </RadioGroup>
                </PreferenceRow>
              </div>

              {resourcePolicyStatus === 'loading' && (
                <Alert type='info' showIcon content={t('settings.browserResourcePolicyLoading')} />
              )}
              {resourcePolicyStatus === 'unavailable' && (
                <Alert type='info' showIcon content={t('settings.browserResourcePolicyUnavailable')} />
              )}
              {resourcePolicyStatus === 'error' && (
                <div className='space-y-8px'>
                  <Alert
                    type='warning'
                    showIcon
                    content={resourcePolicyErrorMessage || t('settings.browserResourcePolicyLoadFailed')}
                  />
                  <Button size='small' onClick={() => void loadResourcePolicy()}>
                    {t('settings.browserResourcePolicyRetry')}
                  </Button>
                </div>
              )}
              {resourcePolicyStatus === 'ready' && (resourcePolicyErrorMessage || resourcePolicyValidationError) && (
                <Alert
                  type='warning'
                  showIcon
                  content={resourcePolicyErrorMessage || resourcePolicyValidationError?.message}
                />
              )}

              <Collapse
                bordered={false}
                className='[&_.arco-collapse-item]:!border-none [&_.arco-collapse-item-header]:!px-0 [&_.arco-collapse-item-content-box]:!px-0 [&_.arco-collapse-item-content-box]:!pb-0'
              >
                <Collapse.Item
                  name='advanced'
                  header={
                    <div>
                      <div className='text-14px text-2'>{t('settings.browserResourcePolicyAdvanced')}</div>
                      <div className='text-12px text-t-tertiary mt-4px'>
                        {t('settings.browserResourcePolicyAdvancedDesc')}
                      </div>
                    </div>
                  }
                >
                  <div className='w-full flex flex-col divide-y divide-x-0 divide-solid divide-[var(--color-border-2)]'>
                  <PreferenceRow
                    label={t('settings.browserResourceMaxMemoryRatio')}
                    description={t('settings.browserResourceMaxMemoryRatioDesc')}
                  >
                    <InputNumber
                      value={resourcePolicy.advanced?.max_memory_ratio}
                      disabled={resourcePolicyDisabled}
                      min={BROWSER_RESOURCE_POLICY_LIMITS.max_memory_ratio.min}
                      max={BROWSER_RESOURCE_POLICY_LIMITS.max_memory_ratio.max}
                      step={0.05}
                      precision={2}
                      placeholder={resourcePolicyPlaceholder}
                      style={{ width: 180 }}
                      onChange={(value) => handleResourcePolicyAdvancedChange('max_memory_ratio', value)}
                    />
                  </PreferenceRow>
                  <PreferenceRow
                    label={t('settings.browserResourceMaxTaskMemoryBytes')}
                    description={t('settings.browserResourceMaxTaskMemoryBytesDesc')}
                  >
                    <InputNumber
                      value={resourcePolicy.advanced?.max_task_memory_bytes}
                      disabled={resourcePolicyDisabled}
                      min={BROWSER_RESOURCE_POLICY_LIMITS.max_task_memory_bytes.min}
                      max={BROWSER_RESOURCE_POLICY_LIMITS.max_task_memory_bytes.max}
                      step={268435456}
                      suffix='B'
                      placeholder={resourcePolicyPlaceholder}
                      style={{ width: 180 }}
                      onChange={(value) => handleResourcePolicyAdvancedChange('max_task_memory_bytes', value)}
                    />
                  </PreferenceRow>
                  <PreferenceRow
                    label={t('settings.browserResourceMaxTaskActiveOperations')}
                    description={t('settings.browserResourceMaxTaskActiveOperationsDesc')}
                  >
                    <InputNumber
                      value={resourcePolicy.advanced?.max_task_active_operations}
                      disabled={resourcePolicyDisabled}
                      min={BROWSER_RESOURCE_POLICY_LIMITS.max_task_active_operations.min}
                      max={BROWSER_RESOURCE_POLICY_LIMITS.max_task_active_operations.max}
                      precision={0}
                      placeholder={resourcePolicyPlaceholder}
                      style={{ width: 180 }}
                      onChange={(value) => handleResourcePolicyAdvancedChange('max_task_active_operations', value)}
                    />
                  </PreferenceRow>
                  <PreferenceRow
                    label={t('settings.browserResourceMaxTaskOpenLanes')}
                    description={t('settings.browserResourceMaxTaskOpenLanesDesc')}
                  >
                    <InputNumber
                      value={resourcePolicy.advanced?.max_task_open_lanes}
                      disabled={resourcePolicyDisabled}
                      min={BROWSER_RESOURCE_POLICY_LIMITS.max_task_open_lanes.min}
                      max={BROWSER_RESOURCE_POLICY_LIMITS.max_task_open_lanes.max}
                      precision={0}
                      placeholder={resourcePolicyPlaceholder}
                      style={{ width: 180 }}
                      onChange={(value) => handleResourcePolicyAdvancedChange('max_task_open_lanes', value)}
                    />
                  </PreferenceRow>
                  <PreferenceRow
                    label={t('settings.browserResourceMaxTaskTabs')}
                    description={t('settings.browserResourceMaxTaskTabsDesc')}
                  >
                    <InputNumber
                      value={resourcePolicy.advanced?.max_task_tabs}
                      disabled={resourcePolicyDisabled}
                      min={BROWSER_RESOURCE_POLICY_LIMITS.max_task_tabs.min}
                      max={BROWSER_RESOURCE_POLICY_LIMITS.max_task_tabs.max}
                      precision={0}
                      placeholder={resourcePolicyPlaceholder}
                      style={{ width: 180 }}
                      onChange={(value) => handleResourcePolicyAdvancedChange('max_task_tabs', value)}
                    />
                  </PreferenceRow>
                  <PreferenceRow
                    label={t('settings.browserResourceReservedMemoryBytes')}
                    description={t('settings.browserResourceReservedMemoryBytesDesc')}
                  >
                    <InputNumber
                      value={resourcePolicy.advanced?.reserved_memory_bytes}
                      disabled={resourcePolicyDisabled}
                      min={BROWSER_RESOURCE_POLICY_LIMITS.reserved_memory_bytes.min}
                      max={BROWSER_RESOURCE_POLICY_LIMITS.reserved_memory_bytes.max}
                      step={268435456}
                      suffix='B'
                      placeholder={resourcePolicyPlaceholder}
                      style={{ width: 180 }}
                      onChange={(value) => handleResourcePolicyAdvancedChange('reserved_memory_bytes', value)}
                    />
                  </PreferenceRow>
                  <PreferenceRow
                    label={t('settings.browserResourceMaxActiveOperations')}
                    description={t('settings.browserResourceMaxActiveOperationsDesc')}
                  >
                    <InputNumber
                      value={resourcePolicy.advanced?.max_active_operations}
                      disabled={resourcePolicyDisabled}
                      min={BROWSER_RESOURCE_POLICY_LIMITS.max_active_operations.min}
                      max={BROWSER_RESOURCE_POLICY_LIMITS.max_active_operations.max}
                      precision={0}
                      placeholder={resourcePolicyPlaceholder}
                      style={{ width: 180 }}
                      onChange={(value) => handleResourcePolicyAdvancedChange('max_active_operations', value)}
                    />
                  </PreferenceRow>
                  <PreferenceRow
                    label={t('settings.browserResourceMaxOpenLanes')}
                    description={t('settings.browserResourceMaxOpenLanesDesc')}
                  >
                    <InputNumber
                      value={resourcePolicy.advanced?.max_open_lanes}
                      disabled={resourcePolicyDisabled}
                      min={BROWSER_RESOURCE_POLICY_LIMITS.max_open_lanes.min}
                      max={BROWSER_RESOURCE_POLICY_LIMITS.max_open_lanes.max}
                      precision={0}
                      placeholder={resourcePolicyPlaceholder}
                      style={{ width: 180 }}
                      onChange={(value) => handleResourcePolicyAdvancedChange('max_open_lanes', value)}
                    />
                  </PreferenceRow>
                  <PreferenceRow
                    label={t('settings.browserResourceMaxQueuedRequests')}
                    description={t('settings.browserResourceMaxQueuedRequestsDesc')}
                  >
                    <InputNumber
                      value={resourcePolicy.advanced?.max_queued_requests}
                      disabled={resourcePolicyDisabled}
                      min={BROWSER_RESOURCE_POLICY_LIMITS.max_queued_requests.min}
                      max={BROWSER_RESOURCE_POLICY_LIMITS.max_queued_requests.max}
                      precision={0}
                      placeholder={resourcePolicyPlaceholder}
                      style={{ width: 180 }}
                      onChange={(value) => handleResourcePolicyAdvancedChange('max_queued_requests', value)}
                    />
                  </PreferenceRow>
                  <PreferenceRow
                    label={t('settings.browserResourceMaxOwnerQueuedRequests')}
                    description={t('settings.browserResourceMaxOwnerQueuedRequestsDesc')}
                  >
                    <InputNumber
                      value={resourcePolicy.advanced?.max_owner_queued_requests}
                      disabled={resourcePolicyDisabled}
                      min={BROWSER_RESOURCE_POLICY_LIMITS.max_owner_queued_requests.min}
                      max={BROWSER_RESOURCE_POLICY_LIMITS.max_owner_queued_requests.max}
                      precision={0}
                      placeholder={resourcePolicyPlaceholder}
                      style={{ width: 180 }}
                      onChange={(value) => handleResourcePolicyAdvancedChange('max_owner_queued_requests', value)}
                    />
                  </PreferenceRow>
                  </div>
                  <div className='flex justify-end pt-12px'>
                    <Button
                      type='primary'
                      size='small'
                      loading={resourcePolicySaving}
                      disabled={resourcePolicyStatus !== 'ready' || resourcePolicyValidationError !== null}
                      onClick={handleSaveResourcePolicyAdvanced}
                    >
                      {t('settings.browserResourcePolicySaveAdvanced')}
                    </Button>
                  </div>
                </Collapse.Item>
                </Collapse>
              </div>
            </section>
          )}

          <Alert type='warning' showIcon content={t('settings.browserUseRiskHint')} />
        </div>
      </NomiScrollArea>
    </div>
  );
};

export default BrowserUseSettingsContent;
