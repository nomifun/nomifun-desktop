/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { describe, expect, test } from 'bun:test';
import { parseEntityId } from '@/common/types/ids';
import { prefixedId, shortId } from './prefixedId';

const CANONICAL_UUID_V7 =
  /^[0-9a-f]{8}-[0-9a-f]{4}-7[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/;

describe('prefixedId', () => {
  test('mints the same canonical UUIDv7 contract accepted at entity boundaries', () => {
    const id = prefixedId('prov');
    expect(parseEntityId('provider', id)).toBe(id);
    expect(CANONICAL_UUID_V7.test(id.slice('prov_'.length))).toBe(true);
  });

  test('untyped UUID helper still returns a full canonical UUIDv7', () => {
    expect(CANONICAL_UUID_V7.test(shortId())).toBe(true);
  });

  test('rejects non-canonical prefixes', () => {
    for (const prefix of ['', 'Conv', 'bad_prefix', 'bad-prefix', '1bad', 'a'.repeat(33)]) {
      let error: unknown;
      try {
        prefixedId(prefix);
      } catch (caught) {
        error = caught;
      }
      expect(error instanceof TypeError).toBe(true);
    }
  });
});
