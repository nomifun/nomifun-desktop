/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { describe, expect, test } from 'bun:test';
import {
  BackendHttpError,
  BackendRequestError,
} from '@/common/adapter/httpBridge';
import {
  createBrowserDisplayModeController,
  isBrowserDisplayModePersistenceFailure,
} from './browserDisplayModeController';
import type { BrowserDisplayMode } from './browserTypes';

const deferred = <T>() => {
  let resolve!: (value: T) => void;
  let reject!: (error: unknown) => void;
  const promise = new Promise<T>((resolvePromise, rejectPromise) => {
    resolve = resolvePromise;
    reject = rejectPromise;
  });
  return { promise, resolve, reject };
};

describe('Browser display-mode controller', () => {
  test('invalidates an older GET when a save starts', async () => {
    const oldGet = deferred<{ display_mode: BrowserDisplayMode }>();
    let mode: BrowserDisplayMode = 'headless';
    let getCalls = 0;
    const controller = createBrowserDisplayModeController({
      get: () => {
        getCalls += 1;
        return getCalls === 1
          ? oldGet.promise
          : Promise.resolve({ display_mode: mode });
      },
      put: async (next) => {
        mode = next;
        return { display_mode: next };
      },
    });

    const loading = controller.load();
    const saving = controller.save('external');
    oldGet.resolve({ display_mode: 'headless' });

    expect(await loading).toEqual({ kind: 'stale' });
    expect(await saving).toEqual({
      kind: 'applied',
      displayMode: 'external',
      recoveredLostResponse: false,
    });
  });

  test('never claims persistence after network or timeout response loss', async () => {
    for (const kind of ['network', 'timeout'] as const) {
      let mode: BrowserDisplayMode = 'headless';
      const responseLoss = new BackendRequestError(
        kind,
        `${kind} response lost`
      );
      const controller = createBrowserDisplayModeController({
        get: async () => ({ display_mode: mode }),
        put: async (next) => {
          mode = next;
          throw responseLoss;
        },
      });

      expect(await controller.save('external')).toEqual({
        kind: 'rejected',
        displayMode: 'external',
        error: responseLoss,
        unconfirmed: true,
        nonPersistent: false,
      });
    }
  });

  test('never recovers an explicit storage failure from matching live state', async () => {
    const storageError = new BackendHttpError({
      method: 'PUT',
      path: '/api/browser/display-mode',
      status: 500,
      body: {
        code: 'browser_display_mode_storage_failed',
        message: 'The browser display mode could not be saved.',
      },
    });
    const controller = createBrowserDisplayModeController({
      // Simulates storage failure followed by a failed live rollback: the Hub
      // still reports the requested value, but the v2 preference was not saved.
      get: async () => ({ display_mode: 'external' }),
      put: async () => {
        throw storageError;
      },
    });

    expect(isBrowserDisplayModePersistenceFailure(storageError)).toBe(true);
    expect(await controller.save('external')).toEqual({
      kind: 'rejected',
      displayMode: 'external',
      error: storageError,
      unconfirmed: true,
      nonPersistent: true,
    });
  });

  test('does not classify generic 5xx or response loss as an explicit storage failure', () => {
    const genericServerError = new BackendHttpError({
      method: 'PUT',
      path: '/api/browser/display-mode',
      status: 500,
      body: {
        code: 'browser_unavailable',
        message: 'Browser unavailable.',
      },
    });

    expect(isBrowserDisplayModePersistenceFailure(genericServerError)).toBe(false);
    expect(
      isBrowserDisplayModePersistenceFailure(
        new BackendRequestError('network', 'response lost')
      )
    ).toBe(false);
  });

  test('does not recover a structured backend rejection from matching live state', async () => {
    const backendRejection = new BackendHttpError({
      method: 'PUT',
      path: '/api/browser/display-mode',
      status: 503,
      body: {
        code: 'browser_unavailable',
        message: 'Browser unavailable.',
      },
    });
    const controller = createBrowserDisplayModeController({
      get: async () => ({ display_mode: 'external' }),
      put: async () => {
        throw backendRejection;
      },
    });

    expect(await controller.save('external')).toEqual({
      kind: 'rejected',
      displayMode: 'external',
      error: backendRejection,
      unconfirmed: false,
      nonPersistent: false,
    });
  });

  test('does not recover a mismatched successful PUT from a coincidental GET', async () => {
    const controller = createBrowserDisplayModeController({
      get: async () => ({ display_mode: 'external' }),
      put: async () => ({ display_mode: 'headless' }),
    });

    const result = await controller.save('external');
    expect(result.kind).toBe('rejected');
    expect(result.kind === 'rejected' && result.displayMode).toBe('external');
    expect(result.kind === 'rejected' && result.unconfirmed).toBe(true);
    expect(result.kind === 'rejected' && result.nonPersistent).toBe(false);
  });

  test('does not recover an unknown client error from a coincidental GET', async () => {
    const unknownError = new Error('response mapper failed');
    const controller = createBrowserDisplayModeController({
      get: async () => ({ display_mode: 'external' }),
      put: async () => {
        throw unknownError;
      },
    });

    expect(await controller.save('external')).toEqual({
      kind: 'rejected',
      displayMode: 'external',
      error: unknownError,
      unconfirmed: false,
      nonPersistent: false,
    });
  });

  test('uses authoritative state instead of blindly rolling back', async () => {
    const controller = createBrowserDisplayModeController({
      get: async () => ({ display_mode: 'headless' }),
      put: async () => {
        throw new Error('write rejected');
      },
    });

    const result = await controller.save('external');
    expect(result.kind).toBe('rejected');
    expect(result.kind === 'rejected' && result.displayMode).toBe('headless');
    expect(
      result.kind === 'rejected' &&
        result.error instanceof Error &&
        result.error.message === 'write rejected'
    ).toBe(true);
    expect(result.kind === 'rejected' && result.unconfirmed).toBe(false);
    expect(result.kind === 'rejected' && result.nonPersistent).toBe(false);
  });

  test('marks a mismatched successful response as unconfirmed', async () => {
    const controller = createBrowserDisplayModeController({
      get: async () => ({ display_mode: 'headless' }),
      put: async () => ({ display_mode: 'headless' }),
    });

    const result = await controller.save('external');
    expect(result.kind).toBe('rejected');
    expect(result.kind === 'rejected' && result.unconfirmed).toBe(true);
    expect(result.kind === 'rejected' && result.displayMode).toBe('headless');
    expect(result.kind === 'rejected' && result.nonPersistent).toBe(false);
  });

  test('does not claim a mode when both PUT and authoritative reload fail', async () => {
    const controller = createBrowserDisplayModeController({
      get: async () => {
        throw new Error('reload unavailable');
      },
      put: async () => {
        throw new Error('write response lost');
      },
    });

    const result = await controller.save('external');
    expect(result.kind).toBe('unknown');
    expect(
      result.kind === 'unknown' &&
        result.error instanceof Error &&
        result.error.message.includes('authoritative reload failed')
    ).toBe(true);
  });

  test('defers reconnect loads while a save owns the reconciliation flight', async () => {
    const put = deferred<{ display_mode: BrowserDisplayMode }>();
    const controller = createBrowserDisplayModeController({
      get: async () => ({ display_mode: 'external' }),
      put: () => put.promise,
    });

    const saving = controller.save('external');
    expect(await controller.load()).toEqual({ kind: 'deferred' });
    put.resolve({ display_mode: 'external' });
    expect((await saving).kind).toBe('applied');
  });
});
