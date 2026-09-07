/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { describe, expect, test } from 'bun:test';

import { getContentTypeByExtension, isTextFile } from './fileUtils';

describe('HTML preview classification', () => {
  test('classifies HTML by extension regardless of basename or path separator', () => {
    expect(getContentTypeByExtension('index.html')).toBe('html');
    expect(getContentTypeByExtension('/workspace/miniapp.html')).toBe('html');
    expect(getContentTypeByExtension(String.raw`C:\workspace\MINIAPP.HTML`)).toBe('html');
    expect(getContentTypeByExtension('document.htm')).toBe('html');
  });

  test('keeps ordinary HTML files editable as text previews', () => {
    expect(isTextFile('/workspace/page.html')).toBe(true);
  });
});
