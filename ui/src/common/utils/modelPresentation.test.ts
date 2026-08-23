import { describe, expect, test } from 'bun:test';

import { modelDisplayLabel, modelPresentationRawId } from './modelPresentation';

describe('modelPresentation', () => {
  test('uses a generic display name without changing the runtime id', () => {
    const raw = 'ep-20260823123456-abc123';
    expect(modelDisplayLabel(raw, 'Seedance 1.5 Pro')).toBe('Seedance 1.5 Pro');
    expect(modelPresentationRawId(raw, 'Seedance 1.5 Pro')).toBe(raw);
  });

  test('falls back to the exact model id when no display name is configured', () => {
    const raw = 'vendor/model-v1';
    expect(modelDisplayLabel(raw)).toBe(raw);
    expect(modelPresentationRawId(raw)).toBeUndefined();
  });
});
