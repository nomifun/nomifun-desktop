/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { describe, expect, test } from 'bun:test';
import { deriveUpdateStatus, shouldApplyDownloadEvent } from './deriveUpdateStatus';

describe('deriveUpdateStatus', () => {
  test('a retained package for the offered version restores the install affordance', () => {
    // The regression this pins: after a finished download the user clicks the
    // sidebar update badge (or About → check), the modal re-checks, and the
    // Install button must NOT vanish while the native side still holds the
    // verified package.
    expect(deriveUpdateStatus({ availableVersion: '0.4.2', retainedVersion: '0.4.2' })).toBe('downloaded');
  });

  test('no retained package leaves the user on the download affordance', () => {
    expect(deriveUpdateStatus({ availableVersion: '0.4.2', retainedVersion: null })).toBe('available');
    expect(deriveUpdateStatus({ availableVersion: '0.4.2' })).toBe('available');
  });

  test('a retained package for a different version must not offer to install it', () => {
    // The retained bytes are keyed by version; installing them as if they were
    // the offered release is exactly the mismatch that ends in a native error.
    expect(deriveUpdateStatus({ availableVersion: '0.4.3', retainedVersion: '0.4.2' })).toBe('available');
  });

  test('version comparison ignores surrounding whitespace and a leading v', () => {
    // latest.json is normalized through semver, but the modal also renders
    // tagName-shaped strings; never strand a real package on a formatting diff.
    expect(deriveUpdateStatus({ availableVersion: ' 0.4.2 ', retainedVersion: '0.4.2' })).toBe('downloaded');
    expect(deriveUpdateStatus({ availableVersion: 'v0.4.2', retainedVersion: '0.4.2' })).toBe('downloaded');
  });

  test('an unknown offered version can never match a retained package', () => {
    expect(deriveUpdateStatus({ retainedVersion: '0.4.2' })).toBe('available');
    expect(deriveUpdateStatus({ availableVersion: '', retainedVersion: '' })).toBe('available');
  });

  test('a near-miss version is not treated as a match', () => {
    expect(deriveUpdateStatus({ availableVersion: '0.4.2', retainedVersion: '0.4.20' })).toBe('available');
    expect(deriveUpdateStatus({ availableVersion: '0.4.20', retainedVersion: '0.4.2' })).toBe('available');
  });

  test('a live download for the offered release owns the screen', () => {
    // A re-check must be able to run at ANY time and still land on the truth:
    // guarding the re-check instead left a wedged download with no way back.
    expect(
      deriveUpdateStatus({ availableVersion: '0.4.2', slotState: 'downloading', slotVersion: '0.4.2' })
    ).toBe('downloading');
  });

  test('a download for a different release does not hijack the screen', () => {
    expect(
      deriveUpdateStatus({ availableVersion: '0.4.3', slotState: 'downloading', slotVersion: '0.4.2' })
    ).toBe('available');
  });

  test('a retained package still wins over an unrelated slot state', () => {
    expect(
      deriveUpdateStatus({
        availableVersion: '0.4.2',
        retainedVersion: '0.4.2',
        slotState: 'installing',
        slotVersion: '0.4.2',
      })
    ).toBe('downloaded');
  });
});

describe('shouldApplyDownloadEvent', () => {
  test('applies events from the flow the modal is showing', () => {
    expect(shouldApplyDownloadEvent('0.4.2', '0.4.2')).toBe(true);
  });

  test('discards events from a superseded flow', () => {
    // Includes TERMINAL frames: a stale flow's completion used to flip the modal
    // to the Install screen while the live download was still transferring.
    expect(shouldApplyDownloadEvent('0.4.1', '0.4.2')).toBe(false);
  });

  test('fails open when either side is unknown so the UI can never freeze', () => {
    expect(shouldApplyDownloadEvent(undefined, '0.4.2')).toBe(true);
    expect(shouldApplyDownloadEvent('0.4.2', null)).toBe(true);
    expect(shouldApplyDownloadEvent(undefined, null)).toBe(true);
  });
});
