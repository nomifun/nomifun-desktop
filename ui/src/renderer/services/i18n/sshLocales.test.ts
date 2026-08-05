/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { describe, expect, test } from 'bun:test';
import enSsh from './locales/en-US/ssh.json';
import zhSsh from './locales/zh-CN/ssh.json';

type LocaleJson = Record<string, unknown>;

/**
 * One key per `SshLinkPhase` the backend can serialize. The Rust enum, the
 * `ISshLinkPhase` literals, `SSH_STATUS_COLOR` and this list must grow together
 * — a phase with no label renders as a raw key in the header pill.
 */
const SSH_PHASE_KEYS = [
  'status.idle',
  'status.connecting',
  'status.connected',
  'status.degraded',
  'status.reconnecting',
  'status.dropped',
  'status.closed',
] as const;

/** Conversation-header host pill (label + popover rows). */
const SSH_PILL_KEYS = [
  'pill.endpoint',
  'pill.hostKey',
  'pill.hostKeyUnpinned',
  'pill.sudoStored',
  'pill.sudoMissing',
  'pill.attempt',
  'pill.retryIn',
  'pill.detail',
  'pill.noLink',
  'pill.droppedHint',
  'pill.unconfirmedExit',
] as const;

function getLocaleValue(locale: LocaleJson, key: string): unknown {
  if (Object.prototype.hasOwnProperty.call(locale, key)) return locale[key];

  let cursor: unknown = locale;
  for (const segment of key.split('.')) {
    if (!cursor || typeof cursor !== 'object' || !Object.prototype.hasOwnProperty.call(cursor, segment)) {
      return undefined;
    }
    cursor = (cursor as LocaleJson)[segment];
  }
  return cursor;
}

function assertStringKeys(localeName: string, locale: LocaleJson, keys: readonly string[]) {
  const failures: string[] = [];
  for (const key of keys) {
    const value = getLocaleValue(locale, key);
    if (value === undefined) {
      failures.push(`${localeName} missing ssh.${key}`);
    } else if (typeof value !== 'string') {
      failures.push(`${localeName} ssh.${key} should be a string`);
    } else if (!value.trim()) {
      failures.push(`${localeName} ssh.${key} should not be blank`);
    }
  }
  expect(failures).toEqual([]);
}

describe('ssh locale coverage', () => {
  test('every link phase has a label in both locales', () => {
    assertStringKeys('en-US ssh', enSsh as LocaleJson, SSH_PHASE_KEYS);
    assertStringKeys('zh-CN ssh', zhSsh as LocaleJson, SSH_PHASE_KEYS);
  });

  test('the conversation-header host pill has complete copy in both locales', () => {
    assertStringKeys('en-US ssh', enSsh as LocaleJson, SSH_PILL_KEYS);
    assertStringKeys('zh-CN ssh', zhSsh as LocaleJson, SSH_PILL_KEYS);
  });

  test('interpolated pill copy keeps its placeholders in both locales', () => {
    const failures: string[] = [];
    for (const [name, locale] of [
      ['en-US', enSsh as LocaleJson],
      ['zh-CN', zhSsh as LocaleJson],
    ] as const) {
      for (const [key, placeholder] of [
        ['pill.attempt', '{{attempt}}'],
        ['pill.retryIn', '{{seconds}}'],
      ] as const) {
        if (!String(getLocaleValue(locale, key)).includes(placeholder)) {
          failures.push(`${name} ssh.${key} lost ${placeholder}`);
        }
      }
    }
    expect(failures).toEqual([]);
  });
});
