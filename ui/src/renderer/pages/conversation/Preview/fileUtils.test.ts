/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { describe, expect, test } from 'bun:test';

import { getContentTypeByExtension } from './fileUtils';

describe('HTML preview classification', () => {
  test('classifies HTML by extension regardless of basename or path separator', () => {
    expect(getContentTypeByExtension('index.html')).toBe('html');
    expect(getContentTypeByExtension('/workspace/plugin.html')).toBe('html');
    expect(getContentTypeByExtension(String.raw`C:\workspace\PLUGIN.HTML`)).toBe('html');
    expect(getContentTypeByExtension('document.htm')).toBe('html');
  });
});
