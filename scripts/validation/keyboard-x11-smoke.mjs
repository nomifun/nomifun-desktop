#!/usr/bin/env node
// Private X11 server + real window manager. Does not touch the user's display,
// clipboard, model configuration, login items or NomiFun database.
import { spawn, spawnSync } from 'node:child_process';
import { existsSync, mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { isAbsolute, join, resolve } from 'node:path';

if (process.platform !== 'linux' || !process.argv[2] || !isAbsolute(process.argv[2])) throw new Error('Pass the absolute Linux keyboard_window_validation executable path');
const binary = process.argv[2];
const profile = mkdtempSync(join(tmpdir(), 'nomifun-keyboard-x11-'));
const reportPath = resolve(process.argv[3] ?? join(profile, 'result.json'));
const x11Socket = (number) => existsSync(`/tmp/.X11-unix/X${number}`) || readFileSync('/proc/net/unix', 'utf8').split('\n').some((line) => line.endsWith(`/tmp/.X11-unix/X${number}`));
const displayNumber = Array.from({ length: 20 }, (_, offset) => 81 + offset).find((number) => !x11Socket(number) && !existsSync(`/tmp/.X${number}-lock`));
if (!displayNumber) throw new Error('No free private X11 display');
const display = `:${displayNumber}`;
for (const name of ['home', 'runtime', 'config', 'data']) mkdirSync(join(profile, name), { mode: 0o700 });
const environment = { ...process.env, DISPLAY: display, HOME: join(profile, 'home'), XDG_RUNTIME_DIR: join(profile, 'runtime'),
  XDG_CONFIG_HOME: join(profile, 'config'), XDG_DATA_HOME: join(profile, 'data'), NOMIFUN_DATA_DIR: join(profile, 'data'),
  GDK_BACKEND: 'x11', LIBGL_ALWAYS_SOFTWARE: '1', NOMIFUN_KEYBOARD_VALIDATION_REPORT: reportPath,
  NOMIFUN_KEYBOARD_VALIDATION_ALLOW_CLIPBOARD: '1', NOMIFUN_KEYBOARD_VALIDATION_ENVIRONMENT: 'Isolated X11: Xvfb + Openbox' };
delete environment.WAYLAND_DISPLAY;
const children = [];
const start = (program, arguments_) => { const child = spawn(program, arguments_, { env: environment, stdio: ['ignore', 'ignore', 'pipe'] }); let error = ''; child.stderr.on('data', (bytes) => { if (error.length < 4096) error += bytes.toString(); }); const exited = new Promise((done) => { child.once('exit', (code) => done({ code, error })); child.once('error', (failure) => done({ code: null, error: failure.message })); }); const value = { child, exited }; children.push(value); return value; };
const pause = (milliseconds) => new Promise((done) => setTimeout(done, milliseconds));
const wait = async (child, milliseconds) => { let timer; try { return await Promise.race([child.exited, new Promise((_, fail) => { timer = setTimeout(() => fail(new Error('X11 witness timed out')), milliseconds); })]); } finally { clearTimeout(timer); } };
const sentinel = Buffer.from('NomiFun private X11 clipboard sentinel\n中文 👋\n');
let report;
try {
  rmSync(reportPath, { force: true });
  start('Xvfb', [display, '-screen', '0', '1280x900x24', '-nolisten', 'tcp', '-ac', '-noreset']);
  const readyDeadline = Date.now() + 5000;
  while (!x11Socket(displayNumber)) { if (Date.now() >= readyDeadline) throw new Error('Private X11 server did not start'); await pause(40); }
  start('openbox', ['--sm-disable']);
  let ready = false;
  for (let attempt = 0; attempt < 40; attempt++) {
    const query = spawnSync('xprop', ['-root', '_NET_SUPPORTING_WM_CHECK'], { env: environment, encoding: 'utf8', timeout: 1000 });
    if (query.status === 0 && query.stdout.includes('window id #')) { ready = true; break; }
    await pause(50);
  }
  if (!ready) throw new Error('Private Openbox window manager did not become ready');
  const seeded = spawnSync('timeout', ['4s', 'xclip', '-selection', 'clipboard', '-in'], { env: environment, input: sentinel, stdio: ['pipe', 'ignore', 'ignore'], timeout: 5000 });
  if (seeded.status !== 0) throw new Error('Private clipboard fixture did not initialize');
  const native = start(binary, []);
  const completed = await wait(native, 35000);
  if (!existsSync(reportPath)) throw new Error(`Native X11 witness did not report: ${completed.error}`);
  report = JSON.parse(readFileSync(reportPath, 'utf8'));
  const restored = spawnSync('timeout', ['4s', 'xclip', '-selection', 'clipboard', '-out'], { env: environment, timeout: 5000 });
  report.clipboardRestoredAfterWitnessExit = restored.status === 0 && restored.stdout.equals(sentinel);
  report.profileIsolated = true;
  report.userDisplayUntouched = true;
  report.privateX11SocketPath = existsSync(`/tmp/.X11-unix/X${displayNumber}`) ? 'filesystem Unix socket' : 'abstract Unix socket';
  report.xvfbPackage = spawnSync('dpkg-query', ['-W', '-f=${Version}', 'xvfb'], { encoding: 'utf8' }).stdout.trim();
  report.openboxVersion = spawnSync('openbox', ['--version'], { encoding: 'utf8' }).stdout.split('\n')[0];
  report.ok = report.ok === true && report.clipboardRestoredAfterWitnessExit;
} catch (error) {
  report = { ok: false, environment: 'Isolated X11: Xvfb + Openbox', profileIsolated: true, error: error instanceof Error ? error.message : String(error) };
} finally {
  for (const { child } of children.reverse()) if (child.exitCode === null && child.signalCode === null) child.kill('SIGTERM');
  mkdirSync(resolve(reportPath, '..'), { recursive: true });
  writeFileSync(reportPath, JSON.stringify(report, null, 2) + '\n');
  console.log(JSON.stringify({ ok: report.ok, report: reportPath }));
}
process.exitCode = report.ok ? 0 : 1;
