import { describe, expect, test } from 'bun:test';

import {
  createMacosDevLifetime,
  developmentEnvironment,
  ensureGeneratedDevelopmentDataDirectory,
  formatWindowsLinkEnvironmentError,
  hasWindowsLinkEnvironmentShape,
  loadWindowsToolchainEnvironment,
  parseCommandEnvironment,
  validateWindowsLinkEnvironment,
} from './run-dev.mjs';
import { join } from 'node:path';
import { connect } from 'node:net';
import { once } from 'node:events';
import { existsSync } from 'node:fs';

describe('Windows development data generation', () => {
  test('uses a stable clean-start root without altering the input environment', () => {
    const input = { LOCALAPPDATA: 'C:\\Users\\developer\\AppData\\Local', NOMI_CHANNEL: 'stable' };
    const first = developmentEnvironment(input, 'win32');
    expect(first.NOMIFUN_DATA_DIR).toBe(join(input.LOCALAPPDATA, 'NomiFun-dev-plugin-v1'));
    expect(first.NOMI_CHANNEL).toBe('dev');
    expect(developmentEnvironment(input, 'win32')).toEqual(first);
    expect(input.NOMIFUN_DATA_DIR).toBeUndefined();
    expect(input.NOMI_CHANNEL).toBe('stable');
  });

  test('honors explicit data roots including Windows case-insensitive env names', () => {
    for (const key of ['NOMIFUN_DATA_DIR', 'nomifun_data_dir']) {
      const input = { [key]: 'D:\\existing-dev-data' };
      expect(developmentEnvironment(input, 'win32')).toEqual({ ...input, NOMI_CHANNEL: 'dev' });
    }
  });

  test('rejects empty explicit roots or unavailable LOCALAPPDATA instead of silently falling back', () => {
    expect(() => developmentEnvironment({ NOMIFUN_DATA_DIR: ' ' }, 'win32')).toThrow('must not be empty');
    expect(() => developmentEnvironment({}, 'win32')).toThrow('LOCALAPPDATA');
    expect(developmentEnvironment({ localappdata: 'C:\\local' }, 'win32').NOMIFUN_DATA_DIR)
      .toBe(join('C:\\local', 'NomiFun-dev-plugin-v1'));
  });

  test('leaves other platforms data selection unchanged', () => {
    expect(developmentEnvironment({ HOME: '/home/dev' }, 'linux'))
      .toEqual({ HOME: '/home/dev', NOMI_CHANNEL: 'dev' });
    expect(developmentEnvironment({ NOMIFUN_DATA_DIR: '/tmp/dev' }, 'darwin'))
      .toEqual({ NOMIFUN_DATA_DIR: '/tmp/dev', NOMI_CHANNEL: 'dev' });
  });

  test('creates the generated Windows root before launching Tauri', () => {
    const source = { LOCALAPPDATA: 'C:\\Users\\developer\\AppData\\Local' };
    const environment = developmentEnvironment(source, 'win32');
    const calls = [];
    expect(ensureGeneratedDevelopmentDataDirectory(
      environment,
      source,
      'win32',
      (...args) => calls.push(args),
    )).toBe(join(source.LOCALAPPDATA, 'NomiFun-dev-plugin-v1'));
    expect(calls).toEqual([[
      join(source.LOCALAPPDATA, 'NomiFun-dev-plugin-v1'),
      { recursive: true },
    ]]);
  });

  test('does not create caller-owned explicit roots or non-Windows roots', () => {
    const calls = [];
    const createDirectory = (...args) => calls.push(args);
    expect(ensureGeneratedDevelopmentDataDirectory(
      { NOMIFUN_DATA_DIR: 'D:\\owned-by-caller' },
      { NOMIFUN_DATA_DIR: 'D:\\owned-by-caller' },
      'win32',
      createDirectory,
    )).toBeNull();
    expect(ensureGeneratedDevelopmentDataDirectory(
      { NOMIFUN_DATA_DIR: '/tmp/dev' },
      {},
      'darwin',
      createDirectory,
    )).toBeNull();
    expect(calls).toEqual([]);
  });
});

const VALID_X64_ENVIRONMENT = {
  WindowsSDKVersion: '10.0.26100.0\\',
  LIB: [
    'C:\\Program Files (x86)\\Microsoft Visual Studio\\2022\\BuildTools\\VC\\Tools\\MSVC\\14.44.35207\\lib\\x64',
    'C:\\Program Files (x86)\\Windows Kits\\10\\Lib\\10.0.26100.0\\ucrt\\x64',
    'C:\\Program Files (x86)\\Windows Kits\\10\\Lib\\10.0.26100.0\\um\\x64',
  ].join(';'),
  LIBPATH: [
    'C:\\Program Files (x86)\\Microsoft Visual Studio\\2022\\BuildTools\\VC\\Tools\\MSVC\\14.44.35207\\lib\\x64',
    'C:\\Program Files (x86)\\Windows Kits\\10\\References\\10.0.26100.0',
  ].join(';'),
  Path: [
    'C:\\Program Files (x86)\\Microsoft Visual Studio\\2022\\BuildTools\\VC\\Tools\\MSVC\\14.44.35207\\bin\\HostX64\\x64',
    'C:\\Program Files (x86)\\Windows Kits\\10\\bin\\10.0.26100.0\\x64',
  ].join(';'),
};

describe('run-dev native environment', () => {
  test('parses cmd environment without leaking drive pseudo variables', () => {
    const parsed = parseCommandEnvironment(
      [
        'Visual Studio environment initialized',
        '=C:=C:\\workspace',
        'Path=C:\\VC\\Tools\\MSVC\\14.44\\bin\\Hostx64\\x64',
      ].join('\r\n'),
      { KEEP: 'yes', PATH: 'stale-path' },
    );

    expect(parsed.KEEP).toBe('yes');
    expect(parsed.Path).toContain('Hostx64');
    expect(parsed.PATH).toBeUndefined();
    expect(parsed['=C:']).toBeUndefined();
  });

  test('accepts the complete vcvars64 x64 shape across PATH, LIB, and LIBPATH', () => {
    const validation = validateWindowsLinkEnvironment(VALID_X64_ENVIRONMENT);

    expect(validation.ok).toBe(true);
    expect(validation.missing).toEqual([]);
    expect(hasWindowsLinkEnvironmentShape(VALID_X64_ENVIRONMENT)).toBe(true);
  });

  test('rejects x86 or incomplete paths instead of treating them as x64', () => {
    const validation = validateWindowsLinkEnvironment({
      WindowsSDKVersion: VALID_X64_ENVIRONMENT.WindowsSDKVersion,
      LIB: [
        'C:\\Program Files (x86)\\Microsoft Visual Studio\\2022\\BuildTools\\VC\\Tools\\MSVC\\14.44.35207\\lib\\x86',
        'C:\\Program Files (x86)\\Windows Kits\\10\\Lib\\10.0.26100.0\\ucrt\\x86',
        'C:\\Program Files (x86)\\Windows Kits\\10\\Lib\\10.0.26100.0\\um\\x86',
      ].join(';'),
      LIBPATH:
        'C:\\Program Files (x86)\\Microsoft Visual Studio\\2022\\BuildTools\\VC\\Tools\\MSVC\\14.44.35207\\lib\\x86',
      PATH: [
        'C:\\Program Files (x86)\\Microsoft Visual Studio\\2022\\BuildTools\\VC\\Tools\\MSVC\\14.44.35207\\bin\\Hostx86\\x86',
        'C:\\Program Files (x86)\\Windows Kits\\10\\bin\\10.0.26100.0\\x86',
      ].join(';'),
    });

    expect(validation.ok).toBe(false);
    expect(validation.missing.map(({ variable }) => variable)).toEqual([
      'PATH',
      'LIB',
      'LIBPATH',
      'PATH',
      'LIB',
      'LIB',
    ]);
    expect(
      formatWindowsLinkEnvironmentError(validation),
    ).toContain('Hostx64\\x64');
  });

  test('requires LIBPATH and the selected Windows SDK version', () => {
    const validation = validateWindowsLinkEnvironment({
      ...VALID_X64_ENVIRONMENT,
      LIBPATH: '',
      WindowsSDKVersion: '10.0.22621.0\\',
    });

    expect(validation.ok).toBe(false);
    expect(
      validation.missing.some(
        ({ variable, expected }) =>
          variable === 'LIBPATH' && expected.includes('MSVC lib\\x64'),
      ),
    ).toBe(true);
    expect(
      validation.missing.some(
        ({ variable, expected }) =>
          variable === 'PATH' &&
          expected.includes('Windows SDK 10.0.22621.0'),
      ),
    ).toBe(true);
    expect(formatWindowsLinkEnvironmentError(validation)).toContain(
      'Selected Windows SDK: 10.0.22621.0',
    );
  });

  test('leaves non-Windows environments unchanged', () => {
    const environment = {
      PATH: 'C:\\toolchain\\x86',
      LIB: 'C:\\toolchain\\lib',
      LIBPATH: 'C:\\toolchain\\libpath',
    };

    expect(loadWindowsToolchainEnvironment(environment, 'darwin')).toEqual(
      environment,
    );
  });
});


describe.skipIf(process.platform !== 'darwin')('macOS development lifetime', () => {
  test('requests graceful exit and waits until the desktop closes its connection', async () => {
    const lifetime = await createMacosDevLifetime();
    const desktop = connect({ path: lifetime.socketPath, allowHalfOpen: true });
    try {
      await once(desktop, 'connect');
      await new Promise((resolve) => setImmediate(resolve));
      const request = once(desktop, 'data');
      desktop.resume();
      let stopped = false;
      const stopping = lifetime.stop().then(() => { stopped = true; });
      const [requestBytes] = await request;
      expect(requestBytes.toString()).toBe('q');
      expect(stopped).toBe(false);
      expect(lifetime.stop()).toBe(lifetime.stop());
      // Simulate the OS closing the connection after verified backend cleanup.
      desktop.destroy();
      await stopping;
      expect(stopped).toBe(true);
      expect(existsSync(lifetime.socketPath)).toBe(false);
    } finally {
      desktop.destroy();
      await lifetime.stop();
    }
  });

  test('stops during startup and keeps concurrent development runs isolated', async () => {
    const first = await createMacosDevLifetime();
    const second = await createMacosDevLifetime();
    try {
      expect(first.socketPath).not.toBe(second.socketPath);
      await first.stop();
      expect(existsSync(first.socketPath)).toBe(false);
      expect(existsSync(second.socketPath)).toBe(true);
      const desktop = connect(second.socketPath);
      await once(desktop, 'connect');
      const closed = once(desktop, 'close');
      desktop.destroy();
      await closed;
    } finally {
      await first.stop();
      await second.stop();
    }
  });
});
