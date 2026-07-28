/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { readFileSync } from 'node:fs';
import { describe, expect, test } from 'bun:test';
import {
  BROWSER_SETTINGS_OVERVIEW_RETRY_DELAYS_MS,
  BROWSER_RESOURCE_POLICY_LIMITS,
  browserSettingsServerErrorMessage,
  createBrowserSettingsCapabilityLoader,
  persistBrowserSecuritySettingsTransaction,
  validateBrowserResourcePolicy,
} from './BrowserUseSettingsContent';

const readSource = (url: URL) => readFileSync(url, 'utf8');

type SecurityKey =
  | 'agent.browserUse.persistentLogin'
  | 'agent.browserUse.fullPower';
type SecurityValues = Record<SecurityKey, boolean>;

const PERSISTENT_LOGIN_KEY = 'agent.browserUse.persistentLogin' as const;
const FULL_POWER_KEY = 'agent.browserUse.fullPower' as const;

const cloneSecurityValues = (values: SecurityValues): SecurityValues => ({ ...values });

const captureError = async (operation: () => Promise<unknown>): Promise<unknown> => {
  try {
    await operation();
    return undefined;
  } catch (error) {
    return error;
  }
};

const browserOverviewWithCapabilities = {
  supported: true,
  enabled: true,
  running_lanes: 0,
  queued_lanes: 0,
  can_close_all: false,
  can_manage_browser_settings: true,
  can_manage_primary_identity: true,
};

const createManualScheduler = () => {
  let nextId = 0;
  const pending = new Map<number, { callback: () => void | Promise<void>; delayMs: number }>();
  return {
    schedule: (callback: () => void | Promise<void>, delayMs: number): number => {
      const id = ++nextId;
      pending.set(id, { callback, delayMs });
      return id;
    },
    cancel: (handle: unknown) => {
      pending.delete(handle as number);
    },
    delays: () => [...pending.values()].map(({ delayMs }) => delayMs),
    runNext: async () => {
      const entry = pending.entries().next().value as
        | [number, { callback: () => void | Promise<void>; delayMs: number }]
        | undefined;
      if (!entry) throw new Error('No scheduled Browser overview retry.');
      pending.delete(entry[0]);
      await entry[1].callback();
    },
  };
};

describe('Browser Use settings contract', () => {
  test('trusts only v2 local fallback state and applies display mode through the live API', () => {
    const source = readSource(new URL('./BrowserUseSettingsContent.tsx', import.meta.url));

    expect(source.includes("configService.get('agent.browserUse.displayModeVersion')")).toBe(true);
    expect(source.includes("configService.get('agent.browserUse.silent')")).toBe(false);
    expect(source.includes("configService.set('agent.browserUse.displayMode'")).toBe(false);
    expect(source.includes('ipcBridge.browserSession.displayMode.get.invoke()')).toBe(true);
    expect(source.includes('ipcBridge.browserSession.displayMode.put.invoke')).toBe(true);
    expect(source.includes('createBrowserDisplayModeController')).toBe(true);
    expect(source.includes('setDisplayMode(previous)')).toBe(false);
    expect(source.includes("configService.setLocal('agent.browserUse.displayMode'")).toBe(true);
    expect(
      source.includes("configService.setLocal(\n    'agent.browserUse.displayModeVersion'")
    ).toBe(true);
    expect(source.includes('BROWSER_DISPLAY_MODE_POLICY_VERSION')).toBe(true);
    expect(/configService\.(?:set|setLocal|setBatch)\('agent\.browserUse\.silent'/.test(source)).toBe(false);
    const rejectedStart = source.indexOf("} else if (result.kind === 'rejected') {");
    const rejectedEnd = source.indexOf("} else if (result.kind === 'unknown') {", rejectedStart);
    expect(rejectedStart).toBeGreaterThan(-1);
    expect(rejectedEnd).toBeGreaterThan(rejectedStart);
    expect(
      source
        .slice(rejectedStart, rejectedEnd)
        .includes('cacheAuthoritativeBrowserDisplayMode')
    ).toBe(false);
    expect(source.slice(rejectedStart, rejectedEnd).includes('result.nonPersistent')).toBe(
      true
    );
    // The removed embedded viewer must never come back as a selectable mode;
    // headless (default) and external are the two trusted user policies.
    expect(source.includes("<Radio value='embedded'>")).toBe(false);
    expect(source.includes("<Radio value='headless'>")).toBe(true);
    expect(source.includes("<Radio value='external'>")).toBe(true);
    expect(source.includes("t('settings.browserDisplayModeHeadless')")).toBe(true);
    expect(source.includes("t('settings.browserDisplayModeExternal')")).toBe(true);
    expect(source.includes("t('settings.browserDisplayModeDesc')")).toBe(true);
    expect(source.includes("persistBoolean('agent.browserUse.takeover'")).toBe(true);
    expect(source.includes("configService.get('agent.browserUse.takeover')")).toBe(true);
    expect(source.includes('displayModeStatus !== \'ready\'')).toBe(true);
  });

  test('exposes the three resource presets and advanced resource fields', () => {
    const source = readSource(new URL('./BrowserUseSettingsContent.tsx', import.meta.url));

    expect(source.includes("<Radio value='automatic'>")).toBe(true);
    expect(source.includes("<Radio value='resource_saving'>")).toBe(true);
    expect(source.includes("<Radio value='high_concurrency'>")).toBe(true);
    expect(source.includes("name='advanced'")).toBe(true);
    expect(source.includes("'max_memory_ratio'")).toBe(true);
    expect(source.includes("'reserved_memory_bytes'")).toBe(true);
    expect(source.includes("'max_active_operations'")).toBe(true);
    expect(source.includes("'max_open_lanes'")).toBe(true);
    expect(source.includes("'max_queued_requests'")).toBe(true);
    expect(source.includes("'max_owner_queued_requests'")).toBe(true);
  });

  test('fails closed for installation-owner Browser controls while keeping ordinary preferences', () => {
    const source = readSource(new URL('./BrowserUseSettingsContent.tsx', import.meta.url));

    expect(source.includes('ipcBridge.browserSession.overview')).toBe(true);
    expect(source.includes('resolveBrowserOverviewCapabilities(null)')).toBe(true);
    expect(source.includes('resolveBrowserOverviewCapabilities(overview)')).toBe(true);
    expect(source.includes('if (!canManageBrowserSettings) return;')).toBe(true);
    expect(source.includes('if (!canManagePrimaryIdentity)')).toBe(true);
    expect(source.includes('{canManagePrimaryIdentity && (')).toBe(true);
    expect(source.includes('{canManageBrowserSettings && (')).toBe(true);

    // The global display policy is owner-scoped, while per-user security
    // preferences remain available to ordinary users.
    expect(source.includes("label={t('settings.browserDisplayMode')}")).toBe(true);
    expect(source.includes('{canManageBrowserSettings && (')).toBe(true);
    expect(source.includes("label={t('settings.browserPersistentLogin')}")).toBe(true);
    expect(source.includes("label={t('settings.browserFullPower')}")).toBe(true);
    expect(source.includes("label={t('settings.browserTakeover')}")).toBe(true);
  });

  test('recovers Browser capabilities after a bounded delayed overview retry', async () => {
    const scheduler = createManualScheduler();
    const capabilities: Array<{
      canCloseAll: boolean;
      canManageBrowserSettings: boolean;
      canManagePrimaryIdentity: boolean;
    }> = [];
    let calls = 0;
    const loader = createBrowserSettingsCapabilityLoader({
      invoke: async () => {
        calls += 1;
        if (calls === 1) throw new Error('backend is still starting');
        return browserOverviewWithCapabilities;
      },
      onCapabilities: (next) => capabilities.push(next),
      schedule: scheduler.schedule,
      cancelScheduled: scheduler.cancel,
    });

    await loader.start();
    expect(calls).toBe(1);
    expect(capabilities).toEqual([]);
    expect(scheduler.delays()).toEqual([BROWSER_SETTINGS_OVERVIEW_RETRY_DELAYS_MS[0]]);

    await scheduler.runNext();
    expect(calls).toBe(2);
    expect(capabilities).toEqual([
      {
        canCloseAll: false,
        canManageBrowserSettings: true,
        canManagePrimaryIdentity: true,
      },
    ]);
    expect(scheduler.delays()).toEqual([]);
    loader.dispose();
  });

  test('bounds failed overview attempts and coalesces reload bursts', async () => {
    const scheduler = createManualScheduler();
    let calls = 0;
    const loader = createBrowserSettingsCapabilityLoader({
      invoke: async () => {
        calls += 1;
        throw new Error('temporary overview failure');
      },
      onCapabilities: () => {},
      schedule: scheduler.schedule,
      cancelScheduled: scheduler.cancel,
    });

    await loader.start();
    for (const expectedDelay of BROWSER_SETTINGS_OVERVIEW_RETRY_DELAYS_MS) {
      expect(scheduler.delays()).toEqual([expectedDelay]);
      await scheduler.runNext();
    }
    expect(calls).toBe(BROWSER_SETTINGS_OVERVIEW_RETRY_DELAYS_MS.length + 1);
    expect(scheduler.delays()).toEqual([]);

    let resolveOverview!: (value: typeof browserOverviewWithCapabilities) => void;
    const pendingOverview = new Promise<typeof browserOverviewWithCapabilities>((resolve) => {
      resolveOverview = resolve;
    });
    let reloadCalls = 0;
    const reloadLoader = createBrowserSettingsCapabilityLoader({
      invoke: () => {
        reloadCalls += 1;
        return reloadCalls === 1 ? pendingOverview : Promise.resolve(browserOverviewWithCapabilities);
      },
      onCapabilities: () => {},
      schedule: scheduler.schedule,
      cancelScheduled: scheduler.cancel,
    });
    const initialLoad = reloadLoader.start();
    const reloads = Array.from({ length: 20 }, () => reloadLoader.reload());
    expect(reloadCalls).toBe(1);
    resolveOverview(browserOverviewWithCapabilities);
    await Promise.all([initialLoad, ...reloads]);
    expect(reloadCalls).toBe(2);
    reloadLoader.dispose();
    loader.dispose();
  });

  test('cancels overview retry work and never publishes capabilities after disposal', async () => {
    const scheduler = createManualScheduler();
    const capabilities: unknown[] = [];
    let resolveOverview!: (value: typeof browserOverviewWithCapabilities) => void;
    const loader = createBrowserSettingsCapabilityLoader({
      invoke: () =>
        new Promise<typeof browserOverviewWithCapabilities>((resolve) => {
          resolveOverview = resolve;
        }),
      onCapabilities: (next) => capabilities.push(next),
      schedule: scheduler.schedule,
      cancelScheduled: scheduler.cancel,
    });

    const load = loader.start();
    loader.dispose();
    resolveOverview(browserOverviewWithCapabilities);
    await load;
    expect(capabilities).toEqual([]);
    expect(scheduler.delays()).toEqual([]);

    const retryLoader = createBrowserSettingsCapabilityLoader({
      invoke: async () => {
        throw new Error('retry me');
      },
      onCapabilities: () => {},
      schedule: scheduler.schedule,
      cancelScheduled: scheduler.cancel,
    });
    await retryLoader.start();
    expect(scheduler.delays()).toHaveLength(1);
    retryLoader.dispose();
    expect(scheduler.delays()).toEqual([]);
  });

  test('matches the backend resource-policy boundaries and cross-constraint', () => {
    expect(BROWSER_RESOURCE_POLICY_LIMITS).toEqual({
      max_memory_ratio: { min: 0.1, max: 0.8 },
      reserved_memory_bytes: { min: 256 * 1024 * 1024, max: 512 * 1024 * 1024 * 1024 },
      max_active_operations: { min: 1, max: 64 },
      max_open_lanes: { min: 1, max: 128 },
      max_queued_requests: { min: 1, max: 256 },
      max_owner_queued_requests: { min: 1, max: 32 },
    });

    const valid = {
      preset: 'automatic' as const,
      advanced: {
        max_memory_ratio: 0.1,
        reserved_memory_bytes: 256 * 1024 * 1024,
        max_active_operations: 1,
        max_open_lanes: 1,
        max_queued_requests: 1,
        max_owner_queued_requests: 1,
      },
    };
    expect(validateBrowserResourcePolicy(valid)).toBeNull();
    expect(
      validateBrowserResourcePolicy({
        ...valid,
        advanced: {
          ...valid.advanced,
          max_memory_ratio: 0.8,
          reserved_memory_bytes: 512 * 1024 * 1024 * 1024,
          max_active_operations: 64,
          max_open_lanes: 128,
          max_queued_requests: 256,
          max_owner_queued_requests: 32,
        },
      })
    ).toBeNull();

    const boundaryCases = [
      ['max_memory_ratio', 0.099],
      ['max_memory_ratio', 0.801],
      ['reserved_memory_bytes', 256 * 1024 * 1024 - 1],
      ['reserved_memory_bytes', 512 * 1024 * 1024 * 1024 + 1],
      ['max_active_operations', 0],
      ['max_active_operations', 65],
      ['max_open_lanes', 0],
      ['max_open_lanes', 129],
      ['max_queued_requests', 0],
      ['max_queued_requests', 257],
      ['max_owner_queued_requests', 0],
      ['max_owner_queued_requests', 33],
    ] as const;
    for (const [field, value] of boundaryCases) {
      expect(
        validateBrowserResourcePolicy({
          preset: 'automatic',
          advanced: { [field]: value },
        })
      ).not.toBeNull();
    }

    expect(
      validateBrowserResourcePolicy({
        preset: 'automatic',
        advanced: { max_queued_requests: 4, max_owner_queued_requests: 5 },
      })
    ).toEqual({
      field: 'max_owner_queued_requests',
      message: 'max_owner_queued_requests cannot exceed max_queued_requests.',
    });
  });

  test('shows the structured server error instead of hiding it behind a generic save message', () => {
    const error = {
      name: 'BackendHttpError',
      status: 400,
      code: 'invalid_browser_resource_policy',
      backendMessage: 'fallback backend message',
      body: {
        code: 'invalid_browser_resource_policy',
        message: 'max_owner_queued_requests cannot exceed max_queued_requests.',
      },
    };

    expect(browserSettingsServerErrorMessage(error)).toBe(
      'max_owner_queued_requests cannot exceed max_queued_requests.'
    );
    expect(
      readSource(new URL('./BrowserUseSettingsContent.tsx', import.meta.url)).includes('resourcePolicyError')
    ).toBe(true);
  });

  test('keeps both security mutex transitions fail-closed while writing', async () => {
    const cases = [
      {
        previous: { persistentLogin: false, fullPower: true },
        next: { persistentLogin: true, fullPower: false },
        expectedWrites: [
          [FULL_POWER_KEY, false],
          [PERSISTENT_LOGIN_KEY, true],
        ] as Array<[SecurityKey, boolean]>,
      },
      {
        previous: { persistentLogin: true, fullPower: false },
        next: { persistentLogin: false, fullPower: true },
        expectedWrites: [
          [PERSISTENT_LOGIN_KEY, false],
          [FULL_POWER_KEY, true],
        ] as Array<[SecurityKey, boolean]>,
      },
    ] as const;

    for (const { previous, next, expectedWrites } of cases) {
      let values: SecurityValues = {
        [PERSISTENT_LOGIN_KEY]: previous.persistentLogin,
        [FULL_POWER_KEY]: previous.fullPower,
      };
      const writes: Array<[SecurityKey, boolean]> = [];
      let reloadCalls = 0;
      const saved = await persistBrowserSecuritySettingsTransaction(
        {
          get: (key) => values[key],
          set: async (key, value) => {
            writes.push([key, value]);
            values[key] = value;
          },
          setLocal: (key, value) => {
            values[key] = value;
          },
          reload: async () => {
            reloadCalls += 1;
          },
          isInitialized: () => true,
        },
        previous,
        next
      );

      expect(writes).toEqual(expectedWrites);
      expect(reloadCalls).toBe(0);
      expect(saved).toEqual(next);
      expect(values).toEqual({
        [PERSISTENT_LOGIN_KEY]: next.persistentLogin,
        [FULL_POWER_KEY]: next.fullPower,
      });
    }
  });

  test('rolls back the first security write when the second write fails', async () => {
    const cases = [
      {
        previous: { persistentLogin: false, fullPower: true },
        next: { persistentLogin: true, fullPower: false },
        expectedWrites: [
          [FULL_POWER_KEY, false],
          [PERSISTENT_LOGIN_KEY, true],
          [FULL_POWER_KEY, true],
        ] as Array<[SecurityKey, boolean]>,
      },
      {
        previous: { persistentLogin: true, fullPower: false },
        next: { persistentLogin: false, fullPower: true },
        expectedWrites: [
          [PERSISTENT_LOGIN_KEY, false],
          [FULL_POWER_KEY, true],
          [PERSISTENT_LOGIN_KEY, true],
        ] as Array<[SecurityKey, boolean]>,
      },
    ] as const;

    for (const { previous, next, expectedWrites } of cases) {
      const authoritative: SecurityValues = {
        [PERSISTENT_LOGIN_KEY]: previous.persistentLogin,
        [FULL_POWER_KEY]: previous.fullPower,
      };
      let values = cloneSecurityValues(authoritative);
      const writes: Array<[SecurityKey, boolean]> = [];
      let setCalls = 0;
      let reloadCalls = 0;
      const error = await captureError(() =>
        persistBrowserSecuritySettingsTransaction(
          {
            get: (key) => values[key],
            set: async (key, value) => {
              setCalls += 1;
              writes.push([key, value]);
              // Match configService.set(): update the cache before awaiting the
              // request, so a rejected write still leaves optimistic local data.
              values[key] = value;
              if (setCalls === 2) throw new Error('second step failed');
            },
            setLocal: (key, value) => {
              values[key] = value;
            },
            reload: async () => {
              reloadCalls += 1;
              values = cloneSecurityValues(authoritative);
            },
            isInitialized: () => true,
          },
          previous,
          next
        )
      );

      expect(error instanceof Error).toBe(true);
      expect((error as Error).message).toBe('second step failed');
      expect(writes).toEqual(expectedWrites);
      expect(reloadCalls).toBe(1);
      expect(values).toEqual(authoritative);
    }
  });

  test('uses the authoritative security state when reload succeeds after a failed write', async () => {
    const previous = { persistentLogin: false, fullPower: true };
    const authoritative: SecurityValues = {
      [PERSISTENT_LOGIN_KEY]: previous.persistentLogin,
      [FULL_POWER_KEY]: previous.fullPower,
    };
    let values = cloneSecurityValues(authoritative);
    const writes: Array<[SecurityKey, boolean]> = [];
    let setCalls = 0;
    let reloadCalls = 0;
    const error = await captureError(() =>
      persistBrowserSecuritySettingsTransaction(
        {
          get: (key) => values[key],
          set: async (key, value) => {
            setCalls += 1;
            writes.push([key, value]);
            values[key] = value;
            if (setCalls === 1) {
              authoritative[key] = value;
              return;
            }
            if (setCalls === 2) {
              // The server committed the second write but the response was
              // lost. The subsequent rollback also fails, so reload is the
              // only source of truth for the final state.
              authoritative[key] = value;
              throw new Error('response lost');
            }
            throw new Error('rollback failed');
          },
          setLocal: (key, value) => {
            values[key] = value;
          },
          reload: async () => {
            reloadCalls += 1;
            values = cloneSecurityValues(authoritative);
          },
          isInitialized: () => true,
        },
        previous,
        { persistentLogin: true, fullPower: false }
      )
    );

    expect(error instanceof Error).toBe(true);
    expect((error as Error).message).toBe('response lost');
    expect(writes).toEqual([
      [FULL_POWER_KEY, false],
      [PERSISTENT_LOGIN_KEY, true],
      [FULL_POWER_KEY, true],
    ]);
    expect(reloadCalls).toBe(1);
    expect(values).toEqual({
      [PERSISTENT_LOGIN_KEY]: true,
      [FULL_POWER_KEY]: false,
    });
  });

  test('restores the previous local security state when reload fails', async () => {
    for (const reloadFailure of ['rejects', 'uninitialized'] as const) {
      const previous = { persistentLogin: true, fullPower: false };
      let values: SecurityValues = {
        [PERSISTENT_LOGIN_KEY]: previous.persistentLogin,
        [FULL_POWER_KEY]: previous.fullPower,
      };
      let setCalls = 0;
      let initialized = true;
      const error = await captureError(() =>
        persistBrowserSecuritySettingsTransaction(
          {
            get: (key) => values[key],
            set: async (key, value) => {
              setCalls += 1;
              values[key] = value;
              if (setCalls === 2) throw new Error('second step failed');
            },
            setLocal: (key, value) => {
              values[key] = value;
            },
            reload: async () => {
              values = {
                [PERSISTENT_LOGIN_KEY]: false,
                [FULL_POWER_KEY]: false,
              };
              if (reloadFailure === 'rejects') {
                throw new Error('reload failed');
              }
              initialized = false;
            },
            isInitialized: () => initialized,
          },
          previous,
          { persistentLogin: false, fullPower: true }
        )
      );

      expect(error instanceof Error).toBe(true);
      expect((error as Error).message).toBe('second step failed');
      expect(values).toEqual({
        [PERSISTENT_LOGIN_KEY]: previous.persistentLogin,
        [FULL_POWER_KEY]: previous.fullPower,
      });
    }
  });

  test('does not tie initialization or resource loading to translation function identity', () => {
    const source = readSource(new URL('./BrowserUseSettingsContent.tsx', import.meta.url));
    const componentStart = source.indexOf('const BrowserUseSettingsContent');
    const initializationMarker = source.indexOf('const storedPersistentLogin', componentStart);
    const initializationStart = source.lastIndexOf('useEffect(() => {', initializationMarker);
    const initializationEnd =
      source.indexOf('}, []);', initializationMarker) + '}, []);'.length;
    const loadCallbackStart = source.indexOf('const loadResourcePolicy', initializationMarker);
    const loadEffectStart = source.indexOf('useEffect(() => {', loadCallbackStart);
    const loginStatusMarker = source.indexOf(
      '// Reflect whether the managed Primary sign-in Lane is already open.',
      loadEffectStart
    );

    expect(componentStart).toBeGreaterThan(-1);
    expect(initializationMarker).toBeGreaterThan(-1);
    expect(initializationStart).toBeGreaterThan(-1);
    expect(loadCallbackStart).toBeGreaterThan(initializationStart);
    expect(loadEffectStart).toBeGreaterThan(loadCallbackStart);
    expect(loginStatusMarker).toBeGreaterThan(loadEffectStart);

    const initializationEffect = source.slice(initializationStart, initializationEnd).trim();
    const loadResourcePolicyCallback = source.slice(loadCallbackStart, loadEffectStart).trim();
    const loadResourcePolicyEffect = source.slice(loadEffectStart, loginStatusMarker).trim();

    expect(/},\s*\[\]\);$/.test(initializationEffect)).toBe(true);
    expect(/\bt\(/.test(initializationEffect)).toBe(false);
    expect(initializationEffect.includes('translationRef.current')).toBe(true);

    expect(/},\s*\[\]\);$/.test(loadResourcePolicyCallback)).toBe(true);
    expect(/\bt\(/.test(loadResourcePolicyCallback)).toBe(false);
    expect(
      loadResourcePolicyCallback.includes(
        "fallbackKey: 'settings.browserResourcePolicyLoadFailed'"
      )
    ).toBe(true);

    expect(
      /},\s*\[canManageBrowserSettings,\s*loadResourcePolicy\]\);$/.test(
        loadResourcePolicyEffect
      )
    ).toBe(true);
    expect(
      source.includes('resourcePolicyError.message || t(resourcePolicyError.fallbackKey)')
    ).toBe(true);
  });

  test('keeps Browser settings locale coverage aligned in English and Chinese', () => {
    const en = JSON.parse(
      readSource(new URL('../../../../services/i18n/locales/en-US/settings.json', import.meta.url))
    );
    const zh = JSON.parse(
      readSource(new URL('../../../../services/i18n/locales/zh-CN/settings.json', import.meta.url))
    );
    const requiredKeys = [
      'browserDisplayMode',
      'browserDisplayModeDesc',
      'browserDisplayModeHeadless',
      'browserDisplayModeExternal',
      'browserDisplayModeSaved',
      'browserDisplayModeUnconfirmed',
      'browserDisplayModeUnavailable',
      'browserDisplayModeLoadFailedWithDetails',
      'browserResourcePolicy',
      'browserResourcePolicyAutomatic',
      'browserResourcePolicySaving',
      'browserResourcePolicyHighConcurrency',
      'browserResourcePolicyAdvanced',
    ];

    for (const key of requiredKeys) {
      expect(en[key]).toBeTruthy();
      expect(zh[key]).toBeTruthy();
    }
  });
});
