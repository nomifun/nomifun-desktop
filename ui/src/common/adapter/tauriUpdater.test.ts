/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { describe, expect, test } from 'bun:test';
import { readFileSync } from 'node:fs';

const updaterSource = readFileSync(new URL('./tauriUpdater.ts', import.meta.url), 'utf8');
const shellSource = readFileSync(new URL('./tauriShell.ts', import.meta.url), 'utf8');
const desktopSource = readFileSync(
  new URL('../../../../apps/desktop/src/main.rs', import.meta.url),
  'utf8'
);
const capability = JSON.parse(
  readFileSync(new URL('../../../../apps/desktop/capabilities/default.json', import.meta.url), 'utf8')
) as { permissions: string[] };

describe('desktop updater security boundary', () => {
  test('renderer exposes download but no raw updater install path', () => {
    expect(updaterSource.includes('download(onEvent?')).toBe(false);
    expect(updaterSource.includes('install(): Promise<void>')).toBe(false);
    expect(updaterSource.includes('downloadAndInstall')).toBe(false);
    expect(updaterSource.includes('.install(')).toBe(false);
    expect(updaterSource.includes('await tauriInstallUpdate(version)')).toBe(true);
  });

  test('install goes through the fail-closed preflight/fatal-exit contract', () => {
    // installUpdateWithPreflight owns ordering (preflight → cleanup → install →
    // relaunch) and guarantees a fatal exit once install has started; the raw
    // sequential call path must not come back.
    expect(updaterSource.includes('installUpdateWithPreflight({')).toBe(true);
    expect(updaterSource.includes('fatalExit')).toBe(true);
    expect(updaterSource.includes('prepareShutdown')).toBe(true);
    expect(updaterSource.includes('install: async () => {')).toBe(true);
  });

  test('native download owns progress and install consumes only the retained package', () => {
    expect(shellSource.includes('new Channel<TauriDownloadUpdateProgress>(onProgress)')).toBe(true);
    expect(desktopSource.includes('tauri::ipc::Channel<DownloadUpdateProgress>')).toBe(true);
    expect(desktopSource.includes('phase: "downloading"')).toBe(true);
    expect(desktopSource.includes('downloaded.take_ready(&requested_version)?')).toBe(true);
    expect(desktopSource.includes('package.update.install(&package.bytes)')).toBe(true);
    const installStart = desktopSource.indexOf('async fn install_update(');
    const installEnd = desktopSource.indexOf('const UPDATER_SHUTDOWN_MAX_ATTEMPTS', installStart);
    const installCommand = desktopSource.slice(installStart, installEnd);
    expect(installCommand.includes('.check()')).toBe(false);
    expect(installCommand.includes('.download(')).toBe(false);
    expect(updaterSource.includes("throw new Error('No downloaded update is ready to install')")).toBe(true);
  });

  test('renderer adapter cannot invoke the removed pre-shutdown command', () => {
    expect(updaterSource.includes('prepare_desktop_shutdown')).toBe(false);
    expect(shellSource.includes('prepare_desktop_shutdown')).toBe(false);
    expect(desktopSource.includes('prepare_desktop_shutdown')).toBe(false);
    expect(desktopSource.includes('updater_builder()')).toBe(true);
    expect(desktopSource.includes('.on_before_exit(')).toBe(true);
  });

  test('renderer capability permits updater check only', () => {
    expect(capability.permissions.filter((permission) => permission.startsWith('updater:'))).toEqual([
      'updater:allow-check',
    ]);
  });
});
