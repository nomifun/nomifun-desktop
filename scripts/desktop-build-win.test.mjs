import { spawnSync } from 'node:child_process';
import { copyFileSync, existsSync, mkdirSync, mkdtempSync, readFileSync, realpathSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { basename, dirname, join } from 'node:path';
import { describe, expect, test } from 'bun:test';

const script = new URL('./desktop-build-win.ps1', import.meta.url);
const quote = value => `'${value.replaceAll("'", "''")}'`;

// Run a private copy of the actual PowerShell script with function substitutes
// for every tool it can invoke. No Cargo, rustup, signer or Tauri is executed.
function controlledBuild({ signed = true, failSign = false, failMaintenanceSign = false, passthrough = [] } = {}) {
  const root = mkdtempSync(join(tmpdir(), 'nomifun-win-signing-test-'));
  try {
    mkdirSync(join(root, 'scripts'));
    copyFileSync(script, join(root, 'scripts/desktop-build-win.ps1'));
    const log = join(root, 'commands.jsonl');
    const wrapper = join(root, 'controlled.ps1');
    writeFileSync(wrapper, '\uFEFF' + `
$ErrorActionPreference='Stop'
$global:signTestRoot=${quote(root)}
$global:signTestLog=${quote(log)}
$forwarded=@($args)
function Record-TestCommand($name,$values,$extra=$null) {
  $record=[ordered]@{command=$name;arguments=@($values);extra=$extra}
  [IO.File]::AppendAllText($global:signTestLog,($record|ConvertTo-Json -Depth 10 -Compress)+[Environment]::NewLine)
}
function rustup {
  if (($args -join ' ') -ne 'target list --installed') { throw 'Unexpected rustup call' }
  Record-TestCommand 'rustup' $args
  $global:LASTEXITCODE=0
  'x86_64-pc-windows-msvc'
}
function Get-Command {
  [CmdletBinding()] param([Parameter(Position=0)][string]$Name)
  if ($Name -ne 'signtool.exe') { throw 'Unexpected command lookup' }
  [pscustomobject]@{Source='nomifun-test-signtool'}
}
function nomifun-test-signtool {
  Record-TestCommand 'signtool' $args
  $global:LASTEXITCODE=0
  if ($env:NOMIFUN_TEST_FAIL_SIGN -eq '1' -and $args[0] -eq 'sign') { $global:LASTEXITCODE=7 }
  if ($env:NOMIFUN_TEST_FAIL_MAINTENANCE_SIGN -eq '1' -and $args[0] -eq 'sign' -and $args[-1].EndsWith('nomifun-keyboard-maintenance.exe')) { $global:LASTEXITCODE=9 }
}
function bun {
  if ($args[0].EndsWith('computer/build-worker.mjs') -or $args[0].EndsWith('computer\\build-worker.mjs')) {
    Record-TestCommand 'worker-build' $args
    $directory=Join-Path $global:signTestRoot 'apps/desktop/binaries'
    New-Item -ItemType Directory -Force -Path $directory|Out-Null
    $worker=Join-Path $directory 'nomifun-computer-worker-x86_64-pc-windows-msvc.exe'
    [IO.File]::WriteAllText($worker,'owned fixture bytes, never an executable')
    $global:LASTEXITCODE=0
    $worker
  } elseif ($args[0].EndsWith('build-keyboard-maintenance.mjs')) {
    Record-TestCommand 'maintenance-build' $args
    $profile=$args[-1]
    $directory=Join-Path $global:signTestRoot "target/x86_64-pc-windows-msvc/$profile"
    New-Item -ItemType Directory -Force -Path $directory|Out-Null
    $maintenance=Join-Path $directory 'nomifun-keyboard-maintenance.exe'
    [IO.File]::WriteAllText($maintenance,'owned maintenance fixture bytes, never an executable')
    $global:LASTEXITCODE=0
    $maintenance
  } elseif ($args[0] -eq 'x' -and $args[1] -eq 'tauri' -and $args[2] -eq 'build') {
    $signing=$null
    for ($i=0;$i -lt $args.Count-1;$i++) {
      if ($args[$i] -eq '--config' -and [IO.Path]::GetFileName($args[$i+1]) -like 'nomifun-sign-*.json') {
        $signing=([IO.File]::ReadAllText($args[$i+1])|ConvertFrom-Json).bundle.windows
      }
    }
    Record-TestCommand 'tauri-build' $args $signing
    $directory=Join-Path $global:signTestRoot 'target/x86_64-pc-windows-msvc/release/bundle/nsis'
    New-Item -ItemType Directory -Force -Path $directory|Out-Null
    [IO.File]::WriteAllText((Join-Path $directory 'NomiFun_0.8.14_x64-setup.exe'),'owned package fixture, never executed')
    $global:LASTEXITCODE=0
  } else { throw 'Unexpected bun call; no real tool is allowed' }
}
$tokens=$null;$errors=$null
[Management.Automation.Language.Parser]::ParseFile((Join-Path $global:signTestRoot 'scripts/desktop-build-win.ps1'),[ref]$tokens,[ref]$errors)|Out-Null
if ($errors.Count -ne 0) { throw 'Actual build script did not parse' }
& (Join-Path $global:signTestRoot 'scripts/desktop-build-win.ps1') @forwarded
`, 'utf8');
    const environment = { ...process.env, TEMP: root, TMP: root, NOMIFUN_TEST_FAIL_SIGN: failSign ? '1' : '0',
      NOMIFUN_TEST_FAIL_MAINTENANCE_SIGN: failMaintenanceSign ? '1' : '0' };
    delete environment.WINDOWS_CERTIFICATE_THUMBPRINT;
    delete environment.WINDOWS_TIMESTAMP_URL;
    if (signed) {
      environment.WINDOWS_CERTIFICATE_THUMBPRINT = 'a'.repeat(40);
      environment.WINDOWS_TIMESTAMP_URL = 'https://timestamp.example.invalid/tsa';
    }
    const result = spawnSync('powershell.exe', ['-NoLogo', '-NoProfile', '-NonInteractive', '-ExecutionPolicy', 'Bypass', '-File', wrapper,
      'x64', ...(signed ? ['--signed'] : []), ...(passthrough.length ? ['--', ...passthrough] : [])], {
      cwd: root, env: environment, encoding: 'utf8', windowsHide: true, timeout: 10_000,
    });
    const commands = existsSync(log) ? readFileSync(log, 'utf8').trim().split(/\r?\n/).filter(Boolean).map(line => JSON.parse(line.replace(/^\uFEFF/, ''))) : [];
    return { result, commands, collected: existsSync(join(root, 'dist/desktop/NomiFun_0.8.14_x64-setup.exe')) };
  } finally {
    const actual = realpathSync(root);
    if (dirname(actual) !== realpathSync(tmpdir()) || !basename(actual).startsWith('nomifun-win-signing-test-')) throw new Error('Refuse cleanup outside the exact owned signing-test directory');
    rmSync(actual, { recursive: true, force: true });
  }
}

describe.skipIf(process.platform !== 'win32')('Windows signing build maintenance with controlled PowerShell tools', () => {
  test('passes the same timestamped identity to worker, short-lived maintenance helper and Tauri overlay', () => {
    const { result, commands, collected } = controlledBuild();
    expect(result.error).toBeUndefined();
    expect(result.status).toBe(0);
    expect(commands.map(command => command.command)).toEqual(['rustup', 'worker-build', 'maintenance-build', 'signtool', 'signtool', 'signtool', 'signtool', 'tauri-build']);
    const [sign, verify, maintenanceSign, maintenanceVerify] = commands.filter(command => command.command === 'signtool');
    expect(sign.arguments.slice(0, -1)).toEqual(['sign', '/sha1', 'a'.repeat(40), '/fd', 'SHA256', '/tr', 'https://timestamp.example.invalid/tsa', '/td', 'SHA256']);
    expect(verify.arguments).toEqual(['verify', '/pa', sign.arguments.at(-1)]);
    expect(maintenanceSign.arguments.slice(0, -1)).toEqual(sign.arguments.slice(0, -1));
    expect(maintenanceSign.arguments.at(-1).endsWith('nomifun-keyboard-maintenance.exe')).toBe(true);
    expect(maintenanceVerify.arguments).toEqual(['verify', '/pa', maintenanceSign.arguments.at(-1)]);
    expect(commands.at(-1).extra).toEqual({ certificateThumbprint: 'a'.repeat(40), timestampUrl: 'https://timestamp.example.invalid/tsa', tsp: true });
    expect(collected).toBe(true);
  });

  test('does not call Tauri after a failed worker signature', () => {
    const { result, commands, collected } = controlledBuild({ failSign: true });
    expect(result.status).not.toBe(0);
    expect(commands.map(command => command.command)).toEqual(['rustup', 'worker-build', 'maintenance-build', 'signtool']);
    expect(collected).toBe(false);
  });

  test('does not call Tauri after a failed maintenance helper signature', () => {
    const { result, commands, collected } = controlledBuild({ failMaintenanceSign: true });
    expect(result.status).not.toBe(0);
    expect(commands.map(command => command.command)).toEqual(['rustup', 'worker-build', 'maintenance-build', 'signtool', 'signtool', 'signtool']);
    expect(collected).toBe(false);
  });

  test('keeps unsigned custom passthrough without invoking a signer', () => {
    const { result, commands } = controlledBuild({ signed: false, passthrough: ['--debug', '--config', 'owned-overlay.json'] });
    expect(result.status).toBe(0);
    expect(commands.map(command => command.command)).toEqual(['rustup', 'worker-build', 'maintenance-build', 'tauri-build']);
    expect(commands.find(command => command.command === 'maintenance-build').arguments.slice(-4)).toEqual(['--target', 'x86_64-pc-windows-msvc', '--profile', 'debug']);
    expect(commands.at(-1).arguments.slice(-3)).toEqual(['--debug', '--config', 'owned-overlay.json']);
    expect(commands.at(-1).extra).toBeNull();
  });
});
