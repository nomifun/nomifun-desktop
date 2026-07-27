/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

export type UpdaterInstallReason =
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

export type UpdaterPostCleanupFailurePhase = 'install' | 'relaunch';

export interface UpdaterPostCleanupFailure {
  phase: UpdaterPostCleanupFailurePhase;
  error: unknown;
}

export type UpdaterFatalExit = (failure: UpdaterPostCleanupFailure) => Promise<never>;

export interface InstallUpdateDependencies {
  getContext: () => Promise<UpdaterInstallContext>;
  prepareShutdown: () => Promise<void>;
  install: () => Promise<void>;
  relaunch: () => Promise<void>;
  /** Must terminate the desktop process and never return to the renderer. */
  fatalExit: UpdaterFatalExit;
}

async function fatalAfterCleanupFailure(
  fatalExit: UpdaterFatalExit,
  failure: UpdaterPostCleanupFailure
): Promise<never> {
  await fatalExit(failure);

  // Keep the terminal-state guarantee even if a future native exit adapter
  // accidentally returns after requesting process termination. Once cleanup
  // has succeeded, returning to the renderer can only produce an unusable
  // backend-less shell.
  return new Promise<never>(() => {});
}

export async function installUpdateWithPreflight(deps: InstallUpdateDependencies): Promise<void> {
  const context = await deps.getContext();
  if (!context.autoInstallSupported) {
    throw new Error(`${AUTO_INSTALL_UNSUPPORTED_ERROR}:${context.reason ?? 'metadata_unavailable'}`);
  }

  // Keep cleanup outside the fatal block: if cleanup fails, installation must
  // never start and the still-functional app may report the recoverable error.
  await deps.prepareShutdown();

  try {
    await deps.install();
  } catch (error) {
    return fatalAfterCleanupFailure(deps.fatalExit, { phase: 'install', error });
  }

  try {
    await deps.relaunch();
  } catch (error) {
    return fatalAfterCleanupFailure(deps.fatalExit, { phase: 'relaunch', error });
  }
}
