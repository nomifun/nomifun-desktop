/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { describe, expect, test } from 'bun:test';

import {
  parseDeepLink,
  tauriDownloadUpdate,
  tauriInstallUpdate,
  type TauriDownloadUpdateProgress,
} from './tauriShell';

describe('deep-link parsing', () => {
  test('keeps only non-sensitive provider and model suggestions', () => {
    const query = new URLSearchParams({
      base_url: 'https://api.example.com/v1',
      name: 'Example provider',
      platform: 'custom',
      model: 'model-1',
      task: 'speech_synthesis',
      api_key: 'must-not-travel',
      key: 'must-not-travel-either',
      token: 'also-secret',
    });

    expect(parseDeepLink(`nomifun://add-provider?${query.toString()}`)).toEqual({
      action: 'add-provider',
      params: {
        base_url: 'https://api.example.com/v1',
        name: 'Example provider',
        platform: 'custom',
        model: 'model-1',
        task: 'speech_synthesis',
      },
    });
  });

  test('drops nested URL credentials and action-foreign parameters', () => {
    const query = new URLSearchParams({
      base_url: 'https://user:secret@api.example.com/v1?key=secret#fragment',
      route: '/conversation/not-a-provider-field',
      api_key: 'secret',
    });

    expect(parseDeepLink(`nomifun://add-provider?${query.toString()}`)).toEqual({
      action: 'add-provider',
      params: {},
    });
    expect(parseDeepLink('nomifun://navigate?route=%2Fconversation%2Fid%3Ftoken%3Dsecret')).toEqual({
      action: 'navigate',
      params: {},
    });
  });
});

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
    value: {
      __TAURI_INTERNALS__: {
        invoke,
        transformCallback: () => 1,
        unregisterCallback: () => {},
      },
    },
  });

  try {
    await run();
  } finally {
    restoreWindow();
  }
};

describe('native update commands', () => {
  test('download invokes the Rust-owned command and forwards progress', async () => {
    const calls: Array<{ command: string; args: unknown; options: unknown }> = [];
    const events: TauriDownloadUpdateProgress[] = [];
    await withTauriInternals(async (command, args, options) => {
      calls.push({ command, args, options });
      const payload = args as {
        onEvent: { onmessage: (event: TauriDownloadUpdateProgress) => void };
      };
      payload.onEvent.onmessage({ phase: 'downloading', chunkLength: 64, contentLength: 128 });
    }, async () => {
      await tauriDownloadUpdate('1.2.3', (event) => events.push(event));
    });

    expect(calls).toHaveLength(1);
    expect(calls[0]?.command).toBe('download_update');
    expect((calls[0]?.args as { version: string }).version).toBe('1.2.3');
    expect(calls[0]?.options).toBeUndefined();
    expect(events).toEqual([{ phase: 'downloading', chunkLength: 64, contentLength: 128 }]);
  });

  test('install invokes a separate command with no download channel', async () => {
    const calls: Array<{ command: string; args: unknown; options: unknown }> = [];
    await withTauriInternals(async (command, args, options) => {
      calls.push({ command, args, options });
    }, async () => {
      await tauriInstallUpdate('1.2.3');
    });

    expect(calls).toEqual([
      { command: 'install_update', args: { version: '1.2.3' }, options: undefined },
    ]);
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
