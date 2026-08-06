/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { describe, expect, test } from 'bun:test';
import enNomi from './locales/en-US/nomi.json';
import zhNomi from './locales/zh-CN/nomi.json';

type LocaleJson = Record<string, unknown>;

/**
 * One key per `RobotStatusDto.phase` the backend can serialize. The Rust enum,
 * the `IApiRobotPhase` literals, `ROBOT_STATUS_COLOR` and this list must grow
 * together — a phase with no label renders as a raw key in the list pill.
 */
const ROBOT_PHASE_KEYS = [
  'robot.status.offline',
  'robot.status.idle',
  'robot.status.listening',
  'robot.status.speaking',
] as const;

/** Copy carrying placeholders the section interpolates. */
const ROBOT_INTERPOLATED: readonly [string, string][] = [
  ['robot.hint', '{{companionName}}'],
  ['robot.claimOk', '{{companionName}}'],
  ['robot.board', '{{board}}'],
  ['robot.firmware', '{{version}}'],
  ['robot.lastSeen', '{{time}}'],
];

function getLocaleValue(locale: LocaleJson, key: string): unknown {
  let cursor: unknown = locale;
  for (const segment of key.split('.')) {
    if (
      !cursor ||
      typeof cursor !== 'object' ||
      !Object.prototype.hasOwnProperty.call(cursor, segment)
    ) {
      return undefined;
    }
    cursor = (cursor as LocaleJson)[segment];
  }
  return cursor;
}

describe('robot locale coverage', () => {
  test('every robot phase has a label in both locales', () => {
    const failures: string[] = [];
    for (const [name, locale] of [
      ['en-US', enNomi as unknown as LocaleJson],
      ['zh-CN', zhNomi as unknown as LocaleJson],
    ] as const) {
      for (const key of ROBOT_PHASE_KEYS) {
        const value = getLocaleValue(locale, key);
        if (typeof value !== 'string' || !value.trim()) failures.push(`${name} nomi.${key}`);
      }
    }
    expect(failures).toEqual([]);
  });

  test('interpolated robot copy keeps its placeholders in both locales', () => {
    const failures: string[] = [];
    for (const [name, locale] of [
      ['en-US', enNomi as unknown as LocaleJson],
      ['zh-CN', zhNomi as unknown as LocaleJson],
    ] as const) {
      for (const [key, placeholder] of ROBOT_INTERPOLATED) {
        if (!String(getLocaleValue(locale, key)).includes(placeholder)) {
          failures.push(`${name} nomi.${key} lost ${placeholder}`);
        }
      }
    }
    expect(failures).toEqual([]);
  });

  test('the IM section names its own kind of bot so the two never collide', () => {
    for (const locale of [enNomi as unknown as LocaleJson, zhNomi as unknown as LocaleJson]) {
      const createBot = String(getLocaleValue(locale, 'settings.remoteCreateBot')).toUpperCase();
      const botIdentity = String(getLocaleValue(locale, 'settings.remoteBotIdentity')).toUpperCase();
      expect(createBot.includes('IM')).toBe(true);
      expect(botIdentity.includes('IM')).toBe(true);
    }
  });
});
