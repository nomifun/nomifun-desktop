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
    expect(updaterSource.includes('download(onEvent?')).toBe(true);
    expect(updaterSource.includes('install(): Promise<void>')).toBe(false);
    expect(updaterSource.includes('downloadAndInstall')).toBe(false);
    expect(updaterSource.includes('.install(')).toBe(false);
    expect(updaterSource.includes('tauriInstallUpdate(version)')).toBe(true);
  });

  test('install goes through the fail-closed preflight/fatal-exit contract', () => {
    // installUpdateWithPreflight owns ordering (preflight → cleanup → install →
    // relaunch) and guarantees a fatal exit once install has started; the raw
    // sequential call path must not come back.
    expect(updaterSource.includes('installUpdateWithPreflight({')).toBe(true);
    expect(updaterSource.includes('fatalExit')).toBe(true);
    expect(updaterSource.includes('prepareShutdown')).toBe(true);
    expect(/await\s+tauriInstallUpdate\(/.test(updaterSource)).toBe(false);
  });

  test('renderer adapter cannot invoke the removed pre-shutdown command', () => {
    expect(updaterSource.includes('prepare_desktop_shutdown')).toBe(false);
    expect(shellSource.includes('prepare_desktop_shutdown')).toBe(false);
    expect(desktopSource.includes('prepare_desktop_shutdown')).toBe(false);
    expect(desktopSource.includes('updater_builder()')).toBe(true);
    expect(desktopSource.includes('.on_before_exit(')).toBe(true);
  });

  test('capability permits only updater check and download', () => {
    expect(capability.permissions.filter((permission) => permission.startsWith('updater:'))).toEqual([
      'updater:allow-check',
      'updater:allow-download',
    ]);
  });
});
