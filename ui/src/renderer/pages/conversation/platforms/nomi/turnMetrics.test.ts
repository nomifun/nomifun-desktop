import { describe, expect, test } from 'bun:test';

import {
  calculateCacheHitRatePercent,
  calculateContextUsagePercent,
  calculateContextUsageSegments,
  formatPercent,
  formatTokenCount,
  formatTurnDuration,
} from './turnMetrics';

describe('formatTokenCount', () => {
  test('renders small counts verbatim', () => {
    expect(formatTokenCount(0)).toBe('0');
    expect(formatTokenCount(42)).toBe('42');
    expect(formatTokenCount(999)).toBe('999');
  });

  test('renders thousands with a k suffix and one decimal', () => {
    expect(formatTokenCount(1000)).toBe('1.0k');
    expect(formatTokenCount(1234)).toBe('1.2k');
    expect(formatTokenCount(12_500)).toBe('12.5k');
  });

  test('renders millions with an m suffix', () => {
    expect(formatTokenCount(1_000_000)).toBe('1.0m');
    expect(formatTokenCount(2_300_000)).toBe('2.3m');
  });
});

describe('formatTurnDuration', () => {
  test('renders sub-second durations in milliseconds', () => {
    expect(formatTurnDuration(0)).toBe('0ms');
    expect(formatTurnDuration(840)).toBe('840ms');
  });

  test('renders seconds with one decimal under a minute', () => {
    expect(formatTurnDuration(1000)).toBe('1.0s');
    expect(formatTurnDuration(3450)).toBe('3.5s');
    expect(formatTurnDuration(59_900)).toBe('59.9s');
  });

  test('renders minutes and seconds at or above a minute', () => {
    expect(formatTurnDuration(60_000)).toBe('1m 0s');
    expect(formatTurnDuration(90_000)).toBe('1m 30s');
    expect(formatTurnDuration(605_000)).toBe('10m 5s');
  });
});

describe('calculateContextUsagePercent', () => {
  test('rounds context usage and clamps invalid inputs', () => {
    expect(calculateContextUsagePercent(81_350, 200_000)).toBe(41);
    expect(calculateContextUsagePercent(250_000, 200_000)).toBe(100);
    expect(calculateContextUsagePercent(undefined, 200_000)).toBeNull();
    expect(calculateContextUsagePercent(20_000, 0)).toBeNull();
  });
});

describe('calculateCacheHitRatePercent', () => {
  test('derives cache hit rate from readable usage counters', () => {
    expect(calculateCacheHitRatePercent({ inputTokens: 10_000, cacheReadTokens: 8_000 })).toBe(80);
    expect(calculateCacheHitRatePercent({ inputTokens: 0, cacheReadTokens: 8_000 })).toBeNull();
    expect(calculateCacheHitRatePercent({ inputTokens: 12_000 })).toBe(0);
  });
});

describe('formatPercent', () => {
  test('renders known percentages and an em dash fallback', () => {
    expect(formatPercent(80)).toBe('80%');
    expect(formatPercent(null)).toBe('—');
  });
});

describe('calculateContextUsageSegments', () => {
  test('splits context into cached, fresh, and remaining non-overlapping segments', () => {
    expect(
      calculateContextUsageSegments({
        contextTokens: 120_000,
        contextWindow: 200_000,
        cacheReadTokens: 50_000,
      })
    ).toEqual({
      cachedTokens: 50_000,
      freshTokens: 70_000,
      remainingTokens: 80_000,
      cachedPercent: 25,
      freshPercent: 35,
      remainingPercent: 40,
    });
  });

  test('clamps impossible provider values instead of overfilling the bar', () => {
    expect(
      calculateContextUsageSegments({
        contextTokens: 220_000,
        contextWindow: 200_000,
        cacheReadTokens: 250_000,
      })
    ).toEqual({
      cachedTokens: 200_000,
      freshTokens: 0,
      remainingTokens: 0,
      cachedPercent: 100,
      freshPercent: 0,
      remainingPercent: 0,
    });
  });

  test('returns null without a usable context window', () => {
    expect(calculateContextUsageSegments({ contextTokens: 10_000, contextWindow: 0 })).toBeNull();
  });
});
