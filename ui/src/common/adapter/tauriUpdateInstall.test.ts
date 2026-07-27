/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { describe, expect, test } from 'bun:test';
import {
  AUTO_INSTALL_UNSUPPORTED_ERROR,
  installUpdateWithPreflight,
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
});
