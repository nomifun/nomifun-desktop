/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import type { TauriUpdatePackageState } from '@/common/adapter/tauriShell';

/**
 * Decide what the update modal should show after a check, from the NATIVE side's
 * package slot rather than from React state.
 *
 * Two defects motivate this. (1) The modal used to reach 'downloaded' only via a
 * live status event, so any later re-check — sidebar update badge, About, the
 * startup check — reset the UI to 'available' and the Install button vanished
 * while the native side still held the verified bytes; re-running a whole
 * download was the only way back. (2) Guarding the re-check instead (skip it
 * while a download is in flight) traded that for a worse failure: a download
 * that never settles left the modal with no path back at all. Deriving from the
 * slot fixes both, because a re-check can always run and always lands on the
 * truth.
 */
export type DerivedUpdateStatus = 'available' | 'downloading' | 'downloaded';

/** Compare release versions tolerantly: trim, and ignore a display `v` prefix. */
function sameVersion(a: string | null | undefined, b: string | null | undefined): boolean {
  const normalize = (value: string | null | undefined): string =>
    String(value ?? '')
      .trim()
      .replace(/^v/i, '');
  const left = normalize(a);
  return left.length > 0 && left === normalize(b);
}

export function deriveUpdateStatus(input: {
  availableVersion?: string;
  /** Version whose verified package the native side has ready, if any. */
  retainedVersion?: string | null;
  /** The native slot's state, when known. */
  slotState?: TauriUpdatePackageState | null;
  /** Version the native slot is busy with, for any active state. */
  slotVersion?: string | null;
}): DerivedUpdateStatus {
  // A download already running for this release owns the screen: never offer to
  // start it again, and never hide it behind an "available" affordance.
  if (input.slotState === 'downloading' && sameVersion(input.availableVersion, input.slotVersion)) {
    return 'downloading';
  }
  // The retained bytes are keyed by version; only offer to install them when
  // they are the release actually being offered.
  return sameVersion(input.availableVersion, input.retainedVersion) ? 'downloaded' : 'available';
}

/**
 * Whether a download status event belongs to the flow the modal is currently
 * showing. Two live flows keep separate byte counters, so letting both write is
 * what made one progress bar flip between two unrelated readings — and letting a
 * superseded flow's terminal frame through would flip the modal to the Install
 * screen while the live download was still mid-transfer.
 *
 * Fails OPEN: when either side is unknown the event is applied, so an event
 * without a version stamp can never silently freeze the UI.
 */
export function shouldApplyDownloadEvent(
  eventVersion: string | undefined | null,
  activeVersion: string | null
): boolean {
  if (!eventVersion || !activeVersion) return true;
  return sameVersion(eventVersion, activeVersion);
}
