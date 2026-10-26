#!/usr/bin/env node
// Native Windows Actor recovery with synthetic audio and loopback providers.
// Its suspended process is assigned to a private kill-on-close job before any
// code runs; all profile, WebView2, temporary and model data belong to this run.
import { spawn } from 'node:child_process';
import { createHash } from 'node:crypto';
import { closeSync, existsSync, mkdirSync, mkdtempSync, openSync, readFileSync, readSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { dirname, isAbsolute, join } from 'node:path';
import { fileURLToPath } from 'node:url';

const usage = 'node scripts/validation/keyboard-native-update-recovery.mjs --binary <absolute keyboard_native_validation.exe> --mode <streaming|platform> --frontend-build-id <UUID> --report <absolute new JSON>';
const pause = milliseconds => new Promise(done => setTimeout(done, milliseconds));
const json = path => JSON.parse(readFileSync(path, 'utf8').replace(/^\uFEFF/, ''));

function parse(arguments_) {
  if (arguments_.length === 1 && arguments_[0] === '--help') return null;
  const options = {};
  for (let index = 0; index < arguments_.length; index += 2) {
    const key = arguments_[index], value = arguments_[index + 1];
    if (!['--binary', '--mode', '--frontend-build-id', '--report'].includes(key) || Object.hasOwn(options, key) || !value || value.startsWith('--')) throw new Error(usage);
    options[key] = value;
  }
  if (Object.keys(options).length !== 4 || !['streaming', 'platform'].includes(options['--mode'])
    || ![options['--binary'], options['--report']].every(isAbsolute)
    || !/^[0-9a-f]{8}(?:-[0-9a-f]{4}){3}-[0-9a-f]{12}$/i.test(options['--frontend-build-id'])) throw new Error(usage);
  return { binary: options['--binary'], mode: options['--mode'], buildId: options['--frontend-build-id'], report: options['--report'] };
}

function inspectBinary(options) {
  const required = [options.buildId, '.synthetic-audio-fixture', 'successful_handoff_keeps_old_runtime_paused'];
  const seen = required.map(() => false), hash = createHash('sha256');
  const handle = openSync(options.binary, 'r'), buffer = Buffer.alloc(1024 * 1024);
  let tail = Buffer.alloc(0), first = true;
  try {
    for (;;) {
      const count = readSync(handle, buffer);
      if (!count) break;
      if (first && buffer.subarray(0, 2).toString('ascii') !== 'MZ') throw new Error('Pass a Windows native fixture executable');
      first = false;
      hash.update(buffer.subarray(0, count));
      const chunk = Buffer.concat([tail, buffer.subarray(0, count)]);
      required.forEach((value, index) => { seen[index] ||= chunk.includes(Buffer.from(value)); });
      tail = chunk.subarray(Math.max(0, chunk.length - 256));
    }
  } finally { closeSync(handle); }
  if (first || !seen.every(Boolean)) throw new Error('Fixture lacks the exact frontend stamp, synthetic audio marker, or current update recovery scenarios');
  return hash.digest('hex');
}

function environment(profile) {
  const value = {};
  for (const key of ['SystemRoot', 'WINDIR', 'COMSPEC', 'ProgramFiles', 'ProgramFiles(x86)', 'ProgramW6432', 'ProgramData', 'ALLUSERSPROFILE', 'PROCESSOR_ARCHITECTURE', 'NUMBER_OF_PROCESSORS']) {
    if (process.env[key]) value[key] = process.env[key];
  }
  value.PATH = [join(process.env.SystemRoot, 'System32'), process.env.SystemRoot].join(';');
  value.RUST_MIN_STACK = '134217728';
  for (const [key, part] of Object.entries({ USERPROFILE: 'home', HOME: 'home', LOCALAPPDATA: 'local', APPDATA: 'roaming', TEMP: 'tmp', TMP: 'tmp', NOMIFUN_DATA_DIR: 'data', WEBVIEW2_USER_DATA_FOLDER: 'webview2' })) {
    value[key] = join(profile, part);
    mkdirSync(value[key], { recursive: true });
  }
  return value;
}

function verifyRecovery(report, mode) {
  if (report.passed !== true || report.real_os_microphone !== false || report.asr_mode !== mode || !report.audio_source?.includes('synthetic') || report.provider_requests_loopback_only !== true) throw new Error('Actor fixture failed or violated its synthetic isolation');
  const recovery = report.update_recovery;
  if (recovery?.successful_handoff_keeps_old_runtime_paused !== true || recovery.cloud_completion_not_required !== true || recovery.automatic_recording_not_resumed !== true || recovery.disabled_and_unconfigured_keyboard_admit_maintenance !== true
    || !Array.isArray(recovery.cases) || recovery.cases.length !== 4
    || !['cancel_after_ready', 'cancel_during_native_stop', 'native_stop_timeout', 'external_native_stop_timeout'].every(scenario => recovery.cases.some(value => value.scenario === scenario && value.capture_recovered === true && value.old_hold_not_resumed === true && value.native_stop_proof_retained === true && value.preferences_preserved === true))) throw new Error('Missing native successful handoff, failed-stop, or cancelled-update recovery evidence');
}

async function run(options) {
  if (process.platform !== 'win32') throw new Error('Run this native Windows witness on Windows');
  if (existsSync(options.report)) throw new Error('Report already exists; pass a new report path');
  const binarySha256 = inspectBinary(options);
  const profile = mkdtempSync(join(tmpdir(), 'nomifun-keyboard-update-actor-'));
  const configuration = join(profile, 'configuration.json');
  const fixtureReport = join(profile, 'fixture.json'), processReport = join(profile, 'process.json');
  const isolatedEnvironment = environment(profile);
  writeFileSync(configuration, JSON.stringify({ purpose: 'isolated-synthetic-keyboard-update-recovery-v1', binary: options.binary,
    mode: options.mode === 'streaming' ? '--synthetic-audio' : '--platform-asr-fixture', profile, fixtureReport, processReport,
    log: join(profile, 'fixture.log'), environment: isolatedEnvironment }));
  const launcherScript = fileURLToPath(new URL('./keyboard-native-update-recovery.ps1', import.meta.url));
  const launcher = spawn(join(process.env.SystemRoot, 'System32/WindowsPowerShell/v1.0/powershell.exe'),
    ['-NoProfile', '-NonInteractive', '-WindowStyle', 'Hidden', '-ExecutionPolicy', 'Bypass', '-File', launcherScript, '-Configuration', configuration],
    { cwd: profile, env: isolatedEnvironment, windowsHide: true, stdio: ['ignore', 'pipe', 'pipe'] });
  let diagnostics = '', settled = false, timer;
  launcher.stdout.on('data', value => { diagnostics += value; });
  launcher.stderr.on('data', value => { diagnostics += value; });
  const completed = new Promise((resolve, reject) => {
    launcher.once('error', reject);
    launcher.once('exit', (code, signal) => { settled = true; resolve({ code, signal }); });
  });
  const interrupt = () => { if (!settled) launcher.kill(); };
  process.on('SIGINT', interrupt); process.on('SIGTERM', interrupt);
  const report = { schema: 1, passed: false, binary: options.binary, binarySha256, frontendBuildId: options.buildId, mode: options.mode,
    profile, profileIsolated: true, realOsMicrophoneOpened: false, realSerialOwned: false, keyboardLoginRegistrationManaged: false,
    providers: 'loopback HTTP/WebSocket with fixture credentials', scope: 'Windows native production Actor recovery; package replacement is a separate witness' };
  try {
    const exit = await Promise.race([completed, new Promise((_, reject) => { timer = setTimeout(() => reject(new Error('Owned native fixture launcher exceeded 270 seconds')), 270000); })]);
    report.launcher = exit;
    if (exit.code !== 0) throw new Error(`Owned fixture launcher failed: ${diagnostics.slice(-6000)}`);
    report.process = json(processReport);
    report.fixture = json(fixtureReport);
    if (report.process.exitCode !== 0 || report.process.timedOut !== false || report.process.activeOwnedProcesses !== 0 || report.process.privateJobOwnedChildren !== true || report.process.suspendedBeforeJobAssignment !== true) throw new Error('Fixture process did not complete within its owned native job');
    verifyRecovery(report.fixture, options.mode);
    if (report.fixture.frontend_build_id !== options.buildId) throw new Error('Running fixture frontend identity differs from the requested build');
    report.passed = true;
  } catch (error) { report.error = error.message; }
  finally {
    clearTimeout(timer);
    if (!settled) {
      launcher.kill(); // closes the launcher's job handle; only this run's descendants are terminated
      await Promise.race([completed, pause(5000)]);
    }
    process.off('SIGINT', interrupt); process.off('SIGTERM', interrupt);
    if (diagnostics) writeFileSync(join(profile, 'launcher.log'), diagnostics);
    mkdirSync(dirname(options.report), { recursive: true });
    writeFileSync(options.report, JSON.stringify(report, null, 2) + '\n', { flag: 'wx' });
  }
  console.log(JSON.stringify({ passed: report.passed, mode: options.mode, report: options.report, profile, error: report.error }));
  process.exitCode = report.passed ? 0 : 1;
}

try {
  const options = parse(process.argv.slice(2));
  if (options === null) console.log(usage);
  else await run(options);
} catch (error) { console.error(error.message); process.exitCode = 1; }
