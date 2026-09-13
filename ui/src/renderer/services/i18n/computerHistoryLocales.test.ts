/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { describe, expect, test } from 'bun:test';
import enComputerHistory from './locales/en-US/computerHistory.json';
import zhComputerHistory from './locales/zh-CN/computerHistory.json';

type LocaleJson = Record<string, unknown>;

/**
 * Every computerHistory.* key the settings page and timeline render directly.
 * The backend `computer_history_*` capability family, the
 * `IComputerHistoryStatus` literals and this list must grow together — a key
 * missing here renders as a raw dot-path in the panel.
 */
const COMPUTER_HISTORY_KEYS = [
  'title',
  'description',
  'navTitle',
  'enableLabel',
  'enableHint',
  'statusTitle',
  'statusStateLabel',
  'statusStateRunning',
  'statusStateStopped',
  'statusStatePaused',
  'statusPermissionLabel',
  'statusPermissionGranted',
  'statusPermissionDenied',
  'statusPermissionUnknown',
  'statusStorageLabel',
  'statusStorageUsage',
  'statusStoragePath',
  'statusChatAnalytics',
  'statusChatAnalyticsAvailable',
  'statusChatAnalyticsUnavailable',
  'statusRefresh',
  'permissionTitle',
  'permissionNeeded',
  'permissionHowTo',
  'permissionOpenSettings',
  'retentionTitle',
  'retentionHint',
  'storagePurge',
  'purgeConfirmTitle',
  'purgeConfirmBody',
  'purgeConfirmAction',
  'purgeCancel',
  'timelineTitle',
  'timelineEmpty',
  'windowToday',
  'windowYesterday',
  'windowLast7Days',
  'windowThisWeek',
  'topAppsTitle',
  'segmentColumnApp',
  'segmentColumnTitle',
  'segmentColumnUrl',
  'segmentColumnTime',
  'durationMinutes',
  'durationHoursMinutes',
  'errorLoadFailed',
  'errorSaveFailed',
  'errorPurgeFailed',
] as const;

function getLocaleValue(locale: LocaleJson, key: string): unknown {
  return Object.prototype.hasOwnProperty.call(locale, key) ? locale[key] : undefined;
}

function assertStringKeys(localeName: string, locale: LocaleJson, keys: readonly string[]) {
  const failures: string[] = [];
  for (const key of keys) {
    const value = getLocaleValue(locale, key);
    if (value === undefined) {
      failures.push(`${localeName} missing computerHistory.${key}`);
    } else if (typeof value !== 'string') {
      failures.push(`${localeName} computerHistory.${key} should be a string`);
    } else if (!value.trim()) {
      failures.push(`${localeName} computerHistory.${key} should not be blank`);
    }
  }
  expect(failures).toEqual([]);
}

describe('computerHistory locale coverage', () => {
  test('every settings + timeline key exists in both locales', () => {
    assertStringKeys('en-US computerHistory', enComputerHistory as LocaleJson, COMPUTER_HISTORY_KEYS);
    assertStringKeys('zh-CN computerHistory', zhComputerHistory as LocaleJson, COMPUTER_HISTORY_KEYS);
  });

  test('interpolated copy keeps its placeholders in both locales', () => {
    const failures: string[] = [];
    for (const [name, locale] of [
      ['en-US', enComputerHistory as LocaleJson],
      ['zh-CN', zhComputerHistory as LocaleJson],
    ] as const) {
      for (const [key, placeholder] of [
        ['statusStorageUsage', '{{segments}}'],
        ['statusStorageUsage', '{{bytes}}'],
        ['retentionHint', '{{days}}'],
        ['purgeConfirmBody', '{{count}}'],
        ['durationMinutes', '{{count}}'],
        ['durationHoursMinutes', '{{hours}}'],
        ['durationHoursMinutes', '{{minutes}}'],
      ] as const) {
        if (!String(getLocaleValue(locale, key)).includes(placeholder)) {
          failures.push(`${name} computerHistory.${key} lost ${placeholder}`);
        }
      }
    }
    expect(failures).toEqual([]);
  });

  test('both locales carry the same key set (no cross-locale drift)', () => {
    expect(Object.keys(enComputerHistory as LocaleJson).sort()).toEqual(
      Object.keys(zhComputerHistory as LocaleJson).sort()
    );
  });
});
