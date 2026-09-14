import { describe, expect, test } from 'bun:test';

import {
  developmentEnvironment,
  formatWindowsLinkEnvironmentError,
  hasWindowsLinkEnvironmentShape,
  loadWindowsToolchainEnvironment,
  parseCommandEnvironment,
  validateWindowsLinkEnvironment,
} from './run-dev.mjs';
import { join } from 'node:path';

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
