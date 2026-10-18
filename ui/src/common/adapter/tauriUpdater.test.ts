/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { describe, expect, test } from 'bun:test';
import { readFileSync } from 'node:fs';
import { INSTALL_NOT_ATTEMPTED_ERROR } from './tauriUpdateInstall';
import { getUpdateErrorMessageKey } from '@/renderer/components/settings/updateErrorMessage';
import { tauriUpdateCheck } from './tauriUpdater';

const updaterSource = readFileSync(new URL('./tauriUpdater.ts', import.meta.url), 'utf8');
const shellSource = readFileSync(new URL('./tauriShell.ts', import.meta.url), 'utf8');
const desktopSource = readFileSync(
  new URL('../../../../apps/desktop/src/main.rs', import.meta.url),
  'utf8'
);
const desktopConfig = JSON.parse(
  readFileSync(new URL('../../../../apps/desktop/tauri.conf.json', import.meta.url), 'utf8')
) as { plugins: { updater: { endpoints: string[] } } };
const cloudReleaseSource = readFileSync(
  new URL('../../../../scripts/crabnebula-release.mjs', import.meta.url),
  'utf8'
);
const capability = JSON.parse(
  readFileSync(new URL('../../../../apps/desktop/capabilities/default.json', import.meta.url), 'utf8')
) as { permissions: string[] };

const WINDOWS_TARGETS_NOT_FOUND =
  'None of the fallback platforms `["windows-x86_64-nsis", "windows-x86_64"]` were found in the response `platforms` object';

async function withUpdaterCheck(
  check: () => Promise<unknown>,
  run: (calls: Array<{ command: string; args: unknown }>) => Promise<void>
): Promise<void> {
  const originalInternals = Object.getOwnPropertyDescriptor(window, '__TAURI_INTERNALS__');
  const calls: Array<{ command: string; args: unknown }> = [];
  Object.defineProperty(window, '__TAURI_INTERNALS__', {
    configurable: true,
    value: {
      invoke: async (command: string, args: unknown) => {
        calls.push({ command, args });
        if (command === 'update_package_status') return { state: 'empty', version: null };
        if (command === 'plugin:updater|check') return check();
        if (command === 'plugin:resources|close') return;
        throw new Error(`Unexpected native command: ${command}`);
      },
    },
  });
  try {
    await run(calls);
  } finally {
    // Clear the cached result and close any metadata handle while IPC is present.
    check = async () => null;
    await tauriUpdateCheck(true);
    if (originalInternals) Object.defineProperty(window, '__TAURI_INTERNALS__', originalInternals);
    else Reflect.deleteProperty(window, '__TAURI_INTERNALS__');
  }
}

describe('platform-specific update availability', () => {
  test.each([
    WINDOWS_TARGETS_NOT_FOUND,
    new Error(WINDOWS_TARGETS_NOT_FOUND),
    'None of the fallback platforms `["darwin-aarch64-app", "darwin-aarch64"]` were found in the response `platforms` object',
    'the platform `linux-x86_64` was not found in the response `platforms` object',
  ])('a release without the current target is a cached no-update result: %s', async (error) => {
    await withUpdaterCheck(async () => { throw error; }, async (calls) => {
      expect(await tauriUpdateCheck(true)).toBeNull();
      expect(await tauriUpdateCheck()).toBeNull();
      const checks = calls.filter(({ command }) => command === 'plugin:updater|check');
      expect(checks).toEqual([{ command: 'plugin:updater|check', args: { timeout: 8_000 } }]);
    });
  });

  test('a forced recheck finds a package once the current platform is published', async () => {
    let published = false;
    await withUpdaterCheck(async () => {
      if (!published) throw WINDOWS_TARGETS_NOT_FOUND;
      return { rid: 42, currentVersion: '0.8.12', version: '0.8.14', body: 'Windows release' };
    }, async () => {
      expect(await tauriUpdateCheck(true)).toBeNull();
      published = true;
      expect(await tauriUpdateCheck(true)).toEqual({
        currentVersion: '0.8.12', version: '0.8.14', releaseNotes: 'Windows release', releaseDate: undefined,
      });
    });
  });

  test('a missing target clears a previously available metadata handle', async () => {
    let available = true;
    await withUpdaterCheck(async () => {
      if (!available) throw WINDOWS_TARGETS_NOT_FOUND;
      return { rid: 43, currentVersion: '0.8.12', version: '0.8.13' };
    }, async (calls) => {
      expect((await tauriUpdateCheck(true))?.version).toBe('0.8.13');
      available = false;
      expect(await tauriUpdateCheck(true)).toBeNull();
      expect(calls).toContainEqual({ command: 'plugin:resources|close', args: { rid: 43 } });
      expect(await tauriUpdateCheck()).toBeNull();
    });
  });

  test.each([
    'error sending request for url: connection timed out',
    'Could not fetch a valid release JSON from the remote',
    'missing field `platforms`',
    'invalid type: null, expected struct RemoteRelease',
    new Error('signature verification failed'),
  ])('real update failures still reject and can be retried: %s', async (error) => {
    let failed = true;
    await withUpdaterCheck(async () => {
      if (failed) throw error;
      return null;
    }, async (calls) => {
      await expect(tauriUpdateCheck(true)).rejects.toBe(error);
      failed = false;
      expect(await tauriUpdateCheck()).toBeNull();
      expect(calls.filter(({ command }) => command === 'plugin:updater|check')).toHaveLength(2);
    });
  });
});

describe('desktop updater security boundary', () => {
  test('renderer exposes download but no raw updater install path', () => {
    expect(updaterSource.includes('download(onEvent?')).toBe(false);
    expect(updaterSource.includes('install(): Promise<void>')).toBe(false);
    expect(updaterSource.includes('downloadAndInstall')).toBe(false);
    expect(updaterSource.includes('.install(')).toBe(false);
    expect(updaterSource.includes('await tauriInstallUpdate(version)')).toBe(true);
  });

  test('install goes through the fail-closed preflight/fatal-exit contract', () => {
    // installUpdateWithPreflight owns ordering (preflight → install → relaunch)
    // and reports failures before a fatal exit once install has started; the raw
    // sequential call path must not come back.
    expect(updaterSource.includes('installUpdateWithPreflight({')).toBe(true);
    expect(updaterSource.includes('fatalExit')).toBe(true);
    expect(updaterSource.includes('showFailure')).toBe(true);
    expect(updaterSource.includes('prepareShutdown')).toBe(false);
    expect(updaterSource.includes('install: async () => {')).toBe(true);
  });

  test('native download owns progress and install consumes only the retained package', () => {
    expect(shellSource.includes('new Channel<TauriDownloadUpdateProgress>(onProgress)')).toBe(true);
    expect(desktopSource.includes('tauri::ipc::Channel<DownloadUpdateProgress>')).toBe(true);
    expect(desktopSource.includes('phase: "downloading"')).toBe(true);
    expect(desktopSource.includes('.take_ready(&requested_version)')).toBe(true);
    expect(desktopSource.includes('package.update.install(&package.bytes)')).toBe(true);
    const installStart = desktopSource.indexOf('async fn install_update(');
    const installEnd = desktopSource.indexOf('const UPDATER_SHUTDOWN_MAX_ATTEMPTS', installStart);
    const installCommand = desktopSource.slice(installStart, installEnd);
    expect(installCommand.includes('.check()')).toBe(false);
    expect(installCommand.includes('.download(')).toBe(false);
  });

  test('install readiness is owned by the native slot, not mirrored in the renderer', () => {
    // A renderer-local `downloadComplete` boolean was a SECOND source of truth:
    // it was cleared at the start of every download attempt and never restored on
    // failure, so a rejected attempt bricked the Install button while the native
    // slot still held a perfectly good verified package. The only way to re-arm it
    // was to run a whole download again — the reported "download twice" workaround.
    // Assert on the CODE, not on prose: the doc comment deliberately keeps the
    // name around to explain why the mirror is gone.
    expect(updaterSource.includes('let downloadComplete')).toBe(false);
    expect(updaterSource.includes('downloadComplete =')).toBe(false);
    expect(updaterSource.includes('!downloadComplete')).toBe(false);
    expect(shellSource.includes("invoke<TauriUpdatePackageStatus>('update_package_status')")).toBe(true);
    expect(desktopSource.includes('async fn update_package_status(')).toBe(true);

    // Scope to the install function: `tauriUpdatePackageStatus()` is also called
    // by two other helpers, so a file-wide grep would stay green even if the gate
    // itself were deleted.
    const start = updaterSource.indexOf('export async function tauriUpdateInstallAndRelaunch(');
    const end = updaterSource.indexOf('\n// ---', start);
    const installFn = updaterSource.slice(start, end === -1 ? undefined : end);
    expect(start).toBeGreaterThan(-1);
    expect(installFn.includes('await tauriUpdatePackageStatus()')).toBe(true);
    expect(installFn.includes("throw new Error('No downloaded update is ready to install')")).toBe(true);
    // The version handed to the native install must come from the slot, not from
    // the renderer's metadata handle, or the two can disagree.
    expect(installFn.includes('const version = status.version')).toBe(true);
    expect(installFn.includes('pendingUpdate.version')).toBe(false);
  });

  test('the never-attempted marker is shared verbatim with the native side', () => {
    // Three copies of this string exist (Rust, the adapter constant, the error
    // mapper). Renaming any one silently re-arms the exit(1) defect, so pin them
    // to each other rather than each to itself.
    expect(desktopSource.includes(`const UPDATE_NOT_RETAINED_ERROR: &str = "${INSTALL_NOT_ATTEMPTED_ERROR}"`)).toBe(
      true
    );
    expect(getUpdateErrorMessageKey(`${INSTALL_NOT_ATTEMPTED_ERROR}: whatever`)).toBe(
      'update.packageNoLongerReady'
    );
  });

  test('a refused install is marked never-attempted, and a real failure never is', () => {
    // take_ready failures touch nothing; they must be distinguishable from a real
    // install failure, which may have left the app bundle half replaced.
    const installStart = desktopSource.indexOf('async fn install_update(');
    const installEnd = desktopSource.indexOf('#[tauri::command]', installStart + 1);
    const installCommand = desktopSource.slice(installStart, installEnd);
    expect(installStart).toBeGreaterThan(-1);
    expect(installEnd).toBeGreaterThan(installStart);
    expect(installCommand.includes('UPDATE_NOT_RETAINED_ERROR')).toBe(true);

    // NEGATIVE: everything from the real installer handoff onwards must stay
    // unmarked, or isInstallNotAttempted would swallow a half-replaced bundle and
    // the fail-closed exit would be globally defeated.
    const handoff = installCommand.slice(installCommand.indexOf('package.update.install('));
    expect(handoff.length).toBeGreaterThan(0);
    expect(handoff.includes('UPDATE_NOT_RETAINED_ERROR')).toBe(false);

    // An install already in flight is NOT recoverable: on macOS the running
    // bundle has already been renamed aside by that point. Pin the SPLIT, not
    // just its presence — collapsing it back to one blanket map_err would
    // silently route a half-replaced bundle down the recoverable path.
    expect(installCommand.includes('error.handoff_may_have_started()')).toBe(true);
    expect(desktopSource.includes('fn handoff_may_have_started(&self) -> bool')).toBe(true);
    expect(desktopSource.includes('matches!(self, Self::AlreadyInstalling { .. })')).toBe(true);

    // A completed install must release the slot — and only AFTER the handoff
    // succeeded. A failed replacement must never offer the same package again.
    expect(installCommand.includes('finish_install(')).toBe(true);
    expect(installCommand.indexOf('package.update.install(')).toBeLessThan(
      installCommand.indexOf('finish_install(')
    );
    expect(desktopSource.includes('restore_ready(')).toBe(false);
    expect(installCommand.includes('tracing::error!')).toBe(true);
  });

  test('progress is coalesced natively and carries the version it belongs to', () => {
    // One webview eval + one React render per HTTP chunk meant tens of thousands
    // of renders for a large installer; and untagged progress from a stale
    // download flow could repaint the bar with a second, contradictory series.
    expect(desktopSource.includes('UPDATE_PROGRESS_MIN_INTERVAL')).toBe(true);
    expect(updaterSource.includes('version: downloadVersion')).toBe(true);
  });

  test('CrabNebula is primary, while check and package download have bounded GitHub fallback', () => {
    expect(desktopConfig.plugins.updater.endpoints).toEqual([
      'https://cdn.crabnebula.app/update/nomifun/nomifun-desktop/{{target}}-{{arch}}/{{current_version}}',
      'https://github.com/nomifun/nomifun-desktop/releases/latest/download/latest.json',
    ]);
    expect(updaterSource.includes('check({ timeout: UPDATE_CHECK_TIMEOUT_MS })')).toBe(true);
    expect(desktopSource.includes('const UPDATE_CHECK_TIMEOUT: Duration = Duration::from_secs(8)')).toBe(true);
    expect(desktopSource.includes('const GITHUB_UPDATER_ENDPOINT: &str')).toBe(true);
    expect(desktopSource.includes('Some(vec![github_endpoint])')).toBe(true);
    expect(desktopSource.includes('GitHub fallback download failed')).toBe(true);
  });

  test('switching download source resets renderer progress before GitHub bytes arrive', () => {
    expect(shellSource.includes("'checking' | 'retrying' | 'downloading' | 'downloaded'")).toBe(true);
    const retryStart = updaterSource.indexOf("if (event.phase === 'retrying')");
    const retryEnd = updaterSource.indexOf("if (event.phase === 'downloading')", retryStart);
    const retryBranch = updaterSource.slice(retryStart, retryEnd);
    expect(retryStart).toBeGreaterThan(-1);
    expect(retryBranch.includes('downloaded = 0')).toBe(true);
    expect(retryBranch.includes('lastBytes = 0')).toBe(true);
    expect(retryBranch.includes('transferred: 0')).toBe(true);
  });

  test('cloud release helper keeps credentials external and requires signed updater assets', () => {
    expect(cloudReleaseSource.includes('process.env.CN_API_KEY')).toBe(true);
    expect(cloudReleaseSource.includes("RELEASE_ENV_FILE")).toBe(true);
    expect(cloudReleaseSource.includes('CN_API_KEY=REPLACE_ME')).toBe(false);
    expect(cloudReleaseSource.includes("'--signature'")).toBe(true);
    expect(cloudReleaseSource.includes('does not match requested release')).toBe(true);
    expect(cloudReleaseSource.includes("'--channel'")).toBe(true);
    expect(cloudReleaseSource.includes('signatureMatches(path, entry.signature)')).toBe(true);
    expect(cloudReleaseSource.includes("['release', 'publish', app, releaseId]")).toBe(true);
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
