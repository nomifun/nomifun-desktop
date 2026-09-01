import { describe, expect, test } from 'bun:test';

import {
  EXPECTED_FORK_COMMIT,
  EXPECTED_PROFILES,
  EXPECTED_PROTOCOL_SCHEMA_DIGEST,
  EXPECTED_PROTOCOL_VERSION,
  EXPECTED_RPC_METHODS,
  EXPECTED_RUNTIME_RELEASE_DIGEST,
  EXPECTED_TARGET,
  assertSelfTest,
  validateHelloPayload,
} from './check-macos-arm64-native.mjs';

describe('C8-MA macOS arm64 validation helper', () => {
  test('self-test covers regular files, missing paths, and symlink rejection', () => {
    expect(assertSelfTest()).toEqual({ status: 'pass' });
  });

  test('rejects a hello payload that advertises an experimental RPC', () => {
    const result = validateHelloPayload({
      runtime_release_digest: EXPECTED_RUNTIME_RELEASE_DIGEST,
      fork_commit: EXPECTED_FORK_COMMIT,
      tracked_upstream_commit: EXPECTED_FORK_COMMIT,
      protocol_version: EXPECTED_PROTOCOL_VERSION,
      protocol_schema_digest: EXPECTED_PROTOCOL_SCHEMA_DIGEST,
      runtime_target: EXPECTED_TARGET,
      supported_profiles: EXPECTED_PROFILES,
      full_auto: { ask_for_approval: 'never', sandbox_policy: 'danger-full-access' },
      rpc_allowlist: {
        methods: EXPECTED_RPC_METHODS,
        experimental_methods: ['debug/unsafe'],
      },
    });
    expect(result.status).toBe('fail');
    expect(result.mismatches).toContain('rpc_allowlist.experimental_methods');
  });
});
