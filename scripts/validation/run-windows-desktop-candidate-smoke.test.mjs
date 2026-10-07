import { describe, expect, test } from 'bun:test';
import { resolve } from 'node:path';

import {
  CHECK_IDS,
  MAIN_BINARY_NAME,
  TARGET_ID,
  assertSelfTest,
  buildNsisInstallArgs,
  createInitialResult,
  environmentWithoutSecrets,
  evaluateHealthResponse,
  evaluateRegistryProbe,
  evaluateRegularExeArtifact,
  evaluateSourceCheckpoint,
  findNomiFunCdpTarget,
  isPathWithin,
  parseArgs,
  parsePeMachine,
  validatePortAnnouncement,
  validateWorkRootPath,
} from './run-windows-desktop-candidate-smoke.mjs';

const SOURCE_COMMIT = 'a'.repeat(40);

function amd64PeFixture(machine = 0x8664) {
  const fixture = Buffer.alloc(128);
  fixture.write('MZ', 0, 'ascii');
  fixture.writeUInt32LE(64, 0x3c);
  fixture.write('PE\0\0', 64, 'binary');
  fixture.writeUInt16LE(machine, 68);
  return fixture;
}

describe('Windows Desktop candidate installation smoke', () => {
  test('self-test exercises the harness without installing or launching anything', () => {
    expect(assertSelfTest()).toEqual(
      expect.objectContaining({
        schema_version: '1.0.0',
        target: TARGET_ID,
        status: 'pass',
      }),
    );
  });

  test('accepts only the fixed operational CLI contract', () => {
    expect(
      parseArgs([
        '--installer',
        'target/release/bundle/nsis/NomiFun.exe',
        '--source-commit',
        SOURCE_COMMIT.toUpperCase(),
        '--work-root',
        'build.noindex/candidate',
      ]),
    ).toEqual({
      selfTest: false,
      installer: 'target/release/bundle/nsis/NomiFun.exe',
      sourceCommit: SOURCE_COMMIT,
      workRoot: 'build.noindex/candidate',
    });

    expect(() =>
      parseArgs([
        '--installer',
        'candidate.exe',
        '--source-commit',
        'not-a-commit',
        '--work-root',
        'build.noindex/candidate',
      ]),
    ).toThrow('40 hexadecimal');
    expect(() => parseArgs(['--self-test', '--installer', 'candidate.exe'])).toThrow(
      'cannot be combined',
    );
    expect(() =>
      parseArgs([
        '--installer',
        'one.exe',
        '--installer',
        'two.exe',
        '--source-commit',
        SOURCE_COMMIT,
        '--work-root',
        'build.noindex/candidate',
      ]),
    ).toThrow('duplicate argument');
  });

  test('requires work-root to remain under repository build.noindex', () => {
    expect(validateWorkRootPath('/repo', '/repo/build.noindex/candidate').status).toBe(
      'pass',
    );
    expect(validateWorkRootPath('/repo', '/repo/build.noindex').status).toBe('pass');
    expect(validateWorkRootPath('/repo', '/repo/target/candidate')).toEqual(
      expect.objectContaining({
        status: 'fail',
        reason: 'work_root_must_be_within_repository_build_noindex',
      }),
    );
    expect(isPathWithin('/repo/build.noindex', '/repo/build.noindex-escape')).toBe(false);
  });

  test('places the NSIS /D override last and preserves spaces as one argument', () => {
    const installDirectory = resolve('/repo/build.noindex/candidate run/install');
    const args = buildNsisInstallArgs(installDirectory);
    expect(args).toEqual([
      '/S',
      '/NS',
      `/D=${installDirectory}`,
    ]);
    expect(args.at(-1).startsWith('/D=')).toBe(true);
  });

  test('refuses to overwrite an existing user-level installation', () => {
    expect(
      evaluateRegistryProbe([
        { key: 'uninstall', status: 1 },
        { key: 'manufacturer', status: 1 },
        { key: 'protocol', status: 1 },
      ]),
    ).toEqual({
      status: 'pass',
      existing: [],
      probe_errors: [],
    });
    expect(
      evaluateRegistryProbe([
        { key: 'uninstall', status: 0 },
        { key: 'manufacturer', status: 1 },
        { key: 'protocol', status: 2 },
      ]),
    ).toEqual({
      status: 'fail',
      existing: ['uninstall'],
      probe_errors: ['protocol'],
    });
  });

  test('requires clean HEAD to equal the declared source commit', () => {
    expect(
      evaluateSourceCheckpoint({
        expected: SOURCE_COMMIT,
        head: `${SOURCE_COMMIT}\n`,
        statusOutput: '',
        headStatus: 0,
        statusStatus: 0,
      }),
    ).toEqual(
      expect.objectContaining({
        status: 'pass',
        observed: SOURCE_COMMIT,
        clean: true,
        errors: [],
      }),
    );

    expect(
      evaluateSourceCheckpoint({
        expected: SOURCE_COMMIT,
        head: `${'b'.repeat(40)}\n`,
        statusOutput: ' M scripts/example.mjs',
        headStatus: 0,
        statusStatus: 0,
      }).errors,
    ).toEqual(['head_mismatch', 'dirty_worktree']);
  });

  test('accepts only non-empty real regular exe artifacts inside allowed roots', () => {
    const passing = evaluateRegularExeArtifact({
      resolvedPath: '/repo/build.noindex/candidate/NomiFun.exe',
      realPath: '/repo/build.noindex/candidate/NomiFun.exe',
      allowedRoots: ['/repo'],
      isFile: true,
      isSymbolicLink: false,
      sizeBytes: 1024,
    });
    expect(passing.status).toBe('pass');

    const failing = evaluateRegularExeArtifact({
      resolvedPath: '/outside/NomiFun.exe',
      realPath: '/outside/NomiFun.exe',
      allowedRoots: ['/repo'],
      isFile: false,
      isSymbolicLink: true,
      sizeBytes: 0,
    });
    expect(failing.status).toBe('fail');
    expect(failing.errors).toEqual([
      'not_regular_file',
      'symlink_not_allowed',
      'empty_or_invalid_size',
      'artifact_outside_allowed_roots',
    ]);
  });

  test('validates a loopback port announcement owned by the launched application', () => {
    expect(
      validatePortAnnouncement(
        { host: '127.0.0.1', port: 25808, channel: 'stable', pid: 42 },
        42,
      ),
    ).toEqual({
      status: 'pass',
      errors: [],
      value: {
        host: '127.0.0.1',
        port: 25808,
        channel: 'stable',
        pid: 42,
      },
    });

    expect(
      validatePortAnnouncement(
        { host: '0.0.0.0', port: 70_000, channel: '', pid: 7 },
        42,
      ).errors,
    ).toEqual([
      'host_not_loopback',
      'port_invalid',
      'channel_invalid',
      'pid_mismatch',
    ]);
  });

  test('requires the health payload and WebView2 target to identify NomiFun', () => {
    expect(evaluateHealthResponse(200, '{"status":"ok"}').status).toBe('pass');
    expect(evaluateHealthResponse(503, '{"status":"ok"}').status).toBe('fail');
    expect(evaluateHealthResponse(200, '{"status":"starting"}').status).toBe('fail');

    expect(
      findNomiFunCdpTarget([
        { id: 'other', type: 'page', title: 'Other', url: 'http://tauri.localhost/' },
        {
          id: 'nomifun',
          type: 'page',
          title: 'NomiFun',
          url: 'http://tauri.localhost/index.html',
        },
      ]),
    ).toEqual({
      id: 'nomifun',
      type: 'page',
      title: 'NomiFun',
      url: 'http://tauri.localhost/index.html',
    });
    expect(
      findNomiFunCdpTarget([
        {
          id: 'wrong-origin',
          type: 'page',
          title: 'NomiFun',
          url: 'http://example.test/',
        },
      ]),
    ).toBeNull();
  });

  test('recognizes only an AMD64 PE host binary', () => {
    expect(parsePeMachine(amd64PeFixture())).toEqual({
      status: 'pass',
      reason: null,
      machine: 0x8664,
      machine_hex: '0x8664',
    });
    expect(parsePeMachine(amd64PeFixture(0x014c))).toEqual({
      status: 'fail',
      reason: 'machine_not_amd64',
      machine: 0x014c,
      machine_hex: '0x014c',
    });
    expect(parsePeMachine(Buffer.from('not a PE'))).toEqual({
      status: 'fail',
      reason: 'invalid_dos_header',
      machine: null,
    });
  });

  test('does not forward credential-like environment variables', () => {
    expect(
      environmentWithoutSecrets({
        PATH: 'safe-path',
        NOMIFUN_LIVE_STEPFUN_API_KEY: 'secret',
        ACCESS_TOKEN: 'secret',
        USERPROFILE: 'safe-profile',
      }),
    ).toEqual({
      PATH: 'safe-path',
      USERPROFILE: 'safe-profile',
    });
  });

  test('initial result exposes the required stable evidence sections', () => {
    const result = createInitialResult(SOURCE_COMMIT);
    expect(result).toEqual(
      expect.objectContaining({
        schema_version: '1.0.0',
        source_commit: SOURCE_COMMIT,
        target: TARGET_ID,
        status: 'fail',
        suite: {
          name: 'windows-desktop-candidate-install-smoke',
          checks: CHECK_IDS,
        },
        checks: [],
        logs: [],
        artifacts: expect.any(Object),
        install: expect.objectContaining({
          root: null,
          main_binary_removed: false,
        }),
        data: expect.any(Object),
        backend: expect.any(Object),
        cdp: expect.any(Object),
      }),
    );
    expect(result.artifacts.host).toBeNull();
    expect(result.install.root).toBeNull();
    expect(MAIN_BINARY_NAME).toBe('nomifun-desktop.exe');
  });
});
