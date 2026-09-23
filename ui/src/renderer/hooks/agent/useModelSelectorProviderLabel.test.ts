import { describe, expect, test } from 'bun:test';

import { formatModelSelectorProviderLabel } from './useModelSelectorProviderLabel';

describe('formatModelSelectorProviderLabel', () => {
  test('preserves provider names and falls back to the platform', () => {
    expect(formatModelSelectorProviderLabel({ name: 'OpenAI', platform: 'openai' })).toBe('OpenAI');
    expect(formatModelSelectorProviderLabel({ platform: 'anthropic' })).toBe('anthropic');
  });
});
