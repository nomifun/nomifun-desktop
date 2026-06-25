/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { describe, expect, test } from 'bun:test';
import { DEFAULT_LANGUAGE, SUPPORTED_LANGUAGES, normalizeLanguageCode } from './i18n';

describe('i18n language support', () => {
  test('only exposes simplified Chinese and English as supported app languages', () => {
    expect(SUPPORTED_LANGUAGES).toEqual(['zh-CN', 'en-US']);
    expect(DEFAULT_LANGUAGE).toBe('en-US');
  });

  test('normalizes removed locales away from their old language codes', () => {
    expect(normalizeLanguageCode('zh-TW')).toBe('zh-CN');
    expect(normalizeLanguageCode('ja-JP')).toBe(DEFAULT_LANGUAGE);
    expect(normalizeLanguageCode('ko-KR')).toBe(DEFAULT_LANGUAGE);
    expect(normalizeLanguageCode('tr-TR')).toBe(DEFAULT_LANGUAGE);
    expect(normalizeLanguageCode('ru-RU')).toBe(DEFAULT_LANGUAGE);
    expect(normalizeLanguageCode('uk-UA')).toBe(DEFAULT_LANGUAGE);
  });
});
