#!/usr/bin/env node
// Exercise the complete production Tauri NSIS template with uniquely named,
// test-owned native executables. No production registration, serial hardware,
// microphone, browser profile, credentials or application data is used.
import assert from 'node:assert/strict';
import { spawn, spawnSync } from 'node:child_process';
import { createHash, randomUUID } from 'node:crypto';
import { appendFileSync, copyFileSync, existsSync, mkdirSync, mkdtempSync, readFileSync, readdirSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { dirname, isAbsolute, join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { buildLegacy } from './keyboard-upgrade-native-smoke.mjs';

const repository = resolve(dirname(fileURLToPath(import.meta.url)), '../..');
const digest = path => createHash('sha256').update(readFileSync(path)).digest('hex');
const ps = text => "'" + text.replaceAll("'", "''") + "'";
const pause = ms => new Promise(done => setTimeout(done, ms));
const children = [];

function execute(program, args, environment = process.env, extra = {}) {
  const child = spawn(program, args, { cwd: repository, env: environment, windowsHide: true, stdio: ['ignore', 'pipe', 'pipe'], ...extra });
  let output = '';
  for (const stream of [child.stdout, child.stderr]) stream.on('data', bytes => { if (output.length < 16384) output += bytes.toString(); });
  const result = new Promise(done => {
    child.once('error', error => done({ code: null, error: error.message, output }));
    child.once('exit', code => done({ code, output }));
  });
  const handle = { child, result }; children.push(handle); return handle;
}

async function finish(handle, timeout = 60000) {
  let timer;
  try { return await Promise.race([handle.result, new Promise((_, reject) => { timer = setTimeout(() => reject(new Error(`Process ${handle.child.pid} timed out`)), timeout); })]); }
  finally { clearTimeout(timer); }
}

async function until(predicate, message) {
  const deadline = Date.now() + 15000;
  while (!predicate()) { if (Date.now() > deadline) throw new Error(message); await pause(35); }
}

function readPid(path) { return existsSync(path) ? JSON.parse(readFileSync(path, 'utf8')).pid : null; }

function registry(script) {
  const result = spawnSync('powershell.exe', ['-NoProfile', '-NonInteractive', '-Command', script], { encoding: 'utf8', windowsHide: true });
  assert.equal(result.status, 0, result.stderr ?? result.error?.message);
  return result.stdout.trim();
}

function actualLoginFingerprint() {
  return registry("$ErrorActionPreference='Stop'; $key='HKCU:\\Software\\Microsoft\\Windows\\CurrentVersion\\Run'; $value=(Get-ItemProperty -LiteralPath $key -Name NomiFunKeyboard -ErrorAction SilentlyContinue).NomiFunKeyboard; if($null -eq $value){'absent'}else{$sha=[Security.Cryptography.SHA256]::Create(); [BitConverter]::ToString($sha.ComputeHash([Text.Encoding]::UTF8.GetBytes([string]$value)))}");
}

function fixtureRegistrationAbsent(identity) {
  const keys = [`HKCU:\\Software\\Microsoft\\Windows\\CurrentVersion\\Uninstall\\${identity.product}`, `HKCU:\\Software\\${identity.publisher}\\${identity.product}`, `HKCU:\\Software\\Classes\\${identity.scheme}`];
  return registry(`$keys=@(${keys.map(ps).join(',')}); $remaining=@($keys | Where-Object {Test-Path -LiteralPath $_}); if($remaining.Count -eq 0){'absent'}else{'present'}`) === 'absent';
}

function seedLegacyRegistration(identity, install) {
  const uninstall = `HKCU:\\Software\\Microsoft\\Windows\\CurrentVersion\\Uninstall\\${identity.product}`;
  const product = `HKCU:\\Software\\${identity.publisher}\\${identity.product}`;
  registry(`$ErrorActionPreference='Stop'; foreach($key in @(${ps(uninstall)},${ps(product)})){New-Item -Path $key -Force|Out-Null; Set-Item -LiteralPath $key -Value ${ps(install)}}; New-ItemProperty -LiteralPath ${ps(uninstall)} -Name DisplayVersion -PropertyType String -Value '0.8.14' -Force|Out-Null; New-ItemProperty -LiteralPath ${ps(uninstall)} -Name UninstallString -PropertyType String -Value ${ps(join(install, 'uninstall.exe'))} -Force|Out-Null`);
}

function registeredVersion(identity) {
  return registry(`$key=${ps(`HKCU:\\Software\\Microsoft\\Windows\\CurrentVersion\\Uninstall\\${identity.product}`)}; (Get-ItemProperty -LiteralPath $key -Name DisplayVersion -ErrorAction Stop).DisplayVersion`);
}

function cleanupFailedFixture(identity, installed) {
  assert(/^nomifun-keyboard-upgrade-[0-9a-f]{32}$/.test(identity.main));
  const keys = [`HKCU:\\Software\\Microsoft\\Windows\\CurrentVersion\\Uninstall\\${identity.product}`,
    `HKCU:\\Software\\${identity.publisher}`, `HKCU:\\Software\\Classes\\${identity.scheme}`];
  // Only this UUID installation's image and registration are eligible. Preserve
  // files and the failed report for diagnosis; no other NomiFun is inspected.
  registry(`$ErrorActionPreference='Stop'; $expected=${ps(installed)}; Get-CimInstance Win32_Process | Where-Object {[String]::Equals($_.ExecutablePath,$expected,[StringComparison]::OrdinalIgnoreCase)} | ForEach-Object {Stop-Process -Id $_.ProcessId -Force}; foreach($key in @(${keys.map(ps).join(',')})){if(Test-Path -LiteralPath $key){Remove-Item -LiteralPath $key -Recurse -Force}}`);
}

function environment(directory) {
  const result = { ...process.env, LOCALAPPDATA: join(directory, 'local'), APPDATA: join(directory, 'roaming'),
    HOME: join(directory, 'home'), USERPROFILE: join(directory, 'home'), NOMIFUN_DATA_DIR: join(directory, 'data'),
    NOMIFUN_LEGACY_FIXTURE_LOG: join(directory, 'legacy.log'), NOMIFUN_LEGACY_FIXTURE_STOP: join(directory, 'legacy-stop') };
  for (const name of Object.keys(result)) if (/(?:API_KEY|ACCESS_TOKEN|AUTH_TOKEN|SECRET|PASSWORD|CREDENTIAL)$/i.test(name)) delete result[name];
  for (const name of ['LOCALAPPDATA', 'APPDATA', 'HOME', 'NOMIFUN_DATA_DIR']) mkdirSync(result[name], { recursive: true });
  return result;
}

async function bundle(directory, options, identity) {
  const target = join(directory, 'target'); const rawDirectory = join(target, 'debug'); mkdirSync(rawDirectory, { recursive: true });
  const raw = join(rawDirectory, `${identity.main}.exe`);
  copyFileSync(options.fixture, raw); appendFileSync(raw, '\nnomifun-full-package-new-release\n');
  copyFileSync(options.helper, join(rawDirectory, 'nomifun-keyboard-maintenance.exe'));
  const config = join(directory, 'fixture.json');
  writeFileSync(config, JSON.stringify({ productName: identity.product, identifier: identity.identifier, mainBinaryName: identity.main,
    build: { beforeBundleCommand: null }, bundle: { publisher: identity.publisher, externalBin: [], resources: [], createUpdaterArtifacts: false,
      windows: { webviewInstallMode: { type: 'skip', silent: null } } },
    plugins: { 'deep-link': { desktop: { schemes: [identity.scheme] } } } }, null, 2));
  const command = execute(process.execPath, [join(repository, 'node_modules/@tauri-apps/cli/tauri.js'), 'bundle', '--debug', '--bundles', 'nsis', '--config', config, '--no-sign', '--ci'],
    { ...process.env, CARGO_TARGET_DIR: target, CARGO_BUILD_BUILD_DIR: join(directory, 'build') });
  const result = await finish(command);
  writeFileSync(join(directory, 'bundle.log'), result.output);
  assert.equal(result.code, 0, `Fixture bundle failed: ${result.output}`);
  const nsis = join(rawDirectory, 'bundle/nsis');
  const installers = readdirSync(nsis).filter(name => name.endsWith('-setup.exe'));
  assert.equal(installers.length, 1, 'Exactly one test-owned installer is required');
  return { raw, installer: join(nsis, installers[0]), rendered: join(rawDirectory, 'nsis/x64/installer.nsi') };
}

export async function runKeyboardUpgradePackageSmoke(options) {
  assert.equal(process.platform, 'win32');
  for (const name of ['helper', 'fixture']) assert(options[name] && isAbsolute(options[name]) && existsSync(options[name]), `Pass absolute --${name}`);
  const root = mkdtempSync(join(tmpdir(), 'nomifun-keyboard-package-upgrade-'));
  const reportPath = options.report ?? join(root, 'report.json');
  const report = { schema: 1, root, helperSha256: digest(options.helper), fixtureSha256: digest(options.fixture), productionTemplate: true,
    hookSha256: digest(join(repository, 'apps/desktop/nsis/keyboard-hooks.nsh')),
    templateSha256: digest(join(repository, 'apps/desktop/nsis/installer.nsi')),
    isolation: { uniqueProductAndRegistry: true, privateInstallAndRuntime: true, userRegistrationUntouched: true, serialEnumeration: false, microphoneCapture: false },
    cases: [], unexecuted: ['released old Tauri binary', 'physical K1/K4 and microphone'], ok: false };
  const loginBefore = actualLoginFingerprint();
  const legacy = buildLegacy(root);
  const owned = [];
  try {
    for (const mode of ['closed', 'running', 'failure']) {
      const running = mode === 'running'; const failure = mode === 'failure';
      const name = failure ? 'full-template-file-failure-preserves-old-metadata' : running ? 'running-legacy-app-update' : 'closed-legacy-manual-upgrade';
      const directory = join(root, name); mkdirSync(directory);
      const nonce = randomUUID().replaceAll('-', '');
      const identity = { product: `NomiFun Keyboard Upgrade ${nonce}`, publisher: `NomiFun Keyboard Fixture ${nonce}`,
        identifier: `com.nomifun.keyboardupgrade.${nonce}`, main: `nomifun-keyboard-upgrade-${nonce}`, scheme: `nomifun-keyboard-upgrade-${nonce}` };
      const result = { name, identity, ok: false };
      report.cases.push(result);
      const env = environment(directory); const install = join(directory, 'install'); mkdirSync(install);
      const installed = join(install, `${identity.main}.exe`); copyFileSync(legacy, installed);
      owned.push({ identity, installed });
      const preferences = join(env.NOMIFUN_DATA_DIR, 'keyboard-settings.json');
      writeFileSync(preferences, '{"enabled":false,"launchAtLogin":false,"sentinel":"do-not-change"}\n'); const preferenceDigest = digest(preferences);
      const runtime = join(env.LOCALAPPDATA, 'NomiFun/keyboard-service'); const desktop = join(runtime, 'desktop.json'); const owner = join(runtime, 'owner.json');
      let old;
      try {
        const artifact = await bundle(directory, options, identity);
        result.installer = artifact.installer; result.installerSha256 = digest(artifact.installer); result.rendered = artifact.rendered;
        result.renderedSha256 = digest(artifact.rendered);
        if (running) {
          old = execute(installed, [], env);
          await until(() => existsSync(env.NOMIFUN_LEGACY_FIXTURE_LOG), 'Old test-owned app did not start');
        }
        let locker; let release; let oldUninstaller;
        if (failure) {
          copyFileSync(legacy, join(install, 'uninstall.exe')); oldUninstaller = digest(join(install, 'uninstall.exe'));
          seedLegacyRegistration(identity, install);
          const ready = join(directory, 'uninstaller-locked'); release = join(directory, 'release-lock');
          locker = execute(legacy, ['--lock', join(install, 'uninstall.exe'), ready, release], env);
          await until(() => existsSync(ready), 'Full-template sharing-denied fixture did not acquire its lock');
        }
        const installer = execute(artifact.installer, ['/S', '/NS', ...(running || failure ? ['/UPDATE', '/R'] : []), `/D=${install}`], env, { windowsVerbatimArguments: true });
        const installedResult = await finish(installer);
        result.installExitCode = installedResult.code;
        if (failure) {
          writeFileSync(release, 'release'); await finish(locker);
          assert.equal(digest(installed), digest(legacy), 'Failed full installer must restore the exact old application');
          assert.equal(digest(join(install, 'uninstall.exe')), oldUninstaller, 'Failed full installer must preserve the old uninstaller');
          assert.equal(registeredVersion(identity), '0.8.14', 'Failed file replacement must not publish new version metadata');
          assert.equal(digest(preferences), preferenceDigest);
          await until(() => existsSync(env.NOMIFUN_LEGACY_FIXTURE_LOG), 'Failed updater did not restore old GUI');
          const log = readFileSync(env.NOMIFUN_LEGACY_FIXTURE_LOG, 'utf8');
          assert(!log.includes('--keyboard-stop') && !log.includes('--keyboard-unregister'));
          assert.equal(actualLoginFingerprint(), loginBefore);
          writeFileSync(env.NOMIFUN_LEGACY_FIXTURE_STOP, 'stop-restored-old-fixture'); await pause(100);
          cleanupFailedFixture(identity, installed); assert(fixtureRegistrationAbsent(identity));
          result.ok = true; continue;
        }
        assert.equal(installedResult.code, 0, installedResult.output);
        assert.equal(digest(installed), digest(artifact.raw), 'Installed fixture must equal candidate executable');
        assert.equal(digest(preferences), preferenceDigest);
        if (running) {
          await finish(old);
          await until(() => readPid(desktop), 'Updater /R did not restore new GUI');
          const log = readFileSync(env.NOMIFUN_LEGACY_FIXTURE_LOG, 'utf8');
          assert(!log.includes('--keyboard-stop') && !log.includes('--keyboard-unregister'));
        } else {
          assert(!existsSync(env.NOMIFUN_LEGACY_FIXTURE_LOG), 'Closed old binary must never be launched for maintenance');
          const gui = execute(installed, [], env); await until(() => readPid(desktop) === gui.child.pid, 'New packaged fixture GUI did not start');
        }
        const service = execute(installed, ['--keyboard-service'], env);
        await until(() => readPid(owner) === service.child.pid, 'New packaged keyboard service did not start');
        // Copy the exact generated uninstaller outside INSTDIR, then use the
        // native final unquoted _?= tail. Its process can delete installed files.
        const uninstall = join(directory, 'uninstall-copy.exe'); copyFileSync(join(install, 'uninstall.exe'), uninstall);
        const removed = await finish(execute(uninstall, ['/S', `_?=${install}`], env, { windowsVerbatimArguments: true }));
        result.uninstallExitCode = removed.code; assert.equal(removed.code, 0, removed.output);
        await until(() => !existsSync(installed) && !existsSync(desktop) && !existsSync(owner), 'Generated uninstaller did not stop and remove fixture runtime');
        assert.equal(digest(preferences), preferenceDigest);
        assert(fixtureRegistrationAbsent(identity), 'Unique test-owned registration must be removed');
        assert.equal(actualLoginFingerprint(), loginBefore, 'Actual NomiFun keyboard login registration must remain untouched');
        result.ok = true;
      } catch (error) { result.error = error.stack ?? String(error); throw error; }
      finally { writeFileSync(env.NOMIFUN_LEGACY_FIXTURE_STOP, 'stop-test-owned-legacy'); }
    }
    report.ok = true;
  } catch (error) { report.error = error.stack ?? String(error); }
  finally {
    for (const child of children) if (child.child.exitCode === null && child.child.signalCode === null) child.child.kill();
    await Promise.allSettled(children.map(child => finish(child, 4000)));
    if (!report.ok) {
      await pause(500); // allow installer-parent death to release its temporary lease
      for (const { identity, installed } of owned) {
        try { cleanupFailedFixture(identity, installed); }
        catch (error) { report.cleanupError = String(error); }
      }
    }
    if (report.hookSha256 !== digest(join(repository, 'apps/desktop/nsis/keyboard-hooks.nsh'))
      || report.templateSha256 !== digest(join(repository, 'apps/desktop/nsis/installer.nsi'))) {
      report.ok = false; report.sourceChangedDuringRun = true;
    }
    if (report.helperSha256 !== digest(options.helper) || report.fixtureSha256 !== digest(options.fixture)) {
      report.ok = false; report.binaryChangedDuringRun = true;
    }
    mkdirSync(dirname(reportPath), { recursive: true }); writeFileSync(reportPath, JSON.stringify(report, null, 2) + '\n');
  }
  return { ok: report.ok, report: reportPath, cases: report.cases.map(({ name, ok }) => ({ name, ok })) };
}

if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  const options = {};
  for (let index = 2; index < process.argv.length; index += 2) {
    assert(['--helper', '--fixture', '--report'].includes(process.argv[index]));
    options[process.argv[index].slice(2)] = resolve(process.argv[index + 1]);
  }
  const result = await runKeyboardUpgradePackageSmoke(options); console.log(JSON.stringify(result)); process.exitCode = result.ok ? 0 : 1;
}
