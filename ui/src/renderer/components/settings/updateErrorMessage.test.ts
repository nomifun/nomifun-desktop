/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { describe, expect, test } from 'bun:test';
import { getUpdateErrorMessageKey } from './updateErrorMessage';

describe('getUpdateErrorMessageKey', () => {
  test('maps invalid remote release JSON errors to the localized feed-unavailable message', () => {
    expect(getUpdateErrorMessageKey('Could not fetch a valid release JSON from the remote')).toBe(
      'update.releaseFeedUnavailable'
    );
  });

  test('keeps unknown updater errors on the generic failure message', () => {
    expect(getUpdateErrorMessageKey('permission denied')).toBe('update.checkFailed');
  });

  test.each([
    'NOMIFUN_UPDATER_AUTO_INSTALL_UNSUPPORTED:mounted_volume',
    'Cross-device link (os error 18)',
    'operation crosses devices',
  ])('maps unsafe macOS install error %s to recovery guidance', (message) => {
    expect(getUpdateErrorMessageKey(message)).toBe('update.crossDeviceInstallUnsupported');
  });

  test.each([
    'NOMIFUN_UPDATE_NOT_RETAINED: update 0.4.2 has not been downloaded',
    'NOMIFUN_UPDATE_NOT_RETAINED: update 0.4.2 is still downloading',
  ])('tells the user to download again when the package is gone: %s', (message) => {
    // These used to fall through to 'update.checkFailed', so a failed INSTALL
    // told the user the CHECK had failed — pointing them at the wrong recovery.
    expect(getUpdateErrorMessageKey(message)).toBe('update.packageNoLongerReady');
  });
});
