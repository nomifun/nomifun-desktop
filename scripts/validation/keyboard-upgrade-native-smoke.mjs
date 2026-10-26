#!/usr/bin/env node
// Windows package-maintenance evidence in disposable installation/runtime roots.
// Uses the real helper, locks and authenticated IPC, with a native fixture that
// never enumerates serial ports, opens a microphone or writes HKCU login items.
import assert from 'node:assert/strict';
import { spawn, spawnSync } from 'node:child_process';
import { createHash } from 'node:crypto';
import { appendFileSync, copyFileSync, existsSync, mkdirSync, mkdtempSync, readFileSync, renameSync, unlinkSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { dirname, isAbsolute, join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

const repository = resolve(dirname(fileURLToPath(import.meta.url)), '../..');
const pause = milliseconds => new Promise(done => setTimeout(done, milliseconds));
const digest = path => createHash('sha256').update(readFileSync(path)).digest('hex');
const quotePowerShell = value => "'" + value.replaceAll("'", "''") + "'";

function arguments_(argv) {
  const options = {};
  for (let index = 0; index < argv.length; index += 2) {
    assert(['--helper', '--fixture', '--report', '--makensis'].includes(argv[index]), `Unknown argument ${argv[index]}`);
    assert(argv[index + 1] && isAbsolute(argv[index + 1]), `${argv[index]} requires an absolute path`);
    options[argv[index].slice(2)] = resolve(argv[index + 1]);
  }
  return options;
}

function status(session) {
  return iniStatus(join(session, 'status.ini'));
}

function iniStatus(path) {
  if (!existsSync(path)) return {};
  const bytes = readFileSync(path);
  const text = bytes[0] === 0xff && bytes[1] === 0xfe ? bytes.subarray(2).toString('utf16le') : bytes.toString('utf8');
  return { state: /^state=(.*)$/m.exec(text)?.[1]?.trim(), detail: /^detail=(.*)$/m.exec(text)?.[1]?.trim() };
}

function nsisText(value) { return value.replaceAll('$', '$$').replaceAll('"', '$\\"'); }

function compileHookFixture(item, options, action) {
  const executable = join(item.directory, 'hook-installer.exe');
  const source = join(item.directory, 'hook-installer.nsi');
  const snapshot = join(item.directory, 'nsis-status.ini');
  const callback = [
    '  Call NomiFunMaintenanceAbort',
    ...(action === 'cancel-before-start' ? [
      `  WriteINIStr "${nsisText(join(item.directory, 'early-recovery.ini'))}" "maintenance" "state" "$NomiFunMaintenanceFinished"`,
      '  Call NomiFunMaintenanceAbort',
      `  WriteINIStr "${nsisText(join(item.directory, 'early-recovery.ini'))}" "maintenance" "detail" "$NomiFunMaintenanceFinished"`,
    ] : []),
    ...(action === 'cancel-before-start' ? [] : [
      `  CopyFiles /SILENT "$NomiFunMaintenanceSession\\status.ini" "${nsisText(snapshot)}"`,
      `  CopyFiles /SILENT "$NomiFunMaintenanceSession\\outcome.json" "${nsisText(join(item.directory, 'nsis-outcome.json'))}"`,
    ]),
  ];
  writeFileSync(source, [
    'Unicode true', 'RequestExecutionLevel user', 'SilentInstall silent', 'SetCompress off',
    '!include "LogicLib.nsh"', '!include "FileFunc.nsh"', 'Var UpdateMode',
    '!define MAINBINARYNAME "nomifun-keyboard-fixture"',
    `!define MAINBINARYSRCPATH "${nsisText(join(item.payload, 'nomifun-keyboard-fixture.exe'))}"`,
    `!define NOMIFUN_MAINTENANCE_BINARY "${nsisText(options.helper)}"`,
    `!include "${nsisText(join(repository, 'apps/desktop/nsis/keyboard-hooks.nsh'))}"`,
    '!insertmacro NomiFunMaintenanceFunctions "" "no"',
    `OutFile "${nsisText(executable)}"`, `InstallDir "${nsisText(item.install)}"`,
    'Function .onInstSuccess', ...callback, 'FunctionEnd',
    'Function .onInstFailed', ...callback, 'FunctionEnd',
    'Function .onGUIEnd', ...callback, 'FunctionEnd',
    'Section',
    ...(action === 'cancel-before-start' ? [
      '  StrCpy $UpdateMode 1', '  Abort "fixture requested cancellation before maintenance start"',
    ] : [
      '  InitPluginsDir', '  SetOutPath "$PLUGINSDIR\\payload"', `  File /r "${nsisText(item.payload)}\\*"`,
      '  !insertmacro NSIS_HOOK_PREINSTALL', '  Call NomiFunMaintenanceApply',
      ...(action === 'cancel' ? ['  Abort "fixture requested cancellation after apply"'] : ['  Call NomiFunMaintenanceCommit']),
    ]),
    'SectionEnd',
  ].join('\n'));
  const compiler = options.makensis ?? join(process.env.LOCALAPPDATA, 'tauri/NSIS/makensis.exe');
  assert(existsSync(compiler), 'NSIS compiler is required for native hook validation');
  const compiled = spawnSync(compiler, ['/V2', source], { encoding: 'utf8', windowsHide: true });
  assert.equal(compiled.status, 0, `NSIS fixture compilation: ${compiled.stderr ?? compiled.error?.message}\n${compiled.stdout}`);
  return { executable, snapshot };
}

async function until(predicate, description, timeout = 15000) {
  const deadline = Date.now() + timeout;
  while (!predicate()) {
    if (Date.now() >= deadline) throw new Error(`Timed out: ${description}`);
    await pause(30);
  }
}

function process_(program, args, environment, children) {
  const child = spawn(program, args, { env: environment, windowsHide: true, stdio: ['ignore', 'pipe', 'pipe'] });
  let output = '';
  for (const stream of [child.stdout, child.stderr]) stream.on('data', bytes => { if (output.length < 8192) output += bytes.toString(); });
  const result = new Promise(done => {
    child.once('error', error => done({ code: null, error: error.message }));
    child.once('exit', (code, signal) => done({ code, signal, output }));
  });
  const handle = { child, result, diagnostics: () => ({ pid: child.pid, executable: program, code: child.exitCode, signal: child.signalCode, output }) };
  children.push(handle);
  return handle;
}

async function finished(handle, timeout = 20000) {
  let timer;
  try {
    return await Promise.race([handle.result, new Promise((_, reject) => {
      timer = setTimeout(() => reject(new Error(`Process ${handle.child.pid} did not finish`)), timeout);
    })]);
  } finally { clearTimeout(timer); }
}

function privateEnvironment(root) {
  const environment = { ...process.env, LOCALAPPDATA: join(root, 'local'), APPDATA: join(root, 'roaming'),
    HOME: join(root, 'home'), USERPROFILE: join(root, 'home'), NOMIFUN_DATA_DIR: join(root, 'data') };
  for (const name of Object.keys(environment)) {
    if (/(?:API_KEY|ACCESS_TOKEN|AUTH_TOKEN|SECRET|PASSWORD|CREDENTIAL)$/i.test(name)) delete environment[name];
  }
  for (const directory of [environment.LOCALAPPDATA, environment.APPDATA, environment.HOME, environment.NOMIFUN_DATA_DIR]) mkdirSync(directory, { recursive: true });
  return environment;
}

const legacySource = String.raw`
use std::{env, fs::{self,OpenOptions}, io::Write, path::PathBuf, thread, time::{Duration,Instant}};
use std::os::windows::fs::OpenOptionsExt;
fn main() {
    let args: Vec<String> = env::args().skip(1).collect();
    if args.first().map(String::as_str) == Some("--lock") {
        let _file = OpenOptions::new().read(true).share_mode(1).open(&args[1]).unwrap();
        fs::write(&args[2], b"locked").unwrap();
        let end=Instant::now()+Duration::from_secs(60);
        while !PathBuf::from(&args[3]).exists() && Instant::now()<end { thread::sleep(Duration::from_millis(20)); }
        return;
    }
    let log=PathBuf::from(env::var_os("NOMIFUN_LEGACY_FIXTURE_LOG").unwrap());
    let mut file=OpenOptions::new().create(true).append(true).open(log).unwrap();
    writeln!(file,"pid={} args={:?}", std::process::id(), args).unwrap();
    let stop=PathBuf::from(env::var_os("NOMIFUN_LEGACY_FIXTURE_STOP").unwrap());
    let end=Instant::now()+Duration::from_secs(60);
    while !stop.exists() && Instant::now()<end { thread::sleep(Duration::from_millis(20)); }
}`;

export function buildLegacy(root) {
  const source = join(root, 'legacy.rs'); const binary = join(root, 'legacy.exe');
  writeFileSync(source, legacySource);
  const sysroot = spawnSync('rustc', ['--print', 'sysroot'], { encoding: 'utf8', windowsHide: true });
  assert.equal(sysroot.status, 0, sysroot.stderr);
  const linker = join(sysroot.stdout.trim(), 'lib/rustlib/x86_64-pc-windows-msvc/bin/rust-lld.exe');
  const compiled = spawnSync('rustc', ['--edition=2024', '-C', `linker=${linker}`, '-O', source, '-o', binary], { encoding: 'utf8', windowsHide: true });
  assert.equal(compiled.status, 0, compiled.stderr);
  return binary;
}

function fixture(root, name, fixtureBinary) {
  const directory = join(root, name); mkdirSync(directory);
  const environment = privateEnvironment(directory);
  const install = join(directory, 'install'); const payload = join(directory, 'payload');
  mkdirSync(install); mkdirSync(payload);
  const installed = join(install, 'nomifun-keyboard-fixture.exe');
  copyFileSync(fixtureBinary, installed); copyFileSync(fixtureBinary, join(payload, 'nomifun-keyboard-fixture.exe'));
  // A PE overlay changes the release identity without changing executable code.
  // The success case must launch these different installed bytes; cancellation
  // and faults must restore the exact original executable, not only text files.
  appendFileSync(join(payload, 'nomifun-keyboard-fixture.exe'), '\nnomifun-upgrade-fixture-new-release\n');
  writeFileSync(join(install, 'a.txt'), 'old-a'); writeFileSync(join(payload, 'a.txt'), 'new-a');
  writeFileSync(join(install, 'z.txt'), 'old-z'); writeFileSync(join(payload, 'z.txt'), 'new-z');
  writeFileSync(join(environment.NOMIFUN_DATA_DIR, 'keyboard-settings.json'), '{"enabled":true,"launchAtLogin":false,"sentinel":"preserve"}\n');
  environment.NOMIFUN_LEGACY_FIXTURE_LOG = join(directory, 'legacy-arguments.log');
  environment.NOMIFUN_LEGACY_FIXTURE_STOP = join(directory, 'legacy-stop');
  const runtime = join(environment.LOCALAPPDATA, 'NomiFun/keyboard-service');
  return { directory, environment, install, installed, payload, runtime, originalExecutable: digest(installed),
    originalSettings: digest(join(environment.NOMIFUN_DATA_DIR, 'keyboard-settings.json')),
    desktop: join(runtime, 'desktop.json'), owner: join(runtime, 'owner.json'),
    settings: join(environment.NOMIFUN_DATA_DIR, 'keyboard-settings.json') };
}

function descriptorPid(path) { return existsSync(path) ? JSON.parse(readFileSync(path, 'utf8')).pid : null; }

async function startRuntime(item, children, { fail = false } = {}) {
  const gui = process_(item.installed, fail ? ['gui-fail'] : [], item.environment, children);
  const service = process_(item.installed, ['--keyboard-service'], item.environment, children);
  await until(() => existsSync(item.desktop) && existsSync(item.owner), 'fixture GUI/service endpoints');
  assert.equal(descriptorPid(item.desktop), gui.child.pid);
  assert.equal(descriptorPid(item.owner), service.child.pid);
  return { gui, service, guiPid: gui.child.pid, servicePid: service.child.pid };
}

async function startMaintenance(item, helper, children, options = {}) {
  const session = join(item.directory, `session-${Date.now()}-${Math.random().toString(16).slice(2)}`);
  mkdirSync(session);
  const args = ['--session', session, '--installed', options.installed ?? item.installed, '--payload', item.payload,
    '--parent-pid', String(options.parentPid ?? process.pid), ...(options.noResume ? ['--no-resume'] : []),
    ...(options.resumeApp ? ['--resume-app'] : []), ...(options.deferAppCommit ? ['--defer-app-on-commit'] : [])];
  const handle = process_(helper, args, item.environment, children);
  await until(() => ['ready', 'error', 'repair_required'].includes(status(session).state), 'maintenance preparation', 30000);
  return { session, process: handle, state: () => status(session) };
}

async function command(maintenance, value, expected) {
  const pending = join(maintenance.session, 'command.next');
  writeFileSync(pending, value + '\n'); renameSync(pending, join(maintenance.session, 'command'));
  await until(() => [...expected, 'error', 'repair_required'].includes(maintenance.state().state), `maintenance ${value}`, 25000);
  assert(expected.includes(maintenance.state().state), `${value}: ${JSON.stringify(maintenance.state())}`);
}

function filesAre(item, prefix) {
  assert.equal(readFileSync(join(item.install, 'a.txt'), 'utf8'), `${prefix}-a`);
  assert.equal(readFileSync(join(item.install, 'z.txt'), 'utf8'), `${prefix}-z`);
  assert.equal(digest(item.settings), item.originalSettings, 'Keyboard preferences must remain byte-identical');
  assert.equal(digest(item.installed), prefix === 'old' ? item.originalExecutable : digest(join(item.payload, 'nomifun-keyboard-fixture.exe')),
    'Executable must match the complete old or new release');
}

async function restored(item, previous) {
  await until(() => descriptorPid(item.desktop) && descriptorPid(item.desktop) !== previous.guiPid
    && descriptorPid(item.owner) && descriptorPid(item.owner) !== previous.servicePid, 'restored GUI and service', 15000);
}

async function cleanup(item, helper, children) {
  // Finish any still-live test session before starting cleanup. Real user state
  // is never consulted: every descriptor and process belongs to this temp root.
  writeFileSync(item.environment.NOMIFUN_LEGACY_FIXTURE_STOP, 'stop-test-owned-legacy');
  try {
    const maintenance = await startMaintenance(item, helper, children, { noResume: true });
    if (maintenance.state().state === 'ready') await command(maintenance, 'abort', ['aborted']);
    await finished(maintenance.process);
  } catch { /* exact test-owned fallback below */ }
  const pids = [descriptorPid(item.desktop), descriptorPid(item.owner)].filter(Number.isInteger);
  if (pids.length) {
    // A PID alone is insufficient: match the unique temp installation image
    // before stopping a resumed fixture process that is not our direct child.
    const script = join(item.directory, 'stop-owned.ps1');
    writeFileSync(script, `$ErrorActionPreference='Stop'\n$expected=${quotePowerShell(item.installed)}\nforeach($ownedPid in @(${pids.join(',')})){\n  $process=Get-CimInstance Win32_Process -Filter "ProcessId=$ownedPid"\n  if($process -and [String]::Equals($process.ExecutablePath,$expected,[StringComparison]::OrdinalIgnoreCase)){Stop-Process -Id $ownedPid -Force}\n}\n`);
    spawnSync('powershell.exe', ['-NoProfile', '-NonInteractive', '-ExecutionPolicy', 'Bypass', '-File', script], { encoding: 'utf8', windowsHide: true });
  }
}

export async function runKeyboardUpgradeSmoke(options) {
  assert.equal(process.platform, 'win32', 'Native upgrade witness requires Windows');
  for (const name of ['helper', 'fixture']) assert(options[name] && existsSync(options[name]), `Pass --${name} with the built native executable`);
  const root = mkdtempSync(join(tmpdir(), 'nomifun-keyboard-upgrade-'));
  const reportPath = options.report ?? join(root, 'report.json');
  const report = { schema: 1, platform: 'windows', root, helper: options.helper, helperSha256: digest(options.helper), fixture: options.fixture,
    fixtureSha256: digest(options.fixture), isolation: { privateInstall: true, privateRuntime: true, userDataUntouched: true, hkcuUntouched: true,
      serialEnumeration: false, microphoneCapture: false }, evidence: 'native helper, Windows file locks and authenticated IPC; synthetic GUI/device runtime',
    hookSha256: digest(join(repository, 'apps/desktop/nsis/keyboard-hooks.nsh')),
    templateSha256: digest(join(repository, 'apps/desktop/nsis/installer.nsi')),
    cases: [], unexecuted: ['physical keyboard K1/K4 and microphone', 'full production Tauri GUI', 'released old application binary'], ok: false };
  const children = []; const items = [];
  const legacy = buildLegacy(root);
  const run = async (name, action) => {
    const item = fixture(root, name, options.fixture); items.push(item);
    const started = Date.now(); const result = { name, ok: false };
    try { await action(item); result.ok = true; }
    catch (error) { result.error = error.stack ?? String(error); throw error; }
    finally { result.elapsedMs = Date.now() - started; report.cases.push(result); await cleanup(item, options.helper, children); }
  };
  try {
    await run('closed-legacy-upgrade', async item => {
      copyFileSync(legacy, item.installed);
      const maintenance = await startMaintenance(item, options.helper, children);
      assert.equal(maintenance.state().state, 'ready', JSON.stringify(maintenance.state()));
      await command(maintenance, 'apply', ['applied']); filesAre(item, 'new');
      await command(maintenance, 'commit', ['committed']); assert.equal((await finished(maintenance.process)).code, 0);
      assert(!existsSync(item.environment.NOMIFUN_LEGACY_FIXTURE_LOG), 'Legacy executable must never receive the new maintenance command');
      assert.equal(digest(item.installed), digest(join(item.payload, 'nomifun-keyboard-fixture.exe')));
    });
    await run('running-legacy-cancel-restores-old-gui', async item => {
      copyFileSync(legacy, item.installed); item.originalExecutable = digest(item.installed);
      const previous = process_(item.installed, [], item.environment, children);
      await until(() => existsSync(item.environment.NOMIFUN_LEGACY_FIXTURE_LOG), 'running legacy fixture');
      const maintenance = await startMaintenance(item, options.helper, children);
      assert.equal(maintenance.state().state, 'ready', JSON.stringify(maintenance.state()));
      // The old binary has no maintenance IPC. This exact-owned close models
      // the ordinary app-running check, never an unsupported old CLI command.
      writeFileSync(item.environment.NOMIFUN_LEGACY_FIXTURE_STOP, 'normal-old-app-exit');
      assert.equal((await finished(previous)).code, 0); unlinkSync(item.environment.NOMIFUN_LEGACY_FIXTURE_STOP);
      await command(maintenance, 'apply', ['applied']); await command(maintenance, 'abort', ['aborted']);
      assert.equal((await finished(maintenance.process)).code, 0); filesAre(item, 'old');
      await until(() => readFileSync(item.environment.NOMIFUN_LEGACY_FIXTURE_LOG, 'utf8').trim().split('\n').length === 2, 'old legacy GUI restored');
      const invocations = readFileSync(item.environment.NOMIFUN_LEGACY_FIXTURE_LOG, 'utf8');
      assert(!invocations.includes('--keyboard-stop') && !invocations.includes('--keyboard-unregister'), 'Old GUI must only be resumed normally');
    });
    await run('success-restores-runtime', async item => {
      const previous = await startRuntime(item, children);
      const maintenance = await startMaintenance(item, options.helper, children);
      assert.equal(maintenance.state().state, 'ready', JSON.stringify(maintenance.state()));
      assert.equal((await finished(previous.gui)).code, 0); assert.equal((await finished(previous.service)).code, 0);
      const rejectedGui = process_(item.installed, [], item.environment, children);
      const rejectedService = process_(item.installed, ['--keyboard-service'], item.environment, children);
      await finished(rejectedGui); await finished(rejectedService);
      assert(!existsSync(item.desktop) && !existsSync(item.owner), 'Maintenance must fence replacement GUI and daemon admission');
      await command(maintenance, 'apply', ['applied']); filesAre(item, 'new');
      await command(maintenance, 'commit', ['committed']); assert.equal((await finished(maintenance.process)).code, 0);
      await restored(item, previous);
    });
    await run('disabled-voice-preserves-background-service', async item => {
      writeFileSync(item.settings, '{"enabled":false,"launchAtLogin":false,"sentinel":"preserve-disabled"}\n');
      item.originalSettings = digest(item.settings);
      const service = process_(item.installed, ['--keyboard-service'], item.environment, children);
      await until(() => existsSync(item.owner), 'background-only service endpoint');
      const maintenance = await startMaintenance(item, options.helper, children);
      assert.equal(maintenance.state().state, 'ready');
      assert.equal((await finished(service)).code, 0);
      await command(maintenance, 'apply', ['applied']); await command(maintenance, 'commit', ['committed']);
      assert.equal((await finished(maintenance.process)).code, 0); filesAre(item, 'new');
      await until(() => descriptorPid(item.owner) && descriptorPid(item.owner) !== service.child.pid, 'background-only service restored');
      assert(!existsSync(item.desktop), 'Updating a background-only keyboard must not create an unwanted GUI');
    });
    for (const applied of [false, true]) {
      await run(applied ? 'cancel-after-apply-restores-runtime-and-files' : 'cancel-before-apply-restores-runtime', async item => {
        const previous = await startRuntime(item, children);
        const maintenance = await startMaintenance(item, options.helper, children);
        assert.equal(maintenance.state().state, 'ready');
        if (applied) { await command(maintenance, 'apply', ['applied']); filesAre(item, 'new'); }
        await command(maintenance, 'abort', ['aborted']); assert.equal((await finished(maintenance.process)).code, 0);
        filesAre(item, 'old'); await restored(item, previous);
      });
    }
    await run('unexpected-command-restores-runtime-and-files', async item => {
      const previous = await startRuntime(item, children);
      const maintenance = await startMaintenance(item, options.helper, children);
      assert.equal(maintenance.state().state, 'ready');
      await command(maintenance, 'apply', ['applied']); await command(maintenance, 'unexpected', ['error']);
      await finished(maintenance.process); filesAre(item, 'old'); await restored(item, previous);
    });
    for (const action of ['commit', 'abort']) {
      await run(`updater-handoff-${action}`, async item => {
        const maintenance = await startMaintenance(item, options.helper, children, { resumeApp: true, deferAppCommit: true });
        assert.equal(maintenance.state().state, 'ready'); await command(maintenance, 'apply', ['applied']);
        await command(maintenance, action, [action === 'commit' ? 'committed' : 'aborted']);
        await finished(maintenance.process); filesAre(item, action === 'commit' ? 'new' : 'old');
        if (action === 'commit') {
          assert(!existsSync(item.desktop), 'Commit must leave GUI restart to the updater /R handoff');
          const gui = process_(item.installed, [], item.environment, children);
          await until(() => descriptorPid(item.desktop) === gui.child.pid, 'updater-owned new GUI restart');
        } else {
          await until(() => descriptorPid(item.desktop), 'aborted updater handoff restores old GUI');
        }
      });
    }
    await run('sharing-denied-rolls-back-partial-copy', async item => {
      const previous = await startRuntime(item, children);
      const ready = join(item.directory, 'file-locked'); const release = join(item.directory, 'release-lock');
      const locker = process_(legacy, ['--lock', join(item.install, 'z.txt'), ready, release], item.environment, children);
      await until(() => existsSync(ready), 'Windows sharing-denied fixture');
      const maintenance = await startMaintenance(item, options.helper, children);
      assert.equal(maintenance.state().state, 'ready');
      await command(maintenance, 'apply', ['error']);
      writeFileSync(release, 'release'); await finished(locker);
      await finished(maintenance.process); filesAre(item, 'old'); await restored(item, previous);
    });
    await run('installer-parent-exit-rolls-back', async item => {
      const parent = process_(legacy, [], item.environment, children);
      await until(() => existsSync(item.environment.NOMIFUN_LEGACY_FIXTURE_LOG), 'test-owned installer parent');
      const maintenance = await startMaintenance(item, options.helper, children, { parentPid: parent.child.pid });
      assert.equal(maintenance.state().state, 'ready');
      await command(maintenance, 'apply', ['applied']); filesAre(item, 'new');
      writeFileSync(item.environment.NOMIFUN_LEGACY_FIXTURE_STOP, 'stop'); await finished(parent);
      await until(() => maintenance.state().state === 'aborted', 'parent death rollback'); await finished(maintenance.process);
      filesAre(item, 'old');
    });
    await run('stale-descriptors-do-not-block-upgrade', async item => {
      mkdirSync(item.runtime, { recursive: true });
      for (const filename of ['desktop.json', 'owner.json']) writeFileSync(join(item.runtime, filename), '{invalid stale descriptor');
      const maintenance = await startMaintenance(item, options.helper, children);
      assert.equal(maintenance.state().state, 'ready');
      await command(maintenance, 'apply', ['applied']); await command(maintenance, 'commit', ['committed']);
      assert.equal((await finished(maintenance.process)).code, 0); filesAre(item, 'new');
    });
    await run('native-stop-rejection-preserves-installation', async item => {
      const previous = await startRuntime(item, children, { fail: true });
      const maintenance = await startMaintenance(item, options.helper, children);
      assert.equal(maintenance.state().state, 'error'); await finished(maintenance.process);
      filesAre(item, 'old');
      assert.equal(descriptorPid(item.desktop), previous.guiPid, 'Rejected native stop must retain the live GUI');
    });
    await run('other-installation-is-not-stopped-or-restarted', async item => {
      const previous = await startRuntime(item, children);
      const other = join(item.directory, 'other-install'); mkdirSync(other);
      const installed = join(other, 'other.exe'); copyFileSync(options.fixture, installed);
      const maintenance = await startMaintenance(item, options.helper, children, { installed });
      assert.equal(maintenance.state().state, 'error'); await finished(maintenance.process);
      filesAre(item, 'old');
      assert.equal(descriptorPid(item.desktop), previous.guiPid); assert.equal(descriptorPid(item.owner), previous.servicePid);
      const events = readFileSync(join(item.runtime, 'events.log'), 'utf8').trim().split('\n');
      assert.equal(events.length, 2, 'Maintaining a different installation must not launch duplicate runtimes');
    });
    for (const action of ['success', 'cancel', 'failure']) {
      await run(`native-nsis-${action}`, async item => {
        const previous = await startRuntime(item, children);
        let locker; let release;
        if (action === 'failure') {
          const ready = join(item.directory, 'nsis-file-locked'); release = join(item.directory, 'nsis-release-lock');
          locker = process_(legacy, ['--lock', join(item.install, 'z.txt'), ready, release], item.environment, children);
          await until(() => existsSync(ready), 'NSIS Windows sharing-denied fixture');
        }
        const compiled = compileHookFixture(item, options, action);
        const installer = process_(compiled.executable, ['/S'], item.environment, children);
        const result = await finished(installer, 45000);
        if (release) { writeFileSync(release, 'release'); await finished(locker); }
        assert(existsSync(compiled.snapshot), 'NSIS exit callback must preserve maintenance outcome');
        const expected = action === 'success' ? 'committed' : action === 'cancel' ? 'aborted' : 'error';
        assert.equal(iniStatus(compiled.snapshot).state, expected, JSON.stringify({ result, state: iniStatus(compiled.snapshot) }));
        if (action === 'success') assert.equal(result.code, 0);
        filesAre(item, action === 'success' ? 'new' : 'old'); await restored(item, previous);
      });
    }
    await run('native-nsis-cancel-before-helper-restores-updater-gui-once', async item => {
      assert(!existsSync(item.desktop) && !existsSync(item.owner));
      const compiled = compileHookFixture(item, options, 'cancel-before-start');
      await finished(process_(compiled.executable, ['/S'], item.environment, children), 45000);
      filesAre(item, 'old');
      await until(() => descriptorPid(item.desktop), 'Early updater cancellation restores unchanged installed GUI');
      assert(!existsSync(item.owner), 'Early cancellation must not start an unrequested service');
      assert.deepEqual(iniStatus(join(item.directory, 'early-recovery.ini')), { state: '1', detail: '1' }, 'Repeated recovery callbacks must retain the finished marker');
      const events = readFileSync(join(item.runtime, 'events.log'), 'utf8').trim().split('\n');
      assert.equal(events.filter(event => event.startsWith('gui ')).length, 1, 'Repeated Abort callbacks must leave exactly one GUI owner');
    });
    report.ok = true;
  } catch (error) { report.error = error.stack ?? String(error); }
  finally {
    for (const child of children) if (child.child.exitCode === null && child.child.signalCode === null) child.child.kill();
    await Promise.allSettled(children.map(child => finished(child, 4000)));
    report.fixtureProcesses = children.map(child => child.diagnostics());
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
  const result = await runKeyboardUpgradeSmoke(arguments_(process.argv.slice(2)));
  console.log(JSON.stringify(result)); process.exitCode = result.ok ? 0 : 1;
}
