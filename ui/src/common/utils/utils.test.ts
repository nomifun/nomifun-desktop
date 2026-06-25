/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { describe, expect, test } from 'bun:test';
import { resolveLocaleKey } from './utils';

describe('resolveLocaleKey', () => {
  test('only resolves supported app language families to Chinese or English locale keys', () => {
    expect(resolveLocaleKey('zh-CN')).toBe('zh-CN');
    expect(resolveLocaleKey('zh')).toBe('zh-CN');
    expect(resolveLocaleKey('en-US')).toBe('en-US');
    expect(resolveLocaleKey('ja-JP')).toBe('en-US');
    expect(resolveLocaleKey('ko-KR')).toBe('en-US');
    expect(resolveLocaleKey('tr-TR')).toBe('en-US');
    expect(resolveLocaleKey('ru-RU')).toBe('en-US');
    expect(resolveLocaleKey('uk-UA')).toBe('en-US');
  });
});
