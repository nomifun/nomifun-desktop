import { describe, expect, test } from 'bun:test';

import {
  formatWindowsLinkEnvironmentError,
  hasWindowsLinkEnvironmentShape,
  loadWindowsToolchainEnvironment,
  parseCommandEnvironment,
  validateWindowsLinkEnvironment,
} from './run-dev.mjs';

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
