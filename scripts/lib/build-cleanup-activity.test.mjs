import { describe, expect, test } from 'bun:test';
import { inspectBuildActivity } from './build-cleanup-activity.mjs';

describe('build cleanup process snapshot', () => {
  test('Windows uses hidden structured process names without command lines or a shell', () => {
    const calls = [];
    const result = inspectBuildActivity({
      platform: 'win32',
      run: (...args) => {
        calls.push(args);
        return { status: 0, stdout: JSON.stringify(['Code', 'CARGO', 'rustc.exe', 'rust-lld', 'rustc.exe']) };
      },
    });
    expect(result).toEqual({ state: 'busy', names: ['cargo', 'rust-lld', 'rustc'] });
    const [command, args, options] = calls[0];
    expect(command).toBe('powershell.exe');
    expect(args).toContain('Hidden');
    expect(args.at(-1)).toContain('ExpandProperty ProcessName');
    expect(args.at(-1)).not.toContain('CommandLine');
    expect(options).toMatchObject({ windowsHide: true, shell: false, timeout: 10_000 });
  });

  test('macOS and Linux read comm with argument arrays and recognize compiler/linker paths', () => {
    for (const platform of ['darwin', 'linux']) {
      const calls = [];
      const result = inspectBuildActivity({
        platform,
        run: (...args) => {
          calls.push(args);
          return { status: 0, stdout: '/usr/bin/launchd\n/opt/rust/bin/rustdoc\n/usr/bin/ld\n' };
        },
      });
      expect(result).toEqual({ state: 'busy', names: ['ld', 'rustdoc'] });
      expect(calls[0].slice(0, 2)).toEqual(['ps', ['-A', '-o', 'comm=']]);
      expect(calls[0][2].shell).toBe(false);
    }
  });

  test('covers Rust tools and common standalone linker drivers', () => {
    for (const name of [
      'cargo', 'rustc', 'rustdoc', 'rust-lld', 'lld-link', 'link.exe',
      'ld', 'ld.bfd', 'ld.gold', 'ld.lld', 'ld64', 'ld64.lld', 'wasm-ld', 'mold',
      'collect2', 'cc', 'c++', 'gcc', 'g++', 'clang', 'clang++', 'clang-cl',
    ]) {
      expect(inspectBuildActivity({
        platform: 'linux', run: () => ({ status: 0, stdout: `${name}\n` }),
      }).state).toBe('busy');
    }
  });

  test('ordinary processes and finalized-cache lock names are not build activity', () => {
    expect(inspectBuildActivity({
      platform: 'win32',
      run: () => ({ status: 0, stdout: JSON.stringify(['bun', 'Code', 'cargo-metadata', 's-id.lock']) }),
    })).toEqual({ state: 'idle' });
    expect(inspectBuildActivity({
      platform: 'win32', run: () => ({ status: 0, stdout: '\uFEFF"Code"' }),
    })).toEqual({ state: 'idle' });
  });

  test('failed, timed out, truncated or malformed enumeration is unknown', () => {
    const invalid = [
      { status: 1, stdout: '[]' },
      { status: null, stdout: '["Code"]' },
      { status: 0, stdout: '["Code"]', error: new Error('failed') },
      { status: 0, stdout: '' },
      { status: 0, stdout: '[]' },
      { status: 0, stdout: 'null' },
      { status: 0, stdout: '["Code",{}]' },
      { status: 0, stdout: '["Code",' },
      { status: 0 },
    ];
    for (const result of invalid) {
      expect(inspectBuildActivity({ platform: 'win32', run: () => result })).toEqual({ state: 'unknown' });
    }
    expect(inspectBuildActivity({ platform: 'linux', run: () => ({ status: 0, stdout: '\n' }) }))
      .toEqual({ state: 'unknown' });
    expect(inspectBuildActivity({ platform: 'linux', run: () => { throw new Error('no ps'); } }))
      .toEqual({ state: 'unknown' });
  });

  test('unsupported platforms fail closed without starting a subprocess', () => {
    let called = false;
    expect(inspectBuildActivity({ platform: 'unknown', run: () => { called = true; } }))
      .toEqual({ state: 'unknown' });
    expect(called).toBe(false);
  });
});
