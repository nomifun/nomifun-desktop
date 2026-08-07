/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { describe, expect, test } from 'bun:test';
import {
  AUTO_INSTALL_UNSUPPORTED_ERROR,
  INSTALL_NOT_ATTEMPTED_ERROR,
  installUpdateWithPreflight,
  isInstallNotAttempted,
  type UpdaterInstallContext,
} from './tauriUpdateInstall';

const safe: UpdaterInstallContext = {
  platform: 'macos',
  appBundlePath: '/Applications/NomiFun.app',
  tempDir: '/private/var/folders/tmp',
  appDeviceId: 7,
  tempDeviceId: 7,
  autoInstallSupported: true,
  reason: null,
};

describe('installUpdateWithPreflight', () => {
  test('safe context prepares shutdown, installs, and then relaunches in order', async () => {
    const calls: string[] = [];

    await installUpdateWithPreflight({
      getContext: async () => {
        calls.push('getContext');
        return safe;
      },
      prepareShutdown: async () => void calls.push('prepareShutdown'),
      install: async () => void calls.push('install'),
      relaunch: async () => void calls.push('relaunch'),
      fatalExit: async () => {
        throw new Error('fatalExit must not be called');
      },
    });

    expect(calls).toEqual(['getContext', 'prepareShutdown', 'install', 'relaunch']);
  });

  test('unsafe context never prepares shutdown, installs, or relaunches', async () => {
    const calls: string[] = [];
    const result = installUpdateWithPreflight({
      getContext: async () => {
        calls.push('getContext');
        return { ...safe, autoInstallSupported: false, reason: 'mounted_volume' };
      },
      prepareShutdown: async () => void calls.push('prepareShutdown'),
      install: async () => void calls.push('install'),
      relaunch: async () => void calls.push('relaunch'),
      fatalExit: async () => {
        throw new Error('fatalExit must not be called');
      },
    });

    let errorMessage = '';
    try {
      await result;
    } catch (error) {
      errorMessage = error instanceof Error ? error.message : String(error);
    }

    expect(errorMessage).toBe(`${AUTO_INSTALL_UNSUPPORTED_ERROR}:mounted_volume`);
    expect(calls).toEqual(['getContext']);
  });

  test('prepare shutdown failure prevents install and relaunch', async () => {
    const calls: string[] = [];
    const result = installUpdateWithPreflight({
      getContext: async () => {
        calls.push('getContext');
        return safe;
      },
      prepareShutdown: async () => {
        calls.push('prepareShutdown');
        throw new Error('cleanup failed');
      },
      install: async () => void calls.push('install'),
      relaunch: async () => void calls.push('relaunch'),
      fatalExit: async () => {
        throw new Error('fatalExit must not be called');
      },
    });

    let errorMessage = '';
    try {
      await result;
    } catch (error) {
      errorMessage = error instanceof Error ? error.message : String(error);
    }

    expect(errorMessage).toBe('cleanup failed');
    expect(calls).toEqual(['getContext', 'prepareShutdown']);
  });

  test('install failure after cleanup takes the fatal exit path and never relaunches', async () => {
    const calls: string[] = [];
    const installError = new Error('install failed');
    const fatalExitSentinel = new Error('fatal exit invoked');
    let thrown: unknown;

    try {
      await installUpdateWithPreflight({
        getContext: async () => {
          calls.push('getContext');
          return safe;
        },
        prepareShutdown: async () => void calls.push('prepareShutdown'),
        install: async () => {
          calls.push('install');
          throw installError;
        },
        relaunch: async () => void calls.push('relaunch'),
        fatalExit: async (failure) => {
          calls.push(`fatalExit:${failure.phase}`);
          expect(failure.error).toBe(installError);
          throw fatalExitSentinel;
        },
      });
    } catch (error) {
      thrown = error;
    }

    expect(thrown).toBe(fatalExitSentinel);
    expect(calls).toEqual(['getContext', 'prepareShutdown', 'install', 'fatalExit:install']);
  });

  test('relaunch failure after cleanup takes the fatal exit path', async () => {
    const calls: string[] = [];
    const relaunchError = new Error('relaunch failed');
    const fatalExitSentinel = new Error('fatal exit invoked');
    let thrown: unknown;

    try {
      await installUpdateWithPreflight({
        getContext: async () => {
          calls.push('getContext');
          return safe;
        },
        prepareShutdown: async () => void calls.push('prepareShutdown'),
        install: async () => void calls.push('install'),
        relaunch: async () => {
          calls.push('relaunch');
          throw relaunchError;
        },
        fatalExit: async (failure) => {
          calls.push(`fatalExit:${failure.phase}`);
          expect(failure.error).toBe(relaunchError);
          throw fatalExitSentinel;
        },
      });
    } catch (error) {
      thrown = error;
    }

    expect(thrown).toBe(fatalExitSentinel);
    expect(calls).toEqual(['getContext', 'prepareShutdown', 'install', 'relaunch', 'fatalExit:relaunch']);
  });

  test('does not return to the renderer if the fatal exit adapter unexpectedly returns', async () => {
    const calls: string[] = [];
    const result = installUpdateWithPreflight({
      getContext: async () => safe,
      prepareShutdown: async () => void calls.push('prepareShutdown'),
      install: async () => {
        calls.push('install');
        throw new Error('install failed');
      },
      relaunch: async () => void calls.push('relaunch'),
      fatalExit: async () => {
        calls.push('fatalExit');
        return undefined as never;
      },
    });

    const outcome = await Promise.race([
      result.then(() => 'returned' as const),
      new Promise<'still-terminal'>((resolve) => setTimeout(() => resolve('still-terminal'), 10)),
    ]);

    expect(outcome).toBe('still-terminal');
    expect(calls).toEqual(['prepareShutdown', 'install', 'fatalExit']);
  });

  // The native side refuses an install it never started — no retained package for
  // this version, or a download for it is still running — WITHOUT touching the
  // installed app, and it restores whatever it held before returning. Treating
  // that as a post-handoff fatal used to kill the process: the user lost the
  // in-memory package and had to download the whole installer again just to
  // retry — the exact "must download a second time before install works" report.
  // An install ALREADY IN FLIGHT is deliberately excluded (see below).
  test('an install refusal that never reached the installer is recoverable, not fatal', async () => {
    const calls: string[] = [];
    const refusal = new Error(`${INSTALL_NOT_ATTEMPTED_ERROR}: update 0.4.2 has not been downloaded`);
    let thrown: unknown;

    try {
      await installUpdateWithPreflight({
        getContext: async () => safe,
        prepareShutdown: async () => void calls.push('prepareShutdown'),
        install: async () => {
          calls.push('install');
          throw refusal;
        },
        relaunch: async () => void calls.push('relaunch'),
        fatalExit: async () => {
          calls.push('fatalExit');
          return undefined as never;
        },
      });
    } catch (error) {
      thrown = error;
    }

    expect(thrown).toBe(refusal);
    // Never relaunches, and crucially never exits: the app is intact.
    expect(calls).toEqual(['prepareShutdown', 'install']);
  });

  test('an install already in flight keeps the fail-closed exit', async () => {
    // SAFETY BOUNDARY. The native side leaves this one UNMARKED on purpose: by
    // then the handoff has begun and on macOS the running .app may already have
    // been renamed aside, so returning to the renderer is not safe.
    const calls: string[] = [];
    const inFlight = new Error('update 0.4.2 is already installing');

    const result = installUpdateWithPreflight({
      getContext: async () => safe,
      prepareShutdown: async () => void calls.push('prepareShutdown'),
      install: async () => {
        calls.push('install');
        throw inFlight;
      },
      relaunch: async () => void calls.push('relaunch'),
      fatalExit: async () => {
        calls.push('fatalExit');
        return undefined as never;
      },
    });

    const outcome = await Promise.race([
      result.then(() => 'returned' as const).catch(() => 'threw' as const),
      new Promise<'still-terminal'>((resolve) => setTimeout(() => resolve('still-terminal'), 10)),
    ]);

    expect(outcome).toBe('still-terminal');
    expect(calls).toEqual(['prepareShutdown', 'install', 'fatalExit']);
  });
});

describe('isInstallNotAttempted', () => {
  test('recognises the native marker as a prefix', () => {
    expect(isInstallNotAttempted(new Error(`${INSTALL_NOT_ATTEMPTED_ERROR}: whatever`))).toBe(true);
    expect(isInstallNotAttempted(`${INSTALL_NOT_ATTEMPTED_ERROR}: raw string rejection`)).toBe(true);
  });

  test('a real install failure is NOT recoverable — the bundle may be half replaced', () => {
    // macOS install_inner renames the running bundle aside before moving the new
    // one into place; a failure past that point can leave no app at all, so the
    // fail-closed exit must stay for anything without the marker.
    expect(isInstallNotAttempted(new Error('failed to move the new app into place'))).toBe(false);
    expect(isInstallNotAttempted(new Error('update 0.4.2 is already installing'))).toBe(false);
    expect(isInstallNotAttempted(undefined)).toBe(false);
    expect(isInstallNotAttempted(null)).toBe(false);
  });

  test('a message that merely quotes the marker does not become recoverable', () => {
    // The safety decision must not be invertible by string concatenation: the
    // native side emits the marker strictly as a prefix.
    expect(isInstallNotAttempted(new Error(`install failed after ${INSTALL_NOT_ATTEMPTED_ERROR} retry`))).toBe(
      false
    );
  });
});
