import { describe, expect, test } from 'bun:test';
import { isNewApiPlatform } from './platformConstants';

describe('isNewApiPlatform', () => {
  test('identifies only the custom New API gateway preset', () => {
    expect(isNewApiPlatform('new-api')).toBe(true);
    expect(isNewApiPlatform('custom')).toBe(false);
  });
});
