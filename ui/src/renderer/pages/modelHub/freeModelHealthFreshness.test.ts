/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { describe, expect, test } from 'bun:test';

/**
 * A health probe is one short request; it cannot promise that a model will carry
 * an agent turn moments later. Nothing writes a real turn's rate limit back into
 * the stored health, so a model observed healthy once stayed green forever — one
 * was still reading "available / 1770ms" after repeated real rate limits, while
 * users kept retrying against a badge that could not go stale.
 *
 * Mirrors isHealthResultStale in FreeModelsContent.tsx.
 */

const HEALTH_FRESHNESS_MS = 5 * 60 * 1000;

interface HealthResult {
  status: 'healthy' | 'unhealthy' | 'unknown';
  checkedAt?: number;
  latencyMs?: number | null;
}

const isHealthResultStale = (result: HealthResult | undefined, now: number): boolean =>
  result?.status === 'healthy' &&
  typeof result.checkedAt === 'number' &&
  now - result.checkedAt > HEALTH_FRESHNESS_MS;

const NOW = 1_700_000_000_000;

describe('free model health freshness', () => {
  test('a probe from moments ago is still trusted', () => {
    expect(
      isHealthResultStale({ status: 'healthy', checkedAt: NOW - 1_000, latencyMs: 1770 }, NOW)
    ).toBe(false);
  });

  test('the observed case — a healthy probe left to age — goes stale', () => {
    // The reported badge survived a full agent run plus several retries.
    expect(
      isHealthResultStale({ status: 'healthy', checkedAt: NOW - 30 * 60_000, latencyMs: 1770 }, NOW)
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
