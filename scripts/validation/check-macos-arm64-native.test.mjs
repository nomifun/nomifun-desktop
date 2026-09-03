import { describe, expect, test } from 'bun:test';

import {
  EXPECTED_FORK_COMMIT,
  EXPECTED_PROFILES,
  EXPECTED_PROTOCOL_SCHEMA_DIGEST,
  EXPECTED_PROTOCOL_VERSION,
  EXPECTED_RPC_METHODS,
  EXPECTED_TARGET,
  TARGET_ID,
  assertSelfTest,
  compareCapabilityInventory,
  readCanonicalCapabilityIds,
  readCanonicalCapabilityInventory,
  runValidation,
  validateHelloPayload,
} from './check-macos-arm64-native.mjs';

describe('C8-MA macOS arm64 validation helper', () => {
  test('self-test covers regular files, missing paths, and symlink rejection', () => {
    expect(assertSelfTest()).toEqual({ status: 'pass' });
  });

  test('rejects a hello payload that advertises an experimental RPC', () => {
    const result = validateHelloPayload({
      runtime_release_digest: 'a'.repeat(64),
      runtime_build_digest: 'b'.repeat(64),
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

  test('validates release digests by shape without pinning a fixture value', () => {
    const result = validateHelloPayload({
      runtime_release_digest: 'c'.repeat(64),
      runtime_build_digest: 'd'.repeat(64),
      fork_commit: EXPECTED_FORK_COMMIT,
      tracked_upstream_commit: EXPECTED_FORK_COMMIT,
      protocol_version: EXPECTED_PROTOCOL_VERSION,
      protocol_schema_digest: EXPECTED_PROTOCOL_SCHEMA_DIGEST,
      runtime_target: EXPECTED_TARGET,
      supported_profiles: EXPECTED_PROFILES,
      full_auto: { ask_for_approval: 'never', sandbox_policy: 'danger-full-access' },
      rpc_allowlist: {
        methods: EXPECTED_RPC_METHODS,
        experimental_methods: [],
      },
    });
    expect(result).toEqual({ status: 'pass', mismatches: [] });
  });

  test('loads the canonical capability ID set from the generated inventory', () => {
    const inventory = readCanonicalCapabilityInventory();
    const ids = readCanonicalCapabilityIds();

    expect(inventory.packageCount).toBeGreaterThan(0);
    expect(inventory.capabilityIds).toEqual(ids);
    expect(ids.has('browser.render_content')).toBe(true);
  });

  test('compares the live catalog by exact canonical ID set', () => {
    const canonical = new Set(['alpha.capability', 'browser.render_content']);
    const passing = compareCapabilityInventory({
      success: true,
      data: [
        { capability: { id: 'browser.render_content' } },
        { capability: { id: 'alpha.capability' } },
      ],
    }, canonical);
    expect(passing).toEqual(expect.objectContaining({
      status: 'pass',
      expectedCount: 2,
      observedCount: 2,
      missing: [],
      unexpected: [],
      duplicates: [],
      malformed: [],
    }));

    const failing = compareCapabilityInventory({
      success: true,
      data: [
        { capability: { id: 'alpha.capability' } },
        { capability: { id: 'alpha.capability' } },
        { capability: { id: 'unexpected.capability' } },
        {},
      ],
    }, canonical);
    expect(failing.status).toBe('fail');
    expect(failing.missing).toEqual(['browser.render_content']);
    expect(failing.unexpected).toEqual(['unexpected.capability']);
    expect(failing.duplicates).toEqual(['alpha.capability']);
    expect(failing.malformed).toEqual(['response.data[3].capability.id']);
  });

  test('emits a blocked platform-result shape when the real release lock is missing', async () => {
    const result = await runValidation({ releaseLock: null, logs: [] });
    expect(result).toEqual(
      expect.objectContaining({
        schema_version: '1.0.0',
        source_commit: null,
        platform: null,
        target: TARGET_ID,
        status: expect.stringMatching(/^(?:blocked|fail)$/),
        release_lock: null,
        logs: [{ kind: 'embedded_checks', reference: '#/checks' }],
      }),
    );
    expect(result.suite).toEqual(
      expect.objectContaining({
        name: 'macos-arm64-native',
        checks: expect.arrayContaining(['native-host', 'release-lock']),
      }),
    );
    expect(result.checks).toContainEqual(
      expect.objectContaining({
        id: 'release-lock',
        status: 'blocked',
      }),
    );
  });
});
