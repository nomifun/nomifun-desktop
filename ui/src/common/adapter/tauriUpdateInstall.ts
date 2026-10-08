/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

type UpdaterInstallReason =
  | 'app_bundle_not_found'
  | 'app_translocation'
  | 'mounted_volume'
  | 'cross_device'
  | 'metadata_unavailable';

export interface UpdaterInstallContext {
  platform: string;
  appBundlePath: string | null;
  tempDir: string;
  appDeviceId: number | null;
  tempDeviceId: number | null;
  autoInstallSupported: boolean;
  reason: UpdaterInstallReason | null;
}

export const AUTO_INSTALL_UNSUPPORTED_ERROR = 'NOMIFUN_UPDATER_AUTO_INSTALL_UNSUPPORTED';

/**
 * Marker the native `install_update` puts on failures that provably never reached
 * the installer: no retained package for that version, or a download for it is
 * still running. Nothing on disk was touched and the native side has restored
 * whatever it held, so these are recoverable — reporting them is correct and
 * terminating the process is not.
 *
 * An install ALREADY IN FLIGHT is deliberately NOT marked: by then the handoff
 * has begun and on macOS the running .app may already have been renamed aside,
 * so it must keep the fail-closed exit. See `TakeReadyError` and
 * `handoff_may_have_started` in apps/desktop/src/main.rs — the split there is a
 * safety boundary, not a message-formatting choice.
 *
 * Keep this value identical to `UPDATE_NOT_RETAINED_ERROR` in
 * apps/desktop/src/main.rs (pinned by a test in tauriUpdater.test.ts).
 */
export const INSTALL_NOT_ATTEMPTED_ERROR = 'NOMIFUN_UPDATE_NOT_RETAINED';

/**
 * Whether a rejected install never reached the installer (see the marker above).
 *
 * Anchored with `startsWith`, not a substring test: the native side emits the
 * marker strictly as a prefix, and a looser match would let any message that
 * merely quotes an earlier refusal downgrade a genuine post-handoff failure to
 * "recoverable" — inverting the fail-closed guarantee via string concatenation.
 */
export function isInstallNotAttempted(error: unknown): boolean {
  const message = error instanceof Error ? error.message : typeof error === 'string' ? error : '';
  return message.startsWith(INSTALL_NOT_ATTEMPTED_ERROR);
}

type UpdaterInstallFailurePhase = 'install' | 'relaunch';

interface UpdaterInstallFailure {
  phase: UpdaterInstallFailurePhase;
  error: unknown;
}

type UpdaterFatalExit = (failure: UpdaterInstallFailure) => Promise<never>;

export interface InstallUpdateDependencies {
  getContext: () => Promise<UpdaterInstallContext>;
  install: () => Promise<void>;
  relaunch: () => Promise<void>;
  showFailure: (failure: UpdaterInstallFailure) => Promise<void>;
  /** Must terminate the desktop process and never return to the renderer. */
  fatalExit: UpdaterFatalExit;
}

async function exitAfterInstallFailure(
  deps: InstallUpdateDependencies,
  failure: UpdaterInstallFailure
): Promise<never> {
  try {
    await deps.showFailure(failure);
  } catch (error) {
    console.error('Could not show the update failure dialog:', error);
  }
  await deps.fatalExit(failure);

  // The bundle may be partially replaced. Never return to the renderer even
  // if an exit adapter unexpectedly returns after requesting termination.
  return new Promise<never>(() => {});
}

export async function installUpdateWithPreflight(deps: InstallUpdateDependencies): Promise<void> {
  const context = await deps.getContext();
  if (!context.autoInstallSupported) {
    throw new Error(`${AUTO_INSTALL_UNSUPPORTED_ERROR}:${context.reason ?? 'metadata_unavailable'}`);
  }

  try {
    await deps.install();
  } catch (error) {
    // A refusal that never reached the installer leaves the app exactly as it
    // was, so it must travel back to the caller as an ordinary error. Only a
    // failure that may have already replaced part of the installed app takes
    // the terminal path — returning to a renderer sitting on top of a
    // half-replaced bundle is the thing this guard exists to prevent.
    if (isInstallNotAttempted(error)) throw error;
    return exitAfterInstallFailure(deps, { phase: 'install', error });
  }

  try {
    await deps.relaunch();
  } catch (error) {
    return exitAfterInstallFailure(deps, { phase: 'relaunch', error });
  }
}
