import { afterEach, describe, expect, test } from 'bun:test';
import { chmodSync, existsSync, mkdirSync, mkdtempSync, readFileSync, readdirSync, rmSync, writeFileSync } from 'node:fs';
import { spawnSync } from 'node:child_process';
import { tmpdir } from 'node:os';
import { basename, join, resolve, sep } from 'node:path';
import { fileURLToPath } from 'node:url';
import { maintenanceBuildArgs } from './build-app-maintenance.mjs';

const root = fileURLToPath(new URL('..', import.meta.url));
const sourceDirectory = join(root, 'apps/desktop/keyboard-packaging');
const directories = [];
afterEach(() => { for (const directory of directories.splice(0)) {
  const absolute = resolve(directory);
  if (!absolute.startsWith(resolve(tmpdir()) + sep) || !basename(absolute).startsWith('nomifun-keyboard-package-')) throw new Error('Invalid fixture cleanup path');
  rmSync(absolute, { recursive: true, force: true });
} });

function fixture({ owner = false, desktop = false, fallback = false, fallbackDesktop = false, registration = false, failure = false, missingBinary = false } = {}) {
  const directory = mkdtempSync(join(tmpdir(), 'nomifun-keyboard-package-')); directories.push(directory);
  const home = join(directory, 'home'); const runtime = join(directory, 'runtime', '1000');
  const binary = join(directory, 'nomifun-desktop'); const bin = join(directory, 'bin');
  mkdirSync(bin); mkdirSync(home);
  if (!missingBinary) { writeFileSync(binary, '#!/bin/sh\nexit 0\n'); chmodSync(binary, 0o755); }
  const assets = {
    getent: '#!/bin/sh\nprintf "%s\\n" "fixture:x:1000:1000:Fixture:$NOMI_TEST_HOME:/bin/bash"\n',
    runuser: '#!/bin/sh\nprintf "%s\\n" "$*" >> "$NOMI_TEST_LOG"\n[ "$NOMI_TEST_FAIL" != 1 ]\n',
    timeout: '#!/bin/sh\nshift\nexec "$@"\n',
  };
  for (const [name, content] of Object.entries(assets)) { const path = join(bin, name); writeFileSync(path, content); chmodSync(path, 0o755); }
  if (owner) { mkdirSync(join(runtime, 'NomiFun/keyboard-service'), { recursive: true }); writeFileSync(join(runtime, 'NomiFun/keyboard-service/owner.json'), '{}'); }
  if (desktop) { mkdirSync(join(runtime, 'NomiFun/keyboard-service'), { recursive: true }); writeFileSync(join(runtime, 'NomiFun/keyboard-service/desktop.json'), '{}'); }
  if (fallback) { const path = join(home, '.local/state/NomiFun/keyboard-service'); mkdirSync(path, { recursive: true }); writeFileSync(join(path, 'owner.json'), '{}'); }
  if (fallbackDesktop) { const path = join(home, '.local/state/NomiFun/keyboard-service'); mkdirSync(path, { recursive: true }); writeFileSync(join(path, 'desktop.json'), '{}'); }
  if (registration) { mkdirSync(join(home, '.config/autostart'), { recursive: true }); writeFileSync(join(home, '.config/autostart/nomifun-keyboard.desktop'), '[Desktop Entry]'); }
  const script = join(directory, 'package-lifecycle.sh');
  writeFileSync(script, readFileSync(join(sourceDirectory, 'package-lifecycle.sh'), 'utf8')
    .replace('binary=/usr/bin/nomifun-desktop', `binary=${binary}`)
    .replace('runtime=/run/user/$uid', `runtime=${directory}/runtime/$uid`));
  const log = join(directory, 'calls');
  return {
    run: (mode) => spawnSync('/bin/sh', [script, mode], { encoding: 'utf8', env: { ...process.env,
      PATH: `${bin}:${process.env.PATH}`, NOMI_TEST_HOME: home, NOMI_TEST_LOG: log, NOMI_TEST_FAIL: failure ? '1' : '0' } }),
    calls: () => existsSync(log) ? readFileSync(log, 'utf8') : '', home, runtime,
  };
}

// Git's POSIX shell can exercise the script's command routing on Windows too.
// These are temporary command fixtures, not a macOS/launchctl native witness.
const packageShell = process.platform === 'win32'
  ? join(process.env.ProgramFiles ?? 'C:/Program Files', 'Git/usr/bin/sh.exe') : '/bin/sh';
const packageShellAvailable = existsSync(packageShell);
const shellPath = path => path.replaceAll('\\', '/');

function macosUninstallFixture(mode = 'success', missingBinary = false) {
  const directory = mkdtempSync(join(tmpdir(), 'nomifun-keyboard-package-macos-')); directories.push(directory);
  const contents = join(directory, 'NomiFun Voice Fixture.app/Contents');
  const packaging = join(contents, 'Resources/keyboard-packaging');
  const executable = join(contents, 'MacOS/nomifun-desktop');
  mkdirSync(packaging, { recursive: true }); mkdirSync(join(contents, 'MacOS'), { recursive: true });
  const script = join(packaging, 'macos-uninstall.command');
  writeFileSync(script, readFileSync(join(sourceDirectory, 'macos-uninstall.command'), 'utf8'));
  if (!missingBinary) {
    writeFileSync(executable, [
      '#!/bin/sh',
      'printf "%s\\n" "$*" >> "$NOMI_MAC_UNINSTALL_CALLS"',
      '[ "$#" -eq 1 ] && [ "$1" = "--app-maintenance-unregister" ] || exit 92',
      'case "$NOMI_MAC_UNINSTALL_MODE" in',
      '  advisory) printf "%s\\n" "Retired keyboard cleanup warning: fixture legacy item could not be inspected" >&2 ;;',
      '  capture-failure) printf "%s\\n" "Current native capture did not stop" >&2; exit 73 ;;',
      'esac',
      'exit 0',
    ].join('\n'));
    chmodSync(executable, 0o755);
  }
  const calls = join(directory, 'calls');
  const settings = join(directory, 'user-settings.json');
  writeFileSync(settings, '{"sentinel":"preserve user data"}\n');
  return {
    run: () => spawnSync(packageShell, [shellPath(script)], { encoding: 'utf8', windowsHide: true, timeout: 5_000,
      env: { ...process.env, PATH: process.platform === 'win32' ? '/usr/bin:/bin' : process.env.PATH,
        NOMI_MAC_UNINSTALL_CALLS: shellPath(calls), NOMI_MAC_UNINSTALL_MODE: mode } }),
    calls: () => existsSync(calls) ? readFileSync(calls, 'utf8').trim().split(/\r?\n/) : [],
    settings: () => readFileSync(settings, 'utf8'),
  };
}

describe('macOS uninstall command routing with isolated command fixtures', () => {
  test.skipIf(!packageShellAvailable)('the packaged command parses in a POSIX shell on this host', () => {
    const result = spawnSync(packageShell, ['-n', shellPath(join(sourceDirectory, 'macos-uninstall.command'))], { encoding: 'utf8', windowsHide: true });
    expect({ status: result.status, stderr: result.stderr }).toEqual({ status: 0, stderr: '' });
  });

  test.skipIf(!packageShellAvailable)('a bundle path containing spaces invokes modern stop/unregister exactly once', () => {
    const item = macosUninstallFixture();
    const result = item.run();
    expect(result.status, result.stderr).toBe(0);
    expect(item.calls()).toEqual(['--app-maintenance-unregister']);
    expect(result.stdout).toContain('The current NomiFun application was stopped.');
    expect(result.stdout).not.toContain('login item was removed');
    expect(item.settings()).toBe('{"sentinel":"preserve user data"}\n');
  });

  test.skipIf(!packageShellAvailable)('a warning from a previous command implementation does not block a confirmed stop', () => {
    const item = macosUninstallFixture('advisory');
    const result = item.run();
    expect(result.status, result.stderr).toBe(0);
    expect(result.stderr).toContain('Retired keyboard cleanup warning:');
    expect(result.stdout).toContain('You may now move NomiFun.app to Trash.');
    expect(item.calls()).toEqual(['--app-maintenance-unregister']);
    expect(item.settings()).toBe('{"sentinel":"preserve user data"}\n');
  });

  test('the macOS CLI stop implementation has no historical keyboard cleanup dependency', () => {
    const source = readFileSync(join(root, 'apps/desktop/src/app_maintenance/platform.rs'), 'utf8');
    const start = source.lastIndexOf('pub fn stop_application(');
    expect(start).toBeGreaterThan(0);
    const modern = source.slice(start, source.indexOf('\n}', start) + 2);
    expect(modern).toContain('prepare_application_with_intent_checked_in');
    expect(modern).toContain('desktop_control::validate_runtime_ownership');
    expect(modern).not.toMatch(/legacy_cleanup|launchctl|owner\.json|owner\.lock|read_legacy_registration/);
  });

  test.skipIf(!packageShellAvailable)('a real current-capture stop failure still prevents a successful uninstall message', () => {
    const item = macosUninstallFixture('capture-failure');
    const result = item.run();
    expect(result.status).not.toBe(0);
    expect(result.stderr).toContain('Current native capture did not stop');
    expect(result.stderr).toContain('could not confirm that the application stopped');
    expect(result.stdout).not.toContain('You may now move');
    expect(item.calls()).toEqual(['--app-maintenance-unregister']);
  });

  test.skipIf(!packageShellAvailable)('a missing application executable is never replaced with an unrelated command', () => {
    const item = macosUninstallFixture('success', true);
    const result = item.run();
    expect(result.status).not.toBe(0);
    expect(result.stderr).toContain('Installed NomiFun executable is missing.');
    expect(item.calls()).toEqual([]);
    expect(result.stdout).not.toContain('You may now move');
  });
});

describe('native keyboard packaging lifecycle', () => {
  const makensis = process.platform === 'win32' ? join(process.env.LOCALAPPDATA, 'tauri/NSIS/makensis.exe') : '';
  test.skipIf(!makensis || !existsSync(makensis))('native NSIS compiler accepts both install and uninstall hooks', () => {
    const directory = mkdtempSync(join(tmpdir(), 'nomifun-keyboard-package-')); directories.push(directory);
    const source = join(directory, 'fixture.nsi');
    const helper = join(directory, 'maintenance.exe');
    writeFileSync(helper, 'MZ'); // compiler fixture only; native lifecycle runner executes the real helper
    writeFileSync(source, [
      'Unicode true', 'SetCompress off', 'RequestExecutionLevel user', '!include "LogicLib.nsh"', '!include "FileFunc.nsh"',
      '!define MAINBINARYNAME "nomifun-desktop"', 'Var UpdateMode',
      `!define NOMIFUN_MAINTENANCE_BINARY "${helper}"`,
      `!include "${join(root, 'apps/desktop/nsis/app-maintenance-hooks.nsh')}"`,
      '!insertmacro NomiFunMaintenanceFunctions "" "no"', '!insertmacro NomiFunMaintenanceFunctions "un." "yes"',
      `OutFile "${join(directory, 'fixture.exe')}"`, 'Section', '!insertmacro NSIS_HOOK_PREINSTALL',
      'WriteUninstaller "$TEMP\\nomifun-keyboard-fixture-uninstall.exe"', 'SectionEnd', 'Section Uninstall',
      '!insertmacro NSIS_HOOK_PREUNINSTALL', 'SectionEnd',
    ].join('\n'));
    const result = spawnSync(makensis, ['/V2', source], { encoding: 'utf8' });
    expect({ status: result.status, stderr: result.stderr }).toEqual({ status: 0, stderr: '' });
    expect(existsSync(join(directory, 'fixture.exe'))).toBe(true);
  });

  test('all configured package hooks and bundled uninstall resources exist', () => {
    const config = JSON.parse(readFileSync(join(root, 'apps/desktop/tauri.conf.json'), 'utf8'));
    expect(config.bundle.windows.nsis.installerHooks).toBe('nsis/app-maintenance-hooks.nsh');
    expect(config.bundle.resources['keyboard-packaging/']).toBe('keyboard-packaging/');
    for (const format of ['deb', 'rpm']) {
      for (const field of ['preInstallScript', 'preRemoveScript', 'postInstallScript', 'postRemoveScript']) {
        expect(existsSync(join(root, 'apps/desktop', config.bundle.linux[format][field]))).toBe(true);
      }
      for (const path of Object.values(config.bundle.linux[format].files)) expect(existsSync(join(root, 'apps/desktop', path))).toBe(true);
    }
    const hooks = readFileSync(join(root, 'apps/desktop/nsis/app-maintenance-hooks.nsh'), 'utf8');
    expect(hooks).toContain('--parent-pid');
    expect(hooks).toContain('NOMIFUN_MAINTENANCE_BINARY');
    expect(hooks).toContain('NomiFunMaintenanceAbort');
    expect(hooks).toContain('NomiFunMaintenanceCommit');
    expect(hooks).not.toContain('nsExec::ExecToStack');
    expect(hooks).not.toContain('--keyboard-stop');
    expect(hooks).not.toContain('--resume-service');
    expect(hooks).not.toContain('--keyboard-service');
    for (const [name, unregister] of [['package-lifecycle.sh', '--keyboard-unregister'],
      ['appimage-uninstall.sh', '--keyboard-unregister'], ['macos-uninstall.command', '--app-maintenance-unregister']]) {
      const source = readFileSync(join(sourceDirectory, name), 'utf8');
      expect(source).toContain(unregister);
      expect(source).not.toContain('--keyboard-service');
    }
    expect(readFileSync(join(sourceDirectory, '70-nomifun-keyboard.rules'), 'utf8')).not.toContain('0666');
  });

  test('maintenance builds beside the same implicit or explicit desktop target', () => {
    const implicit = maintenanceBuildArgs({ profile: 'debug' });
    expect(implicit).not.toContain('--target');
    expect(implicit).toContain('dev');
    const explicit = maintenanceBuildArgs({ target: 'x86_64-pc-windows-msvc' });
    expect(explicit.slice(explicit.indexOf('--target'), explicit.indexOf('--target') + 2)).toEqual(['--target', 'x86_64-pc-windows-msvc']);
    expect(explicit).toContain('release');
    expect(() => maintenanceBuildArgs({ target: '../other' })).toThrow();
  });

  test('cross-version stop aliases stay in headless application maintenance and cannot start a daemon', () => {
    const entry = readFileSync(join(root, 'apps/desktop/src/app_maintenance.rs'), 'utf8');
    for (const [current, alias, unregister] of [['--app-maintenance-stop', '--keyboard-stop', false],
      ['--app-maintenance-unregister', '--keyboard-unregister', true]]) {
      const pattern = new RegExp(`(?:"${current}"\\s*\\|\\s*"${alias}"|"${alias}"\\s*\\|\\s*"${current}")\\s*=>\\s*${unregister}\\b`);
      expect(entry).toMatch(pattern);
    }
    expect(entry).toMatch(/"--keyboard-service"\s*=>\s*return Some\(Err\(/);
    expect(entry).toContain("command(&args)?.and_then(platform::stop_application)");
  });

  test.skipIf(!packageShellAvailable)('package scripts parse in a real POSIX shell', () => {
    for (const name of readdirSync(sourceDirectory).filter((name) => /\.(sh|command)$/.test(name))) {
      const result = spawnSync(packageShell, ['-n', shellPath(join(sourceDirectory, name))], { encoding: 'utf8', windowsHide: true });
      expect({ name, status: result.status, error: result.stderr }).toEqual({ name, status: 0, error: '' });
    }
  });

  test.skipIf(process.platform === 'win32')('upgrade stops the application and retires the authenticated legacy owner in its user account', () => {
    const item = fixture({ owner: true, registration: true });
    expect(item.run('stop').status).toBe(0);
    expect(item.calls()).toContain('-u fixture -- env HOME=');
    expect(item.calls()).toContain(`XDG_RUNTIME_DIR=${item.runtime}`);
    expect(item.calls()).toContain('--keyboard-stop');
    expect(item.calls()).not.toContain('--keyboard-unregister');
    expect(item.calls()).not.toContain('--app-maintenance-');
  });

  test.skipIf(process.platform === 'win32')('uninstall cleans the registration for a logged out user without claiming a graphical session', () => {
    const item = fixture({ registration: true });
    expect(item.run('unregister').status).toBe(0);
    expect(item.calls()).toContain('env -u XDG_RUNTIME_DIR');
    expect(item.calls()).toContain('--keyboard-unregister');
    expect(item.calls()).not.toContain('--app-maintenance-');
  });

  test.skipIf(process.platform === 'win32')('an unconfirmed application or legacy owner stop prevents package replacement', () => {
    const item = fixture({ owner: true, failure: true });
    expect(item.run('stop').status).not.toBe(0);
  });

  test.skipIf(process.platform === 'win32')('stops a fallback owner even when the graphical runtime directory also exists', () => {
    const item = fixture({ owner: true, fallback: true });
    expect(item.run('stop').status).toBe(0);
    expect(item.calls().trim().split('\n')).toHaveLength(2);
    expect(item.calls()).toContain(`XDG_RUNTIME_DIR=${item.runtime}`);
    expect(item.calls()).toContain('env -u XDG_RUNTIME_DIR');
  });

  test.skipIf(process.platform === 'win32')('always invokes the installed CLI without a published owner descriptor', () => {
    const item = fixture();
    expect(item.run('stop').status).toBe(0);
    expect(item.calls()).toContain('--keyboard-stop');
  });

  test.skipIf(process.platform === 'win32')('stops a GUI-only runtime and fallback without a serial owner', () => {
    const item = fixture({ desktop: true, fallbackDesktop: true });
    expect(item.run('stop').status).toBe(0);
    expect(item.calls().trim().split('\n')).toHaveLength(2);
    expect(item.calls()).toContain(`XDG_RUNTIME_DIR=${item.runtime}`);
    expect(item.calls()).toContain('env -u XDG_RUNTIME_DIR');
  });

  test.skipIf(process.platform === 'win32')('fails closed if executable is missing while any endpoint descriptor remains', () => {
    for (const descriptor of ['owner', 'desktop', 'fallback', 'fallbackDesktop']) {
      const item = fixture({ missingBinary: true, [descriptor]: true });
      expect(item.run('stop').status).not.toBe(0);
      expect(item.calls()).toBe('');
    }
    expect(fixture({ missingBinary: true }).run('stop').status).toBe(0);
  });
});
