/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { describe, expect, test } from 'bun:test';
import { getInstallBlockMessageKey, getUpdateErrorMessageKey } from './updateErrorMessage';
import enUpdate from '@/renderer/services/i18n/locales/en-US/update.json';
import zhUpdate from '@/renderer/services/i18n/locales/zh-CN/update.json';

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
    'NOMIFUN_UPDATE_NOT_RETAINED: update 0.4.2 has not been downloaded',
    'NOMIFUN_UPDATE_NOT_RETAINED: update 0.4.2 is still downloading',
  ])('tells the user to download again when the package is gone: %s', (message) => {
    // These used to fall through to 'update.checkFailed', so a failed INSTALL
    // told the user the CHECK had failed — pointing them at the wrong recovery.
    expect(getUpdateErrorMessageKey(message)).toBe('update.packageNoLongerReady');
  });
});

describe('getInstallBlockMessageKey', () => {
  test.each([
    ['mounted_volume', 'update.installBlockedMountedVolume'],
    ['app_translocation', 'update.installBlockedTranslocation'],
    ['cross_device', 'update.installBlockedCrossDevice'],
    ['metadata_unavailable', 'update.installBlockedMetadataUnavailable'],
    ['app_bundle_not_found', 'update.installBlockedBundleNotFound'],
  ] as const)('maps the %s refusal to its own message', (reason, key) => {
    expect(getInstallBlockMessageKey(reason)).toBe(key);
  });

  test('every refusal reason maps to a distinct key', () => {
    const keys = new Set(
      (
        [
          'mounted_volume',
          'app_translocation',
          'cross_device',
          'metadata_unavailable',
          'app_bundle_not_found',
        ] as const
      ).map((reason) => getInstallBlockMessageKey(reason))
    );
    expect(keys.size).toBe(5);
  });

  test('an unknown refusal falls back to the metadata message', () => {
    expect(getInstallBlockMessageKey(null)).toBe('update.installBlockedMetadataUnavailable');
  });

  test('every install-block string exists in both locales', () => {
    const keys = [
      'installBlockedTitle',
      'installBlockedMountedVolume',
      'installBlockedTranslocation',
      'installBlockedCrossDevice',
      'installBlockedMetadataUnavailable',
      'installBlockedBundleNotFound',
      'installBlockedLocation',
    ] as const;
    for (const update of [enUpdate, zhUpdate]) {
      for (const key of keys) {
        expect(update[key]).toBeTruthy();
      }
      expect(update.installBlockedCrossDevice).toContain('{{tempDir}}');
      expect(update.installBlockedLocation).toContain('{{path}}');
    }
  });
});
