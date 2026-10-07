import { describe, expect, test } from 'bun:test';
import { parseProviderId } from '@/common/types/ids';
import {
  modelCapabilityConfigurationRoute,
  modelConfigurationTarget,
  withoutModelConfigurationTarget,
} from './modelConfigurationRoute';

describe('model capability configuration route', () => {
  test('round-trips an exact provider and model target', () => {
    const providerId = parseProviderId('0190f5fe-7c00-7a00-8000-000000000105');
    const route = modelCapabilityConfigurationRoute(providerId, 'model name/preview');
    const query = new URLSearchParams(route.slice(route.indexOf('?') + 1));

    expect(route.startsWith('/models?section=models&')).toBe(true);
    expect(modelConfigurationTarget(query)).toEqual({
      providerId,
      model: 'model name/preview',
    });
  });

  test('ignores incomplete targets and removes only target parameters', () => {
    const incomplete = new URLSearchParams('section=models&provider=provider-only&focus=capabilities');
    expect(modelConfigurationTarget(incomplete)).toBeNull();

    const consumed = withoutModelConfigurationTarget(
      new URLSearchParams('section=models&provider=p&model=m&focus=capabilities&from=guid'),
    );
    expect(consumed.toString()).toBe('section=models&from=guid');
  });
});
