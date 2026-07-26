/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { describe, expect, test } from 'bun:test';

import { tauriInstallUpdate } from './tauriShell';

const originalWindow = globalThis.window;

const restoreWindow = (): void => {
  if (typeof originalWindow === 'undefined') {
    Reflect.deleteProperty(globalThis, 'window');
  } else {
    Object.defineProperty(globalThis, 'window', {
      configurable: true,
      value: originalWindow,
    });
  }
};

const withTauriInternals = async (
  invoke: (command: string, args: unknown, options: unknown) => Promise<unknown>,
  run: () => Promise<void>
): Promise<void> => {
  Object.defineProperty(globalThis, 'window', {
    configurable: true,
    value: { __TAURI_INTERNALS__: { invoke } },
  });

  try {
    await run();
  } finally {
    restoreWindow();
  }
};

describe('tauriInstallUpdate', () => {
  test('invokes the Rust-owned updater command with the selected version', async () => {
    const calls: Array<{ command: string; args: unknown; options: unknown }> = [];
    await withTauriInternals(async (command, args, options) => {
      calls.push({ command, args, options });
    }, async () => {
      await tauriInstallUpdate('1.2.3');
    });

    expect(calls).toEqual([{ command: 'install_update', args: { version: '1.2.3' }, options: undefined }]);
  });

  test('propagates native installation failures', async () => {
    let errorMessage = '';
    await withTauriInternals(async () => {
      throw new Error('native updater failed');
    }, async () => {
      try {
        await tauriInstallUpdate('1.2.3');
      } catch (error) {
        errorMessage = error instanceof Error ? error.message : String(error);
      }
    });

    expect(errorMessage).toBe('native updater failed');
  });
});
