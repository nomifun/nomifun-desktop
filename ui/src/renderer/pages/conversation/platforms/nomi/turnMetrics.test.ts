import { describe, expect, test } from 'bun:test';

import { formatTokenCount } from './turnMetrics';

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
