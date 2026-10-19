#!/usr/bin/env node
// Real GNOME Shell/Mutter or KDE KWin nested Wayland compositor, with an
// isolated session bus, parent X server, HOME, runtime, display and clipboard.
// The GTK/Tauri witness deliberately uses the application's XWayland path;
// these results never claim native-Wayland input or a full installed session.
import { spawn, spawnSync } from 'node:child_process';
import { existsSync, mkdirSync, mkdtempSync, readdirSync, readFileSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { isAbsolute, join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

const pause = (milliseconds) => new Promise((done) => setTimeout(done, milliseconds));
const x11Displays = () => new Set([
  ...readdirSync('/tmp/.X11-unix'),
  ...readFileSync('/proc/net/unix', 'utf8').split('\n').flatMap((line) => {
    const match = /\/tmp\/\.X11-unix\/(X\d+)$/.exec(line);
    return match ? [match[1]] : [];
  }),
]);
const inner = process.argv[2] === '--session';
const arguments_ = process.argv.slice(inner ? 3 : 2);
const [compositor, binary, requestedReport, suppliedProfile] = arguments_;
if (process.platform !== 'linux' || !['gnome', 'kde'].includes(compositor) || !isAbsolute(binary ?? '')) {
  throw new Error('Pass gnome|kde, the absolute keyboard_window_validation executable, and an optional report path');
}
const profile = suppliedProfile ?? mkdtempSync(join(tmpdir(), `nomifun-keyboard-${compositor}-`));
const reportPath = resolve(requestedReport ?? join(profile, 'result.json'));
const environment = { ...process.env, HOME: join(profile, 'home'), XDG_RUNTIME_DIR: join(profile, 'runtime'),
  XDG_CONFIG_HOME: join(profile, 'config'), XDG_DATA_HOME: join(profile, 'data'), XDG_CACHE_HOME: join(profile, 'cache'),
  NOMIFUN_DATA_DIR: join(profile, 'data'), LIBGL_ALWAYS_SOFTWARE: '1', QT_QUICK_BACKEND: 'software',
  PULSE_SERVER: `unix:${join(profile, 'no-audio-server')}` };
for (const directory of ['home', 'runtime', 'config', 'data', 'cache']) mkdirSync(join(profile, directory), { recursive: true, mode: 0o700 });
if (!inner) {
  delete environment.DBUS_SESSION_BUS_ADDRESS;
  delete environment.DISPLAY; delete environment.WAYLAND_DISPLAY; delete environment.XAUTHORITY;
  const child = spawn('dbus-run-session', ['--', process.execPath, fileURLToPath(import.meta.url), '--session', compositor, binary, reportPath, profile],
    { env: environment, detached: true, stdio: ['ignore', 'inherit', 'inherit'] });
  process.exitCode = await new Promise((done) => { child.once('exit', (code) => done(code ?? 1)); child.once('error', () => done(1)); });
  // Reap only this validation session's process group, including any private
  // D-Bus-activated desktop helpers which survived the compositor's exit.
  try { process.kill(-child.pid, 'SIGTERM'); } catch (error) { if (error.code !== 'ESRCH') throw error; }
} else {
  const children = [];
  const start = (program, args, env = environment) => {
    const child = spawn(program, args, { env, stdio: ['ignore', 'ignore', 'pipe'] });
    const value = { child, error: '' };
    child.stderr.on('data', (bytes) => { if (value.error.length < 65536) value.error += bytes.toString(); });
    value.exited = new Promise((done) => { child.once('exit', (code) => done(code)); child.once('error', (error) => { value.error += error.message; done(1); }); });
    children.push(value); return value;
  };
  const wait = async (child, milliseconds) => {
    let timer;
    try { return await Promise.race([child.exited, new Promise((_, fail) => { timer = setTimeout(() => fail(new Error('Nested compositor witness timed out')), milliseconds); })]); }
    finally { clearTimeout(timer); }
  };
  let report; let shell;
  try {
    rmSync(reportPath, { force: true });
    const initialSockets = x11Displays();
    const number = Array.from({ length: 20 }, (_, offset) => 81 + offset).find((value) => !initialSockets.has(`X${value}`) && !existsSync(`/tmp/.X${value}-lock`));
    if (!number) throw new Error('No private parent X11 display available');
    environment.DISPLAY = `:${number}`;
    delete environment.WAYLAND_DISPLAY;
    delete environment.XAUTHORITY;
    start('Xvfb', [environment.DISPLAY, '-screen', '0', '1280x900x24', '-nolisten', 'tcp', '-ac', '-noreset']);
    let deadline = Date.now() + 5000;
    while (!x11Displays().has(`X${number}`)) { if (Date.now() >= deadline) throw new Error('Private parent X11 server did not start'); await pause(40); }
    const wayland = `nomifun-${compositor}-wayland`;
    const desktop = compositor === 'gnome' ? 'GNOME' : 'KDE';
    const shellEnvironment = { ...environment, XDG_SESSION_TYPE: 'wayland', XDG_CURRENT_DESKTOP: desktop, XDG_SESSION_DESKTOP: desktop.toLowerCase() };
    shell = compositor === 'gnome'
      ? start('gnome-shell', ['--nested', '--wayland', `--wayland-display=${wayland}`], shellEnvironment)
      : start('kwin_wayland', [`--x11-display=${environment.DISPLAY}`, `--socket=${wayland}`, '--xwayland', '--no-lockscreen', '--width=1280', '--height=900'], { ...shellEnvironment, KWIN_COMPOSE: 'Q' });
    deadline = Date.now() + 25000;
    let ready = false;
    while (Date.now() < deadline && !ready) {
      if (shell.child.exitCode !== null) throw new Error(`Nested ${desktop} compositor exited: ${shell.error}`);
      // Mutter reserves its X sockets before starting Xwayland on demand.
      // An initial read wakes it; the next iteration obtains its auth file.
      const processes = spawnSync('ps', ['-eo', 'pid=,ppid=,args='], { encoding: 'utf8' }).stdout;
      const descendants = new Set([shell.child.pid]);
      const rows = processes.split('\n').map((line) => /^\s*(\d+)\s+(\d+)\s+(.*)$/.exec(line)).filter(Boolean);
      for (let pass = 0; pass < rows.length; pass++) for (const row of rows) if (descendants.has(Number(row[2]))) descendants.add(Number(row[1]));
      const xwayland = rows.find((row) => descendants.has(Number(row[1])) && /(?:^|\/)Xwayland\s/.test(row[3]));
      const auth = xwayland?.[3].match(/(?:^|\s)-auth\s+(\S+)/)?.[1];
      const displays = [...x11Displays()].filter((name) => /^X\d+$/.test(name) && name !== `X${number}` && !initialSockets.has(name));
      for (const socket of displays) {
        const candidate = { ...environment, DISPLAY: `:${socket.slice(1)}`, WAYLAND_DISPLAY: wayland, GDK_BACKEND: 'x11' };
        if (auth) candidate.XAUTHORITY = auth;
        const query = spawnSync('xprop', ['-root', '_NET_SUPPORTING_WM_CHECK'], { env: candidate, encoding: 'utf8', timeout: 1000 });
        if (query.status === 0 && query.stdout.includes('window id #')) { Object.assign(environment, candidate); ready = true; break; }
      }
      if (!ready) await pause(100);
    }
    if (!ready) throw new Error(`Nested ${desktop} Xwayland did not become ready: ${shell.error}`);
    const parentEnvironment = { ...environment, DISPLAY: `:${number}` };
    delete parentEnvironment.XAUTHORITY;
    const host = spawnSync('timeout', ['4s', 'xdotool', 'search', '--sync', '--onlyvisible', '--pid', String(shell.child.pid)], { env: parentEnvironment, encoding: 'utf8', timeout: 5000 });
    const hostWindow = host.stdout.trim().split('\n')[0];
    if (host.status !== 0 || !/^\d+$/.test(hostWindow)) throw new Error('Nested compositor parent input window was not found');
    const focused = spawnSync('timeout', ['4s', 'xdotool', 'windowfocus', '--sync', hostWindow, 'key', '--clearmodifiers', 'Escape'], { env: parentEnvironment, timeout: 5000 });
    if (focused.status !== 0) throw new Error('Private parent input focus could not be initialized');
    await pause(600);
    spawnSync('dbus-update-activation-environment', ['DISPLAY', 'XAUTHORITY', 'WAYLAND_DISPLAY', 'XDG_RUNTIME_DIR'], { env: environment, stdio: 'ignore', timeout: 5000 });
    Object.assign(environment, { NOMIFUN_KEYBOARD_VALIDATION_REPORT: reportPath, NOMIFUN_KEYBOARD_VALIDATION_ALLOW_CLIPBOARD: '1',
      NOMIFUN_KEYBOARD_VALIDATION_INPUT_DISPLAY: `:${number}`, NOMIFUN_KEYBOARD_VALIDATION_INPUT_WINDOW: hostWindow,
      NOMIFUN_KEYBOARD_VALIDATION_ENVIRONMENT: `Isolated ${desktop} nested Wayland compositor; GTK/Tauri XWayland path` });
    const sentinel = Buffer.from('NomiFun private Wayland compositor clipboard sentinel\n中文 👋\n');
    const seeded = spawnSync('timeout', ['4s', 'xclip', '-selection', 'clipboard', '-in'], { env: environment, input: sentinel, stdio: ['pipe', 'ignore', 'ignore'], timeout: 5000 });
    if (seeded.status !== 0) throw new Error('Private compositor clipboard fixture did not initialize');
    const witness = start(binary, []);
    await wait(witness, 40000);
    if (!existsSync(reportPath)) throw new Error(`Native witness did not report: ${witness.error}`);
    report = JSON.parse(readFileSync(reportPath, 'utf8'));
    const restored = spawnSync('timeout', ['4s', 'xclip', '-selection', 'clipboard', '-out'], { env: environment, timeout: 5000 });
    report.clipboardRestoredAfterWitnessExit = restored.status === 0 && restored.stdout.equals(sentinel);
    report.ok = report.ok === true && report.clipboardRestoredAfterWitnessExit && report.clipboard === 'passed_webkit_default_ctrl_c_selection_after_hide';
    report.applicationBackend = 'GTK X11 on compositor-managed XWayland';
    report.clipboardTransportEvidence = !report.clipboard?.startsWith('passed_') ? 'Clipboard stage did not complete'
      : 'WebKit/GDK X11 selection verified in the compositor session';
  } catch (error) {
    report = { ok: false, error: error instanceof Error ? error.message : String(error) };
  } finally {
    for (const { child } of children.reverse()) if (child.exitCode === null && child.signalCode === null) child.kill('SIGTERM');
    await Promise.all(children.map((child) => wait(child, 5000).catch(() => { child.child.kill('SIGKILL'); })));
    report.compositor = compositor === 'gnome' ? 'GNOME Shell / Mutter' : 'KDE KWin';
    report.profileIsolated = true; report.sessionBusIsolated = true; report.userDisplayUntouched = true;
    report.packages = Object.fromEntries((compositor === 'gnome' ? ['gnome-shell', 'mutter-common', 'xwayland'] : ['kwin-wayland', 'xwayland']).map((name) =>
      [name, spawnSync('dpkg-query', ['-W', '-f=${Version}', name], { encoding: 'utf8' }).stdout.trim()]));
    mkdirSync(resolve(reportPath, '..'), { recursive: true });
    writeFileSync(reportPath, JSON.stringify(report, null, 2) + '\n');
    console.log(JSON.stringify({ ok: report.ok, report: reportPath }));
    process.exitCode = report.ok ? 0 : 1;
  }
}
