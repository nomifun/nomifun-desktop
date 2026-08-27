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
 * the workspace `Cargo.toml`, the single source of truth) and checks the ordered
 * `plugins.updater.endpoints` in tauri.conf.json (CrabNebula first, GitHub
 * fallback), verifying every selected artifact against `plugins.updater.pubkey`.
 */

import type { AutoUpdateStatus } from '@/common/update/updateTypes';
import { isTauriRuntime } from './tauriRuntime';
import {
  tauriDownloadUpdate,
  tauriGetUpdaterInstallContext,
  tauriInstallUpdate,
  tauriUpdatePackageStatus,
  type TauriDownloadUpdateProgress,
  type TauriUpdatePackageStatus,
} from './tauriShell';
import { installUpdateWithPreflight } from './tauriUpdateInstall';

interface TauriUpdate {
  version: string;
  currentVersion: string;
  date?: string;
  body?: string;
  close(): Promise<void>;
}

const UPDATE_CHECK_TIMEOUT_MS = 8_000;

export interface TauriUpdateInfo {
  version: string;
  /** Version of the currently running bundle (from the Update handle). */
  currentVersion: string;
  releaseNotes?: string;
  releaseDate?: string;
}

// Keep the check resource for version/release metadata. Package bytes — and the
// FACT that a package exists — are owned by the Rust-side DownloadedUpdateState.
// This module deliberately keeps no mirror of that fact: a renderer-local
// `downloadComplete` boolean used to be cleared at the start of every download
// attempt and never restored on failure, so a rejected attempt disabled the
// install action while Rust still held verified bytes, and the only way to
// re-arm it was to run a whole download again.
let pendingUpdate: TauriUpdate | null = null;
// Memoize the in-flight/last check so the modal's autoUpdate.check + update.check
// share ONE round-trip. Checks are also SERIALIZED through it (each chains after
// any in-flight one) so two never run concurrently — concurrent runs would each
// mint an Update handle and leak all but the last, and could clobber the memo.
let checkPromise: Promise<TauriUpdateInfo | null> | null = null;

function infoFromHandle(u: TauriUpdate): TauriUpdateInfo {
  return { version: u.version, currentVersion: u.currentVersion, releaseNotes: u.body, releaseDate: u.date };
}

/**
 * The version the native slot is currently busy with (downloading, ready, or
 * installing), or null when it holds nothing. Any active version means the
 * metadata handle for it must be preserved: replacing it could point install at
 * a different release than the one whose bytes are retained.
 */
async function nativeActiveVersion(): Promise<string | null> {
  try {
    const status = await tauriUpdatePackageStatus();
    return status.version ?? null;
  } catch {
    // Treat an unavailable native side as "nothing retained" — the install path
    // re-queries and fails closed there.
    return null;
  }
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
  const update = (await check({ timeout: UPDATE_CHECK_TIMEOUT_MS })) as TauriUpdate | null;
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
  if (!force && checkPromise) return checkPromise;
  // Chain after any in-flight check so two never run concurrently.
  const prior = checkPromise;
  const run = (async () => {
    try {
      await prior;
    } catch {
      /* the prior check's failure is its caller's problem — we start fresh */
    }
    // Preserve the metadata matching whatever the native side is working on —
    // including DURING a download, which the old boolean could not cover: it
    // was false for the whole download window, so a modal reopen mid-download
    // closed the handle the download was keyed to.
    const active = await nativeActiveVersion();
    if (active && pendingUpdate?.version === active) return infoFromHandle(pendingUpdate);
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

/**
 * The native slot snapshot, so the UI can derive what to show (including "a
 * download is already running") instead of keeping its own copy of that fact.
 * The single place the readiness rule is applied is the bridge that consumes
 * this; deliberately no second `retainedVersion` helper exists to drift from it.
 */
export async function tauriUpdatePackageSnapshot(): Promise<TauriUpdatePackageStatus | null> {
  if (!isTauriRuntime()) return null;
  try {
    return await tauriUpdatePackageStatus();
  } catch {
    return null;
  }
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
  // Pin the version for the whole flow. Re-reading the module global after the
  // await could pick up a handle swapped in by a check that ran mid-download,
  // silently keying the completion event to a different release.
  const downloadVersion = pendingUpdate.version;

  let total = 0;
  let downloaded = 0;
  let speed = 0;
  let lastTs = performance.now();
  let lastBytes = 0;

  // Every progress frame is stamped with the version it belongs to so the UI can
  // discard a series from a superseded download instead of letting two
  // independent byte counters fight over one progress bar.
  emit({
    status: 'downloading',
    version: downloadVersion,
    progress: { percent: 0, transferred: 0, total: 0, bytesPerSecond: 0 },
  });

  await tauriDownloadUpdate(downloadVersion, (event: TauriDownloadUpdateProgress) => {
    if (event.phase === 'checking') return;
    if (event.phase === 'retrying') {
      total = 0;
      downloaded = 0;
      speed = 0;
      lastTs = performance.now();
      lastBytes = 0;
      emit({
        status: 'downloading',
        version: downloadVersion,
        progress: { percent: 0, transferred: 0, total: 0, bytesPerSecond: 0 },
      });
      return;
    }
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
        version: downloadVersion,
        progress: {
          percent: total > 0 ? Math.min(100, (downloaded / total) * 100) : 0,
          transferred: downloaded,
          total,
          bytesPerSecond: speed,
        },
      });
    } else if (event.phase === 'downloaded') {
      // Prefer the length the native side reported: on the "already retained"
      // fast path no chunk events arrive at all, and reporting `total || 0` there
      // painted a 100% bar over "0.0 KB / 0.0 KB".
      const final = event.contentLength ?? (total || downloaded);
      emit({
        status: 'downloading',
        version: downloadVersion,
        progress: { percent: 100, transferred: final, total: final, bytesPerSecond: 0 },
      });
    }
  });

  emit({ status: 'downloaded', version: downloadVersion });
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
  // Ask the owner of the bytes, not a local mirror. This runs BEFORE any
  // shutdown/cleanup, so a "nothing retained" answer is a plain recoverable
  // error: the app stays usable and the user can retry.
  const status = await tauriUpdatePackageStatus();
  if (status.state !== 'ready' || !status.version) {
    throw new Error('No downloaded update is ready to install');
  }
  const version = status.version;

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
