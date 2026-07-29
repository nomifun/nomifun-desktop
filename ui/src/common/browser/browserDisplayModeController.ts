/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import {
  isBackendHttpError,
  isBackendRequestError,
} from '@/common/adapter/httpBridge';
import type {
  BrowserDisplayMode,
  IBrowserDisplayModePolicy,
} from './browserTypes';

export interface BrowserDisplayModeApi {
  get: () => Promise<IBrowserDisplayModePolicy>;
  put: (displayMode: BrowserDisplayMode) => Promise<IBrowserDisplayModePolicy>;
}

export type BrowserDisplayModeLoadResult =
  | { kind: 'applied'; displayMode: BrowserDisplayMode }
  | { kind: 'deferred' | 'stale' }
  | { kind: 'error'; error: unknown };

export type BrowserDisplayModeSaveResult =
  | {
      kind: 'applied';
      displayMode: BrowserDisplayMode;
      recoveredLostResponse: boolean;
      verificationError?: unknown;
    }
  | {
      kind: 'rejected';
      displayMode: BrowserDisplayMode;
      error: unknown;
      unconfirmed: boolean;
      /** The backend explicitly reported that the preference was not persisted. */
      nonPersistent: boolean;
    }
  | { kind: 'busy' | 'stale' }
  | { kind: 'unknown'; error: unknown };

export interface BrowserDisplayModeController {
  load: () => Promise<BrowserDisplayModeLoadResult>;
  save: (displayMode: BrowserDisplayMode) => Promise<BrowserDisplayModeSaveResult>;
  dispose: () => void;
}

const invalidModeResponse = (expected?: BrowserDisplayMode): Error =>
  new Error(
    expected
      ? `The browser manager did not confirm display mode "${expected}".`
      : 'The browser manager returned an invalid display mode.'
  );

const joinErrors = (primary: unknown, verification: unknown): Error => {
  const first = primary instanceof Error ? primary.message : String(primary);
  const second =
    verification instanceof Error ? verification.message : String(verification);
  return new Error(`${first}; authoritative reload failed: ${second}`);
};

/**
 * A live GET only proves the current Hub state. It cannot turn an explicit
 * preference-storage failure into a successful save, even when rollback also
 * failed and the live state happens to equal the requested mode.
 */
export const isBrowserDisplayModePersistenceFailure = (error: unknown): boolean =>
  isBackendHttpError(error) &&
  error.status >= 500 &&
  error.status < 600 &&
  error.code.toLowerCase() === 'browser_display_mode_storage_failed';

/**
 * Serializes live display-mode writes and invalidates older GET responses.
 *
 * Every save reconciles with a fresh authoritative GET. A transport failure
 * can establish the current live Hub mode, but it cannot prove that the
 * preference write committed. Those saves therefore remain unconfirmed and
 * require an explicit retry instead of being reported as persisted.
 */
export const createBrowserDisplayModeController = (
  api: BrowserDisplayModeApi
): BrowserDisplayModeController => {
  let epoch = 0;
  let saving = false;
  let disposed = false;

  const load = async (): Promise<BrowserDisplayModeLoadResult> => {
    if (disposed) return { kind: 'stale' };
    if (saving) return { kind: 'deferred' };
    const requestEpoch = ++epoch;
    try {
      const policy = await api.get();
      if (disposed || requestEpoch !== epoch) return { kind: 'stale' };
      return { kind: 'applied', displayMode: policy.display_mode };
    } catch (error) {
      if (disposed || requestEpoch !== epoch) return { kind: 'stale' };
      return { kind: 'error', error };
    }
  };

  const save = async (
    requestedMode: BrowserDisplayMode
  ): Promise<BrowserDisplayModeSaveResult> => {
    if (disposed) return { kind: 'stale' };
    if (saving) return { kind: 'busy' };
    saving = true;
    const requestEpoch = ++epoch;
    let putConfirmed = false;
    let putResponseMismatch = false;
    let putError: unknown;

    try {
      const saved = await api.put(requestedMode);
      if (saved.display_mode !== requestedMode) {
        putResponseMismatch = true;
        throw invalidModeResponse(requestedMode);
      }
      putConfirmed = true;
    } catch (error) {
      putError = error;
    }

    let authoritativeMode: BrowserDisplayMode | undefined;
    let verificationError: unknown;
    try {
      const policy = await api.get();
      authoritativeMode = policy.display_mode;
    } catch (error) {
      verificationError = error;
    } finally {
      saving = false;
    }

    if (disposed || requestEpoch !== epoch) return { kind: 'stale' };
    if (authoritativeMode != null) {
      const persistenceFailed =
        isBrowserDisplayModePersistenceFailure(putError);
      const putResponseLost =
        isBackendRequestError(putError) &&
        (putError.kind === 'network' || putError.kind === 'timeout');
      if (authoritativeMode === requestedMode && putConfirmed) {
        return {
          kind: 'applied',
          displayMode: authoritativeMode,
          recoveredLostResponse: false,
        };
      }
      if (authoritativeMode === requestedMode) {
        return {
          kind: 'rejected',
          displayMode: authoritativeMode,
          error: putError ?? invalidModeResponse(requestedMode),
          unconfirmed:
            persistenceFailed || putResponseMismatch || putResponseLost,
          nonPersistent: persistenceFailed,
        };
      }
      return {
        kind: 'rejected',
        displayMode: authoritativeMode,
        error: putError ?? invalidModeResponse(requestedMode),
        unconfirmed:
          persistenceFailed ||
          putResponseMismatch ||
          putResponseLost ||
          putError == null,
        nonPersistent: persistenceFailed,
      };
    }
    if (putConfirmed) {
      return {
        kind: 'applied',
        displayMode: requestedMode,
        recoveredLostResponse: false,
        verificationError,
      };
    }
    return {
      kind: 'unknown',
      error: joinErrors(
        putError ?? invalidModeResponse(requestedMode),
        verificationError ?? invalidModeResponse()
      ),
    };
  };

  return {
    load,
    save,
    dispose: () => {
      disposed = true;
      epoch += 1;
    },
  };
};
