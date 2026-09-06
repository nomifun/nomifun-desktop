import { describe, expect, test } from 'bun:test';
import {
  chmodSync,
  mkdirSync,
  mkdtempSync,
  rmSync,
  writeFileSync,
} from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';

import {
  createReleaseLock,
  writeReleaseLock,
} from '../release/release-lock.mjs';

import {
  EXPECTED_FORK_COMMIT,
  EXPECTED_PROFILES,
  EXPECTED_PROTOCOL_SCHEMA_DIGEST,
  EXPECTED_PROTOCOL_VERSION,
  EXPECTED_RPC_METHODS,
  EXPECTED_TARGET,
  TARGET_ID,
  applyMacosSidecarReleasePolicy,
  assertSelfTest,
  compareCapabilityInventory,
  parseArgs,
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

  test('keeps legacy Sidecar CLI inputs optional until a release lock requires them', () => {
    const options = parseArgs([
      '--release-lock',
      '/tmp/release-lock.json',
      '--sidecar',
      '/tmp/nomifun-codex-runtime',
      '--hello',
      '/tmp/nomifun-codex-runtime.hello.json',
      '--sidecar-dir',
      '/tmp/sidecars',
      '--run-sidecar-rpc',
    ]);

    expect(options).toEqual(
      expect.objectContaining({
        sidecar: '/tmp/nomifun-codex-runtime',
        hello: '/tmp/nomifun-codex-runtime.hello.json',
        sidecarDir: '/tmp/sidecars',
        runSidecarRpc: true,
        credentialFile: null,
      }),
    );
  });

  test('marks Sidecar validation not_required for a Host and Package only Nomi-core lock', () => {
    const report = {
      checks: [],
      failures: [],
      blockers: [],
      artifacts: {},
    };
    const lock = {
      schema_version: '1.0.0',
      source_commit: 'a'.repeat(40),
      platform: EXPECTED_TARGET,
      host: { path: 'NomiFun.app/Contents/MacOS/nomifun-desktop', sha256: 'b'.repeat(64) },
      sidecars: {},
      helpers: [],
      package: { path: 'NomiFun.dmg', sha256: 'c'.repeat(64) },
      legal: [],
    };

    const policy = applyMacosSidecarReleasePolicy(report, lock, {
      sidecar: '/tmp/legacy-sidecar',
      hello: '/tmp/legacy-sidecar.hello.json',
      sidecarDir: '/tmp/legacy-sidecars',
      runSidecarRpc: true,
    });

    expect(policy).toEqual({ status: 'not_required', artifact: null });
    expect(report.failures).toEqual([]);
    expect(report.blockers).toEqual([]);
    expect(report.artifacts).toEqual({});
    expect(report.checks).toEqual([
      expect.objectContaining({
        id: 'release-lock:arm64-sidecar',
        status: 'not_required',
        target: TARGET_ID,
        available_targets: [],
        ignored_optional_inputs: [
          '--sidecar',
          '--hello',
          '--sidecar-dir',
          '--run-sidecar-rpc',
        ],
      }),
      expect.objectContaining({
        id: 'sidecar:credential',
        status: 'not_required',
      }),
      expect.objectContaining({
        id: 'sidecar:artifact-target-permissions',
        status: 'not_required',
      }),
      expect.objectContaining({
        id: 'sidecar:release-lock-sha256',
        status: 'not_required',
      }),
      expect.objectContaining({
        id: 'sidecar:native-arm64',
        status: 'not_required',
      }),
      expect.objectContaining({
        id: 'sidecar:hello-profile-rpc-contract',
        status: 'not_required',
      }),
      expect.objectContaining({
        id: 'sidecar:live-hello-rpc',
        status: 'not_required',
      }),
      expect.objectContaining({
        id: 'sidecar:process-cleanup',
        status: 'not_required',
      }),
    ]);
  });

  test('continues required native stages for a verified release lock without a Sidecar', async () => {
    const root = mkdtempSync(join(tmpdir(), 'nomifun-macos-native-'));
    try {
      const sourceCommit = 'd'.repeat(40);
      const host = join(root, 'NomiFun.app', 'Contents', 'MacOS', 'nomifun-desktop');
      const packagePath = join(root, 'NomiFun.dmg');
      const releaseLock = join(root, 'release-lock.json');
      const missingNomiCore = join(root, 'missing-nomicore');
      mkdirSync(join(root, 'NomiFun.app', 'Contents', 'MacOS'), { recursive: true });
      writeFileSync(host, 'host fixture');
      chmodSync(host, 0o555);
      writeFileSync(packagePath, 'package fixture');
      const lock = createReleaseLock({
        root,
        sourceCommit,
        platform: EXPECTED_TARGET,
        host,
        sidecars: {},
        packagePath,
      });
      writeReleaseLock(releaseLock, lock);

      const commands = [];
      const command = (name, args = []) => {
        commands.push([name, ...args]);
        let stdout = '';
        if (name === 'uname') stdout = 'Darwin arm64\n';
        if (name === 'sysctl') stdout = '0\n';
        if (name === 'rustc') stdout = `host: ${EXPECTED_TARGET}\n`;
        if (name === 'git' && args[0] === 'rev-parse') stdout = `${sourceCommit}\n`;
        if (name === 'lipo') stdout = 'arm64\n';
        return {
          command: [name, ...args].join(' '),
          status: 0,
          stdout,
          stderr: '',
          error: null,
          timedOut: false,
        };
      };

      const result = await runValidation(
        {
          releaseLock,
          artifactRoot: root,
          capabilityInventory: null,
          sidecar: '/tmp/legacy-sidecar',
          hello: '/tmp/legacy-sidecar.hello.json',
          sidecarDir: '/tmp/legacy-sidecars',
          app: null,
          dmg: null,
          hostBinary: missingNomiCore,
          endpoint: null,
          bindingId: null,
          token: null,
          credentialFile: null,
          report: null,
          logs: [],
          runStartup: false,
          runLifecycle: false,
          runSidecarRpc: true,
        },
        {
          platform: 'darwin',
          arch: 'arm64',
          command,
          validatePathShape: (path) => ({ status: 'pass', path, mode: '100555' }),
        },
      );

      expect(result.checks).toEqual(
        expect.arrayContaining([
          expect.objectContaining({ id: 'native-host', status: 'pass' }),
          expect.objectContaining({ id: 'release-lock:real-artifacts', status: 'pass' }),
          expect.objectContaining({ id: 'release-lock:arm64-sidecar', status: 'not_required' }),
          expect.objectContaining({ id: 'macos-app:architectures', status: 'pass' }),
          expect.objectContaining({ id: 'macos-package:hdiutil-verify', status: 'pass' }),
          expect.objectContaining({ id: 'canonical-capability-inventory', status: 'pass' }),
          expect.objectContaining({ id: 'startup:host-binary', status: 'blocked' }),
          expect.objectContaining({
            id: 'lifecycle:open-ready-turn-observe-cancel-dispose',
            status: 'blocked',
          }),
        ]),
      );
      expect(result.failures.some((entry) => entry.id.startsWith('sidecar:'))).toBe(false);
      expect(result.blockers.some((entry) => entry.toLowerCase().includes('sidecar'))).toBe(false);
      expect(result.artifacts).not.toHaveProperty('sidecar');
      expect(commands.some(([name]) => name === 'hdiutil')).toBe(true);
      expect(commands.some(([name]) => name === 'codesign')).toBe(true);
      expect(commands.some(([name]) => name === 'file')).toBe(false);
    } finally {
      rmSync(root, { recursive: true, force: true });
    }
  });
});
