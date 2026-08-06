/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

/**
 * Tauri-native in-app updater adapter. Backs the ipcBridge `update` /
 * `autoUpdate` channels with `@tauri-apps/plugin-updater` (+ `plugin-process`
 * for relaunch). Mirrors tauriShell.ts: every call is GUARDED by `isTauriRuntime()`
 * and the Tauri modules load via dynamic `import()` so the WebUI browser bundle
 * never evaluates Tauri IPC code.
 *
 * Lifecycle — one shared renderer `Update` resource flows across check →
 * metadata display only. Package download and installation are separate custom
 * Rust commands backed by one native cache: download verifies and retains the
 * exact package, while install can only consume those retained bytes and never
 * performs network I/O. The modal performs two back-to-back checks
 * (autoUpdate.check then update.check); `checkPromise` memoizes them into ONE
 * network round-trip, while `force` re-checks on retry / modal reopen.
 *
 * The updater compares against the running bundle's version (Tauri reads it from
 * the workspace `Cargo.toml`, the single source of truth) and fetches the signed
 * `latest.json` from `plugins.updater.endpoints` in tauri.conf.json, verifying
 * each artifact against `plugins.updater.pubkey`.
 */

import type { AutoUpdateStatus } from '@/common/update/updateTypes';
import { isTauriRuntime } from './tauriRuntime';
import {
  tauriDownloadUpdate,
  tauriGetUpdaterInstallContext,
  tauriInstallUpdate,
  type TauriDownloadUpdateProgress,
} from './tauriShell';
import { installUpdateWithPreflight } from './tauriUpdateInstall';

interface TauriUpdate {
  version: string;
  currentVersion: string;
  date?: string;
  body?: string;
  close(): Promise<void>;
}

export interface TauriUpdateInfo {
  version: string;
  /** Version of the currently running bundle (from the Update handle). */
  currentVersion: string;
  releaseNotes?: string;
  releaseDate?: string;
}

// Keep the check resource for version/release metadata. Package bytes are owned
// by the Rust-side DownloadedUpdateState instead of this renderer resource.
let pendingUpdate: TauriUpdate | null = null;
// Mirrors whether Rust has retained the verified package for this session. Once
// true we preserve the matching metadata handle when the modal is reopened.
let downloadComplete = false;
// Memoize the in-flight/last check so the modal's autoUpdate.check + update.check
// share ONE round-trip. Checks are also SERIALIZED through it (each chains after
// any in-flight one) so two never run concurrently — concurrent runs would each
// mint an Update handle and leak all but the last, and could clobber the memo.
let checkPromise: Promise<TauriUpdateInfo | null> | null = null;

function infoFromHandle(u: TauriUpdate): TauriUpdateInfo {
  return { version: u.version, currentVersion: u.currentVersion, releaseNotes: u.body, releaseDate: u.date };
}

async function runCheck(): Promise<TauriUpdateInfo | null> {
  const { check } = await import('@tauri-apps/plugin-updater');
  // Free the previous handle before replacing it (releases the Rust resource).
  // Safe to do sequentially because checks are serialized (see tauriUpdateCheck),
  // so no concurrent run is mid-flight on this handle.
  if (pendingUpdate) {
    try {
      await pendingUpdate.close();
    } catch {
      /* handle already gone — ignore */
    }
    pendingUpdate = null;
  }
  const update = (await check()) as TauriUpdate | null;
  pendingUpdate = update;
  return update ? infoFromHandle(update) : null;
}

/**
 * Check for an available update. Resolves to `null` when up to date (or outside
 * the desktop shell). `force` bypasses the memoized result (retry / reopen) but
 * still serializes behind any in-flight check.
 */
export async function tauriUpdateCheck(force = false): Promise<TauriUpdateInfo | null> {
  if (!isTauriRuntime()) return null;
  // Preserve the metadata associated with Rust's completed in-session download.
  // A fresh check could select a different release than the retained package.
  if (downloadComplete && pendingUpdate) return infoFromHandle(pendingUpdate);
  if (!force && checkPromise) return checkPromise;
  // Chain after any in-flight check so two never run concurrently.
  const prior = checkPromise;
  const run = (async () => {
    try {
      await prior;
    } catch {
      /* the prior check's failure is its caller's problem — we start fresh */
    }
    return runCheck();
  })();
  checkPromise = run;
  // Identity-guarded clear: only drop the memo if it is still THIS run, so a late
  // failure can't discard a newer in-flight check (the next call then retries).
  run.catch(() => {
    if (checkPromise === run) checkPromise = null;
  });
  return run;
}

// The running bundle version is constant for the session; cache the lookup.
let currentVersionCache: string | null = null;

/**
 * The running app version (`@tauri-apps/api/app` getVersion). Used so the
 * "up to date" screen can show the current version even when `check()` returns
 * `null` (no Update handle in that case). Empty string outside the desktop shell.
 */
export async function tauriUpdateCurrentVersion(): Promise<string> {
  if (!isTauriRuntime()) return '';
  if (currentVersionCache != null) return currentVersionCache;
  const { getVersion } = await import('@tauri-apps/api/app');
  currentVersionCache = await getVersion();
  return currentVersionCache;
}

/**
 * Ask Rust to download, verify, and retain the selected update, reporting
 * progress via `emit` (the autoUpdate.status channel the modal subscribes to).
 * Installation later consumes this exact retained package without downloading.
 */
export async function tauriUpdateDownload(emit: (s: AutoUpdateStatus) => void): Promise<void> {
  if (!isTauriRuntime()) throw new Error('Updater is unavailable outside the desktop shell');
  if (!pendingUpdate) await tauriUpdateCheck(true);
  if (!pendingUpdate) throw new Error('No update available to download');
  downloadComplete = false;

  let total = 0;
  let downloaded = 0;
  let speed = 0;
  let lastTs = performance.now();
  let lastBytes = 0;

  emit({ status: 'downloading', progress: { percent: 0, transferred: 0, total: 0, bytesPerSecond: 0 } });

  await tauriDownloadUpdate(pendingUpdate.version, (event: TauriDownloadUpdateProgress) => {
    if (event.phase === 'checking') return;
    if (event.phase === 'downloading') {
      total = event.contentLength ?? total;
      downloaded += event.chunkLength ?? 0;
      const now = performance.now();
      const dt = now - lastTs;
      // Throttle speed sampling to ~4 Hz; keep the last value between samples so
      // the UI doesn't flicker to 0 on sub-window chunks.
      if (dt >= 250) {
        speed = ((downloaded - lastBytes) / dt) * 1000;
        lastTs = now;
        lastBytes = downloaded;
      }
      emit({
        status: 'downloading',
        progress: {
          percent: total > 0 ? Math.min(100, (downloaded / total) * 100) : 0,
          transferred: downloaded,
          total,
          bytesPerSecond: speed,
        },
      });
    } else if (event.phase === 'downloaded') {
      const final = total || downloaded;
      emit({
        status: 'downloading',
        progress: { percent: 100, transferred: final, total: final, bytesPerSecond: 0 },
      });
    }
  });

  // Preserve the matching metadata handle while Rust retains the package.
  downloadComplete = true;
  emit({ status: 'downloaded', version: pendingUpdate.version });
}

/**
 * Ask Rust to install the already-downloaded version, then relaunch on
 * platforms where installation returns. Windows exits inside the updater
 * plugin after its fail-closed pre-exit hook. No-op outside the desktop shell.
 *
 * Routed through installUpdateWithPreflight so its fail-closed guarantees are
 * live: the unsupported-context preflight throws before anything runs, and a
 * failure after installation has started (install or the macOS post-install
 * relaunch) terminates the process instead of returning control to a renderer
 * that may be sitting on top of a replaced app bundle.
 */
export async function tauriUpdateInstallAndRelaunch(emit: (s: AutoUpdateStatus) => void): Promise<void> {
  if (!isTauriRuntime()) throw new Error('Updater is unavailable outside the desktop shell');
  if (!pendingUpdate || !downloadComplete) throw new Error('No downloaded update is ready to install');
  const version = pendingUpdate.version;

  // Give immediate feedback while the native install preflight begins.
  emit({ status: 'installing', installPhase: 'preparing' });
  await installUpdateWithPreflight({
    getContext: tauriGetUpdaterInstallContext,
    // No renderer-held resource needs cleanup before the Rust-owned install
    // today; the hook keeps the ordering contract (cleanup failure must
    // prevent installation) wired for when one appears.
    prepareShutdown: async () => {},
    install: async () => {
      emit({ status: 'installing', installPhase: 'installing' });
      await tauriInstallUpdate(version);
    },
    relaunch: async () => {
      const { relaunch } = await import('@tauri-apps/plugin-process');
      await relaunch();
    },
    fatalExit: async (failure) => {
      console.error(
        `[tauriUpdater] fatal ${failure.phase} failure after install started; exiting`,
        failure.error
      );
      const { exit } = await import('@tauri-apps/plugin-process');
      await exit(1);
      // Unreachable when exit() succeeds; installUpdateWithPreflight also
      // guards against an exit adapter that unexpectedly returns.
      return new Promise<never>(() => {});
    },
  });
}

// ---------------------------------------------------------------------------
// autoUpdate.status — a renderer-local emitter (fed by the native download
// channel above), shaped like tauriShell's
// ShellEmitter so ipcBridge can expose it directly as `autoUpdate.status`.
// ---------------------------------------------------------------------------

function createLocalEmitter<T>() {
  const listeners = new Set<(v: T) => void>();
  return {
    on(cb: (v: T) => void): () => void {
      listeners.add(cb);
      return () => {
        listeners.delete(cb);
      };
    },
    emit(v: T): void {
      listeners.forEach((l) => {
        try {
          l(v);
        } catch {
          /* a listener throwing must not break the others */
        }
      });
    },
  };
}

export const autoUpdateStatusEmitter = createLocalEmitter<AutoUpdateStatus>();
