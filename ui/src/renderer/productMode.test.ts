import { describe, expect, test } from 'bun:test';
import { resolveProductMode } from './productMode';

describe('product mode', () => {
  test('enables the sales shell only for an explicit sales value', () => {
    expect(resolveProductMode('sales')).toBe('sales');
    expect(resolveProductMode(' SALES ')).toBe('sales');
  });

  test('keeps the ordinary NomiFun product as the safe default', () => {
    expect(resolveProductMode(undefined)).toBe('default');
    expect(resolveProductMode('')).toBe('default');
    expect(resolveProductMode('creative')).toBe('default');
  });
});

