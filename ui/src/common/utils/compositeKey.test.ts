/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { describe, expect, test } from 'bun:test';
import { compositeKey } from './compositeKey';

describe('compositeKey', () => {
  test('does not collide when tuple boundaries differ', () => {
    expect(compositeKey('ab', 'c') === compositeKey('a', 'bc')).toBe(false);
  });

  test('is deterministic', () => {
    expect(compositeKey('provider', 'model')).toBe(compositeKey('provider', 'model'));
  });
});
