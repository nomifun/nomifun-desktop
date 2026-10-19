#!/usr/bin/env node
// Cold-start and authenticated-stop witness for the ordinary production main,
// without the native-validation feature, real credentials, microphone capture,
// user clipboard, login items or the user's display.
import { spawn, spawnSync } from 'node:child_process';
import { existsSync, mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { isAbsolute, join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

const inner = process.argv[2] === '--session';
const [binary, requestedReport, suppliedProfile] = process.argv.slice(inner ? 3 : 2);
if (process.platform !== 'linux' || !isAbsolute(binary ?? '')) throw new Error('Pass the absolute ordinary Linux desktop executable and an optional report path');
const profile = suppliedProfile ?? mkdtempSync(join(tmpdir(), 'nomifun-keyboard-production-gui-'));
const reportPath = resolve(requestedReport ?? join(profile, 'result.json'));
for (const directory of ['home', 'runtime', 'config', 'data', 'cache']) mkdirSync(join(profile, directory), { recursive: true, mode: 0o700 });
const environment = { ...process.env, HOME: join(profile, 'home'), XDG_RUNTIME_DIR: join(profile, 'runtime'), XDG_CONFIG_HOME: join(profile, 'config'),
  XDG_DATA_HOME: join(profile, 'data'), XDG_CACHE_HOME: join(profile, 'cache'), NOMIFUN_DATA_DIR: join(profile, 'data'),
  GDK_BACKEND: 'x11', LIBGL_ALWAYS_SOFTWARE: '1', NO_AT_BRIDGE: '1', PULSE_SERVER: `unix:${join(profile, 'no-audio-server')}` };
for (const key of ['WAYLAND_DISPLAY', 'XAUTHORITY', 'NOMI_API_KEY', 'NOMIFUN_API_KEY']) delete environment[key];
for (const key of Object.keys(environment)) if (/(?:API_KEY|ACCESS_TOKEN|AUTH_TOKEN|SECRET|PASSWORD|CREDENTIAL)$/i.test(key)) delete environment[key];
if (!inner) {
  delete environment.DBUS_SESSION_BUS_ADDRESS; delete environment.DISPLAY;
  writeFileSync(join(profile, 'data/keyboard-settings.json'), JSON.stringify({ settings: {
    enabled: true, k2Enabled: true, k3Enabled: true, k4Enabled: true, asrMode: 'streaming', asrModel: null, textModel: null, language: 'zh', audioDeviceId: null,
    dictionary: [], beautifyInstruction: '整理标点、语法、口头重复和段落，保留原意、事实、人名、数字、日期、代码和专有词。只输出整理后的完整正文。',
    alwaysOnTop: true, subtitleFontSize: 14, launchAtLogin: false } }));
  const child = spawn('dbus-run-session', ['--', process.execPath, fileURLToPath(import.meta.url), '--session', binary, reportPath, profile],
    { env: environment, detached: true, stdio: ['ignore', 'inherit', 'ignore'] });
  process.exitCode = await new Promise((done) => { child.once('exit', (code) => done(code ?? 1)); child.once('error', () => done(1)); });
  try { process.kill(-child.pid, 'SIGTERM'); } catch (error) { if (error.code !== 'ESRCH') throw error; }
} else {
  const children = [];
  const start = (program, arguments_) => {
    const child = spawn(program, arguments_, { cwd: profile, env: environment, stdio: ['ignore', 'pipe', 'pipe'] });
    // Private diagnostic log only. Never emit backend URLs, tokens or content.
    const log = join(profile, 'production.log');
    for (const stream of [child.stdout, child.stderr]) stream.on('data', (bytes) => { writeFileSync(log, bytes, { flag: 'a' }); });
    const exited = new Promise((done) => { child.once('exit', (code) => done(code)); child.once('error', () => done(-1)); });
    const value = { child, exited }; children.push(value); return value;
  };
  const pause = (milliseconds) => new Promise((done) => setTimeout(done, milliseconds));
  const wait = async (child, milliseconds) => {
    let timer;
    try { return await Promise.race([child.exited, new Promise((_, fail) => { timer = setTimeout(() => fail(new Error('Production process did not exit within its deadline')), milliseconds); })]); }
    finally { clearTimeout(timer); }
  };
  const alive = (pid) => { try { process.kill(pid, 0); return true; } catch (error) { if (error.code === 'ESRCH') return false; throw error; } };
  const root = join(profile, 'runtime/NomiFun/keyboard-service');
  const desktop = join(root, 'desktop.json'); const owner = join(root, 'owner.json');
  const report = { schema: 1, binary, ordinaryProductionMain: true, profileIsolated: true, sessionBusIsolated: true, userDisplayUntouched: true,
    modelsConfigured: false, microphoneCaptureAttempted: false, clipboardOperationsRequested: false,
    mainWindowVisible: false, desktopEndpointPublished: false, serialOwnerPublished: false,
    authenticatedStopSucceeded: false, guiExited: false, daemonExited: false, desktopDescriptorRemoved: false, ownerDescriptorRemoved: false,
    loginNotRegistered: false, ok: false };
  try {
    rmSync(reportPath, { force: true });
    const display = Array.from({ length: 20 }, (_, offset) => offset + 81).find((number) => !existsSync(`/tmp/.X11-unix/X${number}`) && !existsSync(`/tmp/.X${number}-lock`));
    if (!display) throw new Error('No private X11 display available');
    environment.DISPLAY = `:${display}`;
    start('Xvfb', [environment.DISPLAY, '-screen', '0', '1280x900x24', '-nolisten', 'tcp', '-ac', '-noreset']);
    let deadline = Date.now() + 5000;
    while (!existsSync(`/tmp/.X11-unix/X${display}`)) { if (Date.now() >= deadline) throw new Error('Private X11 display did not start'); await pause(40); }
    start('openbox', ['--sm-disable']);
    let wmReady = false;
    for (let attempt = 0; attempt < 40; attempt++) {
      const query = spawnSync('xprop', ['-root', '_NET_SUPPORTING_WM_CHECK'], { env: environment, encoding: 'utf8', timeout: 1000 });
      if (query.status === 0 && query.stdout.includes('window id #')) { wmReady = true; break; }
      await pause(50);
    }
    if (!wmReady) throw new Error('Private Openbox did not start');
    const gui = start(binary, ['--keyboard-activate']);
    deadline = Date.now() + 45000;
    let window;
    while (!(report.mainWindowVisible && report.desktopEndpointPublished && report.serialOwnerPublished)) {
      if (gui.child.exitCode !== null) throw new Error(`Ordinary GUI exited before startup completed (code ${gui.child.exitCode})`);
      if (Date.now() >= deadline) throw new Error('Ordinary GUI did not publish its main window and both endpoints within 45 seconds');
      const visible = spawnSync('xdotool', ['search', '--onlyvisible', '--pid', String(gui.child.pid), '--name', '^NomiFun$'], { env: environment, encoding: 'utf8', timeout: 1000 });
      window = visible.stdout?.trim().split('\n')[0];
      report.mainWindowVisible = visible.status === 0 && /^\d+$/.test(window);
      report.desktopEndpointPublished = existsSync(desktop);
      report.serialOwnerPublished = existsSync(owner);
      await pause(80);
    }
    const geometry = spawnSync('xdotool', ['getwindowgeometry', '--shell', window], { env: environment, encoding: 'utf8', timeout: 1000 }).stdout;
    report.mainWindowSize = { width: Number(geometry.match(/^WIDTH=(\d+)$/m)?.[1]), height: Number(geometry.match(/^HEIGHT=(\d+)$/m)?.[1]) };
    if (report.mainWindowSize.width < 880 || report.mainWindowSize.height < 600) throw new Error('Ordinary main window violated the 880×600 contract');
    const desktopPid = JSON.parse(readFileSync(desktop, 'utf8')).pid;
    const ownerPid = JSON.parse(readFileSync(owner, 'utf8')).pid;
    if (desktopPid !== gui.child.pid || !Number.isInteger(ownerPid) || !alive(ownerPid)) throw new Error('Native endpoint process identity did not match the running production processes');
    const stop = start(binary, ['--keyboard-stop']);
    if (await wait(stop, 90000) !== 0) throw new Error('Authenticated production stop returned failure');
    report.authenticatedStopSucceeded = true;
    report.guiExited = await wait(gui, 5000) === 0;
    deadline = Date.now() + 5000;
    while (alive(ownerPid) && Date.now() < deadline) await pause(40);
    report.daemonExited = !alive(ownerPid);
    report.desktopDescriptorRemoved = !existsSync(desktop);
    report.ownerDescriptorRemoved = !existsSync(owner);
    report.loginNotRegistered = !existsSync(join(profile, 'config/autostart/nomifun-keyboard.desktop'));
    report.ok = report.guiExited && report.daemonExited && report.desktopDescriptorRemoved && report.ownerDescriptorRemoved && report.loginNotRegistered;
  } catch (error) { report.error = error instanceof Error ? error.message : String(error); }
  finally {
    for (const { child } of children.reverse()) if (child.exitCode === null && child.signalCode === null) child.kill('SIGTERM');
    await Promise.all(children.map((child) => wait(child, 5000).catch(() => { child.child.kill('SIGKILL'); })));
    mkdirSync(resolve(reportPath, '..'), { recursive: true }); writeFileSync(reportPath, JSON.stringify(report, null, 2) + '\n');
    console.log(JSON.stringify({ ok: report.ok, report: reportPath })); process.exitCode = report.ok ? 0 : 1;
  }
}
