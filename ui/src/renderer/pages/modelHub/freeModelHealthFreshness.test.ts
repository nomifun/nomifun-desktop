/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { describe, expect, test } from 'bun:test';

import { HEALTH_FRESHNESS_MS, isHealthResultStale } from './freeModelHealth';

const NOW = 1_700_000_000_000;

describe('free model health freshness', () => {
  test('a probe from moments ago is still trusted', () => {
    expect(
      isHealthResultStale({ status: 'healthy', checkedAt: NOW - 1_000 }, NOW)
    ).toBe(false);
  });

  test('the observed case — a healthy probe left to age — goes stale', () => {
    // The reported badge survived a full agent run plus several retries.
    expect(
      isHealthResultStale({ status: 'healthy', checkedAt: NOW - 30 * 60_000 }, NOW)
    ).toBe(true);
  });

  test('staleness applies only to healthy claims', () => {
    // An unhealthy or unknown badge asserts no capability, so ageing it would
    // only add noise.
    for (const status of ['unhealthy', 'unknown'] as const) {
      expect(isHealthResultStale({ status, checkedAt: NOW - 60 * 60_000 }, NOW)).toBe(false);
    }
  });

  test('a result without a timestamp is not silently treated as fresh or stale', () => {
    // checkedAt is non-optional in the wire type; a malformed payload must not
    // flip the badge either way.
    expect(isHealthResultStale({ status: 'healthy' }, NOW)).toBe(false);
    expect(isHealthResultStale(undefined, NOW)).toBe(false);
  });

  test('the boundary is exclusive so a just-inside probe stays trusted', () => {
    expect(isHealthResultStale({ status: 'healthy', checkedAt: NOW - HEALTH_FRESHNESS_MS }, NOW)).toBe(
      false
    );
    expect(
      isHealthResultStale({ status: 'healthy', checkedAt: NOW - HEALTH_FRESHNESS_MS - 1 }, NOW)
    ).toBe(true);
  });
});
