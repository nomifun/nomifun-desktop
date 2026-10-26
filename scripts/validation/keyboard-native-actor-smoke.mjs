#!/usr/bin/env node
// Current Actor regression on a private Linux desktop. Audio and providers are
// explicit synthetic fixtures; neither the user's session nor serial is used.
import { spawn, spawnSync } from 'node:child_process';
import { randomUUID, createHash } from 'node:crypto';
import { accessSync, closeSync, constants, existsSync, mkdirSync, mkdtempSync, openSync, readFileSync, readlinkSync, readdirSync, readSync, statSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { dirname, isAbsolute, join } from 'node:path';
import { fileURLToPath } from 'node:url';

const usage = 'node scripts/validation/keyboard-native-actor-smoke.mjs --binary <absolute Linux keyboard_native_validation> --mode <streaming|platform> --frontend-build-id <UUID> --report <absolute new private JSON>';
const pause = (milliseconds) => new Promise((done) => setTimeout(done, milliseconds));
const bounded = async (promise, milliseconds, message) => {
  let timer;
  try { return await Promise.race([promise, new Promise((_, fail) => { timer = setTimeout(() => fail(new Error(message)), milliseconds); })]); }
  finally { clearTimeout(timer); }
};
const signal = (pid, name) => { try { process.kill(pid, name); } catch (error) { if (error.code !== 'ESRCH') throw error; } };
const x11Socket = (number) => existsSync(`/tmp/.X11-unix/X${number}`) || readFileSync('/proc/net/unix', 'utf8').split('\n').some((line) => line.endsWith(`/tmp/.X11-unix/X${number}`));
const launch = (command, arguments_, environment, profile, logName, detached = false) => {
  const log = openSync(join(profile, logName), 'a', 0o600);
  const child = spawn(command, arguments_, { cwd: profile, env: environment, detached, stdio: ['ignore', log, log] });
  closeSync(log);
  const exited = new Promise((done) => {
    child.once('error', (error) => done({ code: null, error: error.message }));
    child.once('exit', (code, exitSignal) => done({ code, signal: exitSignal }));
  });
  return { child, exited };
};
function parse(arguments_) {
  if (arguments_.length === 1 && arguments_[0] === '--help') return null;
  const names = new Set(['--binary', '--mode', '--frontend-build-id', '--report']);
  const options = {};
  for (let index = 0; index < arguments_.length; index += 2) {
    const name = arguments_[index];
    const value = arguments_[index + 1];
    if (!names.has(name) || Object.hasOwn(options, name) || !value || value.startsWith('--')) throw new Error(`Invalid, repeated or incomplete argument: ${name ?? '(missing)'}. ${usage}`);
    options[name] = value;
  }
  if (!['--binary', '--mode', '--frontend-build-id', '--report'].every(name => options[name]) || !['streaming', 'platform'].includes(options['--mode'])) throw new Error(usage);
  if (![options['--binary'], options['--report']].every(isAbsolute)) throw new Error('Binary and report paths must be absolute');
  if (!/^[0-9a-f]{8}(?:-[0-9a-f]{4}){3}-[0-9a-f]{12}$/i.test(options['--frontend-build-id'])) throw new Error('Pass the exact current frontend UUID');
  return { binary: options['--binary'], mode: options['--mode'], buildId: options['--frontend-build-id'], report: options['--report'] };
}
function inspectBinary(options) {
  accessSync(options.binary, constants.X_OK);
  const file = openSync(options.binary, 'r');
  const hash = createHash('sha256');
  const required = [options.buildId, '.synthetic-audio-fixture', 'successful_handoff_keeps_old_runtime_paused'];
  const seen = required.map(() => false);
  const buffer = Buffer.alloc(1024 * 1024);
  let tail = Buffer.alloc(0);
  let first = true;
  try {
    for (;;) {
      const count = readSync(file, buffer);
      if (!count) break;
      if (first && !buffer.subarray(0, 4).equals(Buffer.from([0x7f, 0x45, 0x4c, 0x46]))) throw new Error('The Actor fixture must be a Linux ELF executable');
      first = false;
      hash.update(buffer.subarray(0, count));
      const chunk = Buffer.concat([tail, buffer.subarray(0, count)]);
      required.forEach((value, index) => { seen[index] ||= chunk.includes(Buffer.from(value)); });
      tail = chunk.subarray(Math.max(0, chunk.length - 256));
    }
  } finally { closeSync(file); }
  if (first || !seen.every(Boolean)) throw new Error('Executable lacks the current frontend stamp or the explicit synthetic fixture marker');
  return hash.digest('hex');
}
function environment(profile) {
  // Allowlist instead of inheriting user credentials, graphical sessions,
  // provider endpoints, audio-server addresses or desktop configuration.
  const value = { PATH: '/usr/bin:/bin', LANG: 'C.UTF-8', LC_ALL: 'C.UTF-8', GDK_BACKEND: 'x11', LIBGL_ALWAYS_SOFTWARE: '1', NO_AT_BRIDGE: '1', GSETTINGS_BACKEND: 'memory', RUST_MIN_STACK: '536870912' };
  for (const [key, directory] of Object.entries({ HOME: 'home', XDG_RUNTIME_DIR: 'runtime', XDG_CONFIG_HOME: 'config', XDG_DATA_HOME: 'data', XDG_CACHE_HOME: 'cache', TMPDIR: 'tmp', NOMIFUN_DATA_DIR: 'data' })) {
    value[key] = join(profile, directory);
    mkdirSync(value[key], { recursive: true, mode: 0o700 });
  }
  value.PULSE_SERVER = `unix:${join(profile, 'no-pulse-server')}`;
  value.PIPEWIRE_RUNTIME_DIR = value.XDG_RUNTIME_DIR;
  value.PIPEWIRE_REMOTE = 'no-pipewire-server';
  value.JACK_NO_START_SERVER = '1';
  return value;
}
function privateProcesses(profile) {
  const marker = Buffer.from(`HOME=${join(profile, 'home')}\0`);
  return readdirSync('/proc').filter((name) => /^\d+$/.test(name)).flatMap((name) => {
    try {
      const directory = `/proc/${name}`;
      if (statSync(directory).uid !== process.getuid()) return [];
      const cwd = readlinkSync(join(directory, 'cwd'));
      return cwd === profile || cwd.startsWith(`${profile}/`) || readFileSync(join(directory, 'environ')).includes(marker) ? [Number(name)] : [];
    } catch (error) { if (['ENOENT', 'ESRCH', 'EACCES', 'EPERM'].includes(error.code)) return []; throw error; }
  });
}
async function session(profile) {
  const configuration = join(profile, 'session.json');
  const details = JSON.parse(readFileSync(configuration, 'utf8'));
  if (statSync(profile).uid !== process.getuid() || (statSync(profile).mode & 0o077) !== 0 || details.token !== process.env.NOMIFUN_ACTOR_RUNNER_TOKEN || !process.env.DBUS_SESSION_BUS_ADDRESS) throw new Error('Private runner session proof is missing');
  const env = environment(profile);
  env.DBUS_SESSION_BUS_ADDRESS = process.env.DBUS_SESSION_BUS_ADDRESS;
  const children = [];
  const start = (command, arguments_, logName) => { const value = launch(command, arguments_, env, profile, logName); children.push(value); return value; };
  try {
    const sockets = readFileSync('/proc/net/unix', 'utf8');
    const display = Array.from({ length: 100 }, (_, offset) => 1900 + offset).find((number) => !existsSync(`/tmp/.X11-unix/X${number}`) && !existsSync(`/tmp/.X${number}-lock`) && !sockets.includes(`/tmp/.X11-unix/X${number}`));
    if (!display) throw new Error('No unused private X11 display');
    env.DISPLAY = `:${display}`;
    writeFileSync(join(profile, 'display.json'), JSON.stringify({ number: display }));
    const server = start('Xvfb', [env.DISPLAY, '-screen', '0', '1280x900x24', '-nolisten', 'tcp', '-ac', '-noreset'], 'xvfb.log');
    let deadline = Date.now() + 5000;
    while (!x11Socket(display)) {
      if (server.child.exitCode !== null || Date.now() >= deadline) throw new Error('Private Xvfb did not start');
      await pause(40);
    }
    start('openbox', ['--sm-disable'], 'openbox.log');
    deadline = Date.now() + 5000;
    for (;;) {
      const query = spawnSync('xprop', ['-root', '_NET_SUPPORTING_WM_CHECK'], { env, encoding: 'utf8', timeout: 1000 });
      if (query.status === 0 && query.stdout.includes('window id #')) break;
      if (Date.now() >= deadline) throw new Error('Private Openbox did not become ready');
      await pause(50);
    }
    const fixtureReport = join(profile, 'fixture.json');
    const argument = details.options.mode === 'streaming' ? '--synthetic-audio' : '--platform-asr-fixture';
    const fixture = start(details.options.binary, [argument, fixtureReport], 'fixture.log');
    const result = await bounded(fixture.exited, 135000, 'Synthetic Actor fixture exceeded 135 seconds');
    const report = JSON.parse(readFileSync(fixtureReport, 'utf8'));
    if (result.code !== 0 || report.passed !== true || report.real_os_microphone !== false || report.asr_mode !== details.options.mode || !report.audio_source?.includes('synthetic')) throw new Error('Actor fixture failed or violated its requested synthetic mode');
    const recovery = report.update_recovery;
    if (recovery?.successful_handoff_keeps_old_runtime_paused !== true || recovery.cloud_completion_not_required !== true || recovery.automatic_recording_not_resumed !== true || recovery.disabled_and_unconfigured_keyboard_admit_maintenance !== true
      || !Array.isArray(recovery.cases) || recovery.cases.length !== 4
      || !['cancel_after_ready', 'cancel_during_native_stop', 'native_stop_timeout', 'external_native_stop_timeout'].every(scenario => recovery.cases.some(value => value.scenario === scenario && value.capture_recovered === true && value.old_hold_not_resumed === true && value.native_stop_proof_retained === true && value.preferences_preserved === true))) {
      throw new Error('Actor fixture lacks successful handoff, failed-stop and cancelled-update recovery evidence');
    }
    writeFileSync(join(profile, 'session-result.json'), JSON.stringify({ passed: true, fixtureExitCode: result.code, display, x11SocketPath: existsSync(`/tmp/.X11-unix/X${display}`) ? 'filesystem Unix socket' : 'abstract Unix socket', dbusSessionIsolated: true }));
  } finally {
    for (const value of children.reverse()) if (value.child.exitCode === null && value.child.signalCode === null) value.child.kill('SIGTERM');
    await Promise.all(children.map(async (value) => {
      try { await bounded(value.exited, 4000, 'Private child did not stop'); }
      catch { value.child.kill('SIGKILL'); await bounded(value.exited, 4000, 'Private child survived SIGKILL'); }
    }));
  }
}
async function run(options) {
  if (process.platform !== 'linux') throw new Error('Run this Actor witness on Linux; it never connects to the Windows desktop');
  if (existsSync(options.report)) throw new Error('Report already exists; pass a new private report path');
  const binarySha256 = inspectBinary(options);
  const profile = mkdtempSync(join(tmpdir(), 'nomifun-native-actor-'));
  const token = randomUUID();
  writeFileSync(join(profile, 'session.json'), JSON.stringify({ options, token }), { mode: 0o600 });
  const env = environment(profile);
  env.NOMIFUN_ACTOR_RUNNER_TOKEN = token;
  const report = { schema: 1, passed: false, binary: options.binary, binarySha256, frontendBuildId: options.buildId, mode: options.mode, environment: 'private Xvfb + Openbox + DBus', profile, profileIsolated: true, userDisplayUntouched: true, userClipboardUntouched: true, realSerialOwned: false, realOsMicrophoneOpened: false, providers: 'loopback synthetic HTTP/WebSocket only; no user credentials or cloud', visualBoundary: 'Xvfb/Openbox does not assess compositor transparency appearance' };
  let interrupt;
  const interrupted = new Promise((_, fail) => { interrupt = () => fail(new Error('Runner interrupted; terminating only its private session')); });
  process.on('SIGINT', interrupt);
  process.on('SIGTERM', interrupt);
  let bus;
  try {
    bus = launch('dbus-run-session', ['--', process.execPath, fileURLToPath(import.meta.url), '--session', profile], env, profile, 'session.log', true);
    const completed = await bounded(Promise.race([bus.exited, interrupted]), 165000, 'Private Actor session exceeded 165 seconds');
    report.sessionExitCode = completed.code;
    report.fixture = JSON.parse(readFileSync(join(profile, 'fixture.json'), 'utf8'));
    if (report.fixture.passed !== true) throw new Error(report.fixture.error ?? 'Synthetic Actor fixture failed');
    report.session = JSON.parse(readFileSync(join(profile, 'session-result.json'), 'utf8'));
    report.passed = completed.code === 0 && report.fixture.passed === true && report.session.passed === true;
  } catch (error) { report.error = error.message; }
  finally {
    if (bus?.child.pid) signal(-bus.child.pid, 'SIGTERM');
    let remaining = privateProcesses(profile);
    for (const pid of remaining) signal(pid, 'SIGTERM');
    for (let attempt = 0; remaining.length && attempt < 40; attempt++) { await pause(50); remaining = privateProcesses(profile); }
    if (bus?.child.pid) signal(-bus.child.pid, 'SIGKILL');
    for (const pid of remaining) signal(pid, 'SIGKILL');
    if (bus) await bounded(bus.exited, 4000, 'Private DBus launcher did not exit');
    await pause(100);
    report.remainingPrivateProcesses = privateProcesses(profile);
    const display = existsSync(join(profile, 'display.json')) ? JSON.parse(readFileSync(join(profile, 'display.json'), 'utf8')).number : null;
    report.privateDisplayReleased = display === null || (!x11Socket(display) && !existsSync(`/tmp/.X${display}-lock`));
    report.passed &&= report.remainingPrivateProcesses.length === 0 && report.privateDisplayReleased;
    mkdirSync(dirname(options.report), { recursive: true, mode: 0o700 });
    writeFileSync(options.report, JSON.stringify(report, null, 2) + '\n', { flag: 'wx', mode: 0o600 });
    process.off('SIGINT', interrupt);
    process.off('SIGTERM', interrupt);
  }
  console.log(JSON.stringify({ passed: report.passed, mode: options.mode, report: options.report, profile }));
  process.exitCode = report.passed ? 0 : 1;
}

try {
  if (process.argv[2] === '--session') {
    if (process.argv.length !== 4 || !process.env.NOMIFUN_ACTOR_RUNNER_TOKEN || !isAbsolute(process.argv[3])) throw new Error('Internal session arguments are invalid');
    await session(process.argv[3]);
  } else {
    const options = parse(process.argv.slice(2));
    if (options === null) console.log(usage);
    else await run(options);
  }
} catch (error) { console.error(error.message); process.exitCode = 1; }
