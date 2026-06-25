/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { describe, expect, test } from 'bun:test';

import { getSessionAgeDays } from './sessionAge';

const DAY_MS = 24 * 60 * 60 * 1000;

describe('getSessionAgeDays', () => {
  test('returns 0 for sessions created less than one day ago', () => {
    expect(getSessionAgeDays(10_000, 10_000 + DAY_MS - 1)).toBe(0);
  });

  test('floors whole elapsed days since creation', () => {
    expect(getSessionAgeDays(10_000, 10_000 + DAY_MS * 3 + 123)).toBe(3);
  });

  test('clamps future timestamps to today', () => {
    expect(getSessionAgeDays(20_000, 10_000)).toBe(0);
  });

  test('returns null for missing timestamps', () => {
    expect(getSessionAgeDays(0, 10_000)).toBeNull();
    expect(getSessionAgeDays(Number.NaN, 10_000)).toBeNull();
  });
});
