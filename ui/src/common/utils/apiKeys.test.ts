/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { describe, expect, test } from 'bun:test';
import { parseApiKeyList } from './apiKeys';

describe('API key list helpers', () => {
  test('parses comma separated API keys', () => {
    expect(parseApiKeyList('key-a, key-b,,key-c')).toEqual(['key-a', 'key-b', 'key-c']);
  });

  test('parses newline separated API keys', () => {
    expect(parseApiKeyList('key-a\nkey-b\r\n key-c ')).toEqual(['key-a', 'key-b', 'key-c']);
  });
});
