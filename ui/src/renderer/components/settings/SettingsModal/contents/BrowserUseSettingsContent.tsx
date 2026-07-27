/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { isBackendHttpError } from '@/common/adapter/httpBridge';
import { configService } from '@/common/config/configService';
import {
  browserResourcePolicyApi,
  isBrowserResourcePolicyUnavailableError,
  migrateBrowserDisplayMode,
  type BrowserResourcePolicy,
  type BrowserResourcePolicyAdvanced,
  type BrowserResourcePolicyPreset,
} from '@/common/browser/browserSettings';
import {
  resolveBrowserOverviewCapabilities,
  type IBrowserOverview,
  type IBrowserOverviewCapabilities,
} from '@/common/browser/browserTypes';
import { ipcBridge } from '@/common';
import NomiScrollArea from '@/renderer/components/base/NomiScrollArea';
import { Alert, Button, Collapse, InputNumber, Message, Modal, Radio, Switch } from '@arco-design/web-react';
import React, { useCallback, useEffect, useRef, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { useNavigate } from 'react-router-dom';
import { useSettingsViewMode } from '../settingsViewContext';
import PreferenceRow from './SystemModalContent/PreferenceRow';

const RadioGroup = Radio.Group;

type BrowserSource = 'managed' | 'system';
type ResourcePolicyStatus = 'loading' | 'ready' | 'unavailable' | 'error';
type BrowserSettingsError = {
  message?: string;
  fallbackKey: BrowserSettingsErrorKey;
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
  'reserved_memory_bytes',
  'max_active_operations',
  'max_open_lanes',
  'max_queued_requests',
  'max_owner_queued_requests',
]);

export const BROWSER_RESOURCE_POLICY_LIMITS = {
  max_memory_ratio: { min: 0.1, max: 0.8 },
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
  const viewMode = useSettingsViewMode();
  const isPageMode = viewMode === 'page';
  const [browserUse, setBrowserUse] = useState(false);
  const [source, setSource] = useState<BrowserSource>('system');
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
      silent: configService.get('agent.browserUse.silent'),
    });
    if (displayModeMigration.shouldPersist) {
      configService.set('agent.browserUse.displayMode', displayModeMigration.displayMode).catch(() => {
        configService.setLocal('agent.browserUse.displayMode', undefined);
        Message.error(translationRef.current('settings.browserDisplayModeSaveFailed'));
      });
    }
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

  // Phase 2b: reflect whether the managed Primary login window is already open.
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

  // Toggle the managed Primary login window for the selected browser source.
  const handleLoginToggle = useCallback(async () => {
    if (!canManagePrimaryIdentity || loginBusy) return;
    setLoginBusy(true);
    try {
      if (loginOpen) {
        const res = await ipcBridge.browserLogin.close.invoke();
        setLoginOpen(res ? !!res.active : false);
      } else {
        const res = await ipcBridge.browserLogin.open.invoke({ source });
        setLoginOpen(res ? !!res.active : false);
        if (res && !res.active && (res.message || '').startsWith('launch_failed')) {
          Message.error(t('settings.browserLoginFailed'));
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
  }, [canManagePrimaryIdentity, loginBusy, loginOpen, navigate, source, t]);

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
  // managed instances shown in an external window.
  const handleSourceChange = useCallback(
    (value: string) => {
      const next: BrowserSource = value === 'system' ? 'system' : 'managed';
      setSource((prev) => {
        configService.set('agent.browserUse.source', next).catch(() => {
          setSource(prev);
          configService.setLocal('agent.browserUse.source', prev);
        });
        return next;
      });
    },
    []
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
      const next = { ...previous, preset: nextPreset };
      const validationError = validateBrowserResourcePolicy(next, persistedResourcePolicy);
      if (validationError) {
        Message.error(validationError.message);
        return;
      }
      setResourcePolicy(next);
      void persistResourcePolicy(next, previous);
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
    const validationError = validateBrowserResourcePolicy(resourcePolicy, persistedResourcePolicy);
    if (validationError) {
      Message.error(validationError.message);
      return;
    }
    void persistResourcePolicy(resourcePolicy);
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
    <div className='flex flex-col h-full w-full'>
      <NomiScrollArea className='flex-1 min-h-0 pb-16px' disableOverflow={isPageMode}>
        <div className='space-y-16px'>
          <div className='px-[12px] md:px-[32px] py-16px bg-2 rd-16px space-y-12px'>
            <div className='text-13px font-600 text-t-secondary'>{t('settings.browserUseSection')}</div>
            <div className='w-full flex flex-col divide-y divide-border-2'>
              <PreferenceRow label={t('settings.browserUse')} description={t('settings.browserUseDesc')}>
                <Switch checked={browserUse} onChange={handleBrowserUseChange} />
              </PreferenceRow>
              <PreferenceRow label={t('settings.browserSource')} description={t('settings.browserSourceDesc')}>
                <RadioGroup type='button' value={source} disabled={!browserUse} onChange={handleSourceChange}>
                  <Radio value='managed'>{t('settings.browserSourceManaged')}</Radio>
                  <Radio value='system'>{t('settings.browserSourceSystem')}</Radio>
                </RadioGroup>
              </PreferenceRow>
              <PreferenceRow
                label={t('settings.browserDisplayMode')}
                description={t('settings.browserDisplayModeDesc')}
              >
                <span className='text-13px text-t-secondary'>
                  {t('settings.browserDisplayModeExternal')}
                </span>
              </PreferenceRow>
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

          {canManageBrowserSettings && (
            <div className='px-[12px] md:px-[32px] py-16px bg-2 rd-16px space-y-12px'>
              <div className='text-13px font-600 text-t-secondary'>
                {t('settings.browserResourcePolicySection')}
              </div>
              <div className='w-full flex flex-col divide-y divide-border-2'>
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
                  <div className='w-full flex flex-col divide-y divide-border-2'>
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
          )}

          <Alert type='warning' showIcon content={t('settings.browserUseRiskHint')} />
        </div>
      </NomiScrollArea>
    </div>
  );
};

export default BrowserUseSettingsContent;
