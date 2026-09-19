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
  CEF_HELPER_NAMES,
  EXPECTED_TARGET,
  TARGET_ID,
  assertSelfTest,
  compareCapabilityInventory,
  parseArgs,
  readCanonicalCapabilityIds,
  readCanonicalCapabilityInventory,
  runValidation,
} from './check-macos-arm64-native.mjs';

describe('macOS arm64 host validation helper', () => {
  test('rejects retired executor flags instead of ignoring or launching them', () => {
    for (const flag of ['--sidecar', '--hello', '--sidecar-dir', '--credential-file', '--run-sidecar-rpc']) {
      expect(() => parseArgs([flag])).toThrow('retired');
      expect(() => parseArgs([`${flag}=unused`])).toThrow('retired');
    }
  });

  test('rejects direct legacy options before invoking host commands', async () => {
    for (const key of ['sidecar', 'hello', 'sidecarDir', 'credentialFile', 'runSidecarRpc']) {
      let calls = 0;
      await expect(runValidation({ [key]: null }, {
        command: () => { calls += 1; throw new Error('must not run'); },
      })).rejects.toThrow('retired');
      expect(calls).toBe(0);
    }
  });
  test('self-test covers regular files, missing paths, and symlink rejection', () => {
    expect(assertSelfTest()).toEqual({ status: 'pass' });
  });



  test('loads the canonical capability ID set from the generated inventory', () => {
    const inventory = readCanonicalCapabilityInventory();
    const ids = readCanonicalCapabilityIds();

    expect(inventory.packageCount).toBeGreaterThan(0);
    expect(inventory.capabilityIds).toEqual(ids);
    expect(ids.has('browser')).toBe(true);
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
      const frameworks = join(root, 'NomiFun.app', 'Contents', 'Frameworks');
      const cefFramework = join(frameworks, 'Chromium Embedded Framework.framework');
      mkdirSync(cefFramework, { recursive: true });
      const cefBinary = join(cefFramework, 'Chromium Embedded Framework');
      writeFileSync(cefBinary, 'cef fixture');
      chmodSync(cefBinary, 0o555);
      for (const name of CEF_HELPER_NAMES) {
        const helper = join(frameworks, `${name}.app`, 'Contents', 'MacOS', name);
        mkdirSync(join(frameworks, `${name}.app`, 'Contents', 'MacOS'), { recursive: true });
        writeFileSync(helper, `${name} fixture`);
        chmodSync(helper, 0o555);
      }
      const cefResources = join(root, 'NomiFun.app', 'Contents', 'Resources', 'browser-cef');
      mkdirSync(cefResources, { recursive: true });
      writeFileSync(join(cefResources, 'CREDITS.html'), 'credits');
      writeFileSync(join(cefResources, 'runtime.json'), JSON.stringify({
        cef: '152.0.6',
        chromium: '152.0.7977.83',
        architecture: 'arm64',
        archive: 'cef_binary_fixture_macosarm64_minimal.tar.bz2',
        archive_sha1: 'a'.repeat(40),
      }));
      writeFileSync(packagePath, 'package fixture');
      const lock = createReleaseLock({
        root,
        sourceCommit,
        platform: EXPECTED_TARGET,
        host,
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
          app: null,
          dmg: null,
          hostBinary: missingNomiCore,
          endpoint: null,
          bindingId: null,
          token: null,
          report: null,
          logs: [],
          runStartup: false,
          runLifecycle: false,
          requireNotarization: false,
        },
        {
          platform: 'darwin',
          arch: 'arm64',
          command,
          validatePathShape: (path) => ({ status: 'pass', path, mode: '100555' }),
          inspectDmg: () => ({
            status: 'pass',
            applications_link: 'pass',
            app_matches_staged_source: true,
            cef: { status: 'pass' },
          }),
        },
      );

      expect(result.checks).toEqual(
        expect.arrayContaining([
          expect.objectContaining({ id: 'native-host', status: 'pass' }),
          expect.objectContaining({ id: 'release-lock:real-artifacts', status: 'pass' }),
          expect.objectContaining({ id: 'macos-app:architectures', status: 'pass' }),
          expect.objectContaining({ id: 'macos-app:cef-runtime-framework-helpers', status: 'pass' }),
          expect.objectContaining({ id: 'macos-package:hdiutil-verify', status: 'pass' }),
          expect.objectContaining({ id: 'macos-package:mounted-app-cef-identity', status: 'pass' }),
          expect.objectContaining({ id: 'macos-package:codesign', status: 'not_required' }),
          expect.objectContaining({ id: 'macos-package:notarization-ticket', status: 'not_required' }),
          expect.objectContaining({ id: 'canonical-capability-inventory', status: 'pass' }),
          expect.objectContaining({ id: 'startup:absent-root', status: 'not_required' }),
          expect.objectContaining({
            id: 'startup:precreated-empty-root',
            status: 'not_required',
          }),
          expect.objectContaining({
            id: 'lifecycle:open-ready-turn-observe-cancel-dispose',
            status: 'not_required',
          }),
        ]),
      );
      expect(result.failures.some((entry) => entry.id.startsWith('sidecar:'))).toBe(false);
      expect(result.checks.some((entry) => entry.id.includes('sidecar'))).toBe(false);
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
