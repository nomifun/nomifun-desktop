import '../../../../../test/setup-dom.ts';
import { describe, expect, test } from 'bun:test';
import { runtimeEngineOptions } from './GuidRuntimeEngineSelector';

describe('runtime engine catalog options', () => {
  test('discovers arbitrary families and profiles with exact build identity', () => {
    const descriptor = {
      family_id: 'customer.workflow', build_id: 'build-42', build_digest: 'a'.repeat(64),
      host_contract_version: 1, display_name: 'Custom runtime', supported_profiles: ['review', 'workflow'],
    };
    const options = runtimeEngineOptions([descriptor]);
    expect(options.map((option) => option.selection.profile)).toEqual(['review', 'workflow']);
    expect(options[0].selection.selector).toEqual({
      selection: 'exact', family_id: descriptor.family_id, build_id: descriptor.build_id, build_digest: descriptor.build_digest,
    });
    expect(new Set(options.map((option) => option.value)).size).toBe(2);
    expect(runtimeEngineOptions([])).toEqual([]);
  });
});
