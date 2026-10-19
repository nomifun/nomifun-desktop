#!/usr/bin/env node
// Real executable witness. The private profile is generated for this run;
// no user model catalog, credentials, runtime descriptor or login item is used.
import { spawn } from 'node:child_process';
import { closeSync, existsSync, mkdirSync, mkdtempSync, openSync, readFileSync, readSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { isAbsolute, join, resolve } from 'node:path';

if (!['linux', 'win32'].includes(process.platform)) throw new Error('This witness targets Windows/Linux native executables');
const binary = resolve(process.argv[2] ?? '');
if (!process.argv[2] || !isAbsolute(process.argv[2]) || !existsSync(binary)) throw new Error('Pass the absolute built desktop executable path');
const profile = mkdtempSync(join(tmpdir(), 'nomifun-keyboard-service-smoke-'));
const runtime = join(profile, 'runtime'); const configuration = join(profile, 'config'); const data = join(profile, 'data');
for (const directory of [runtime, configuration, data]) mkdirSync(directory, { mode: 0o700 });
const environment = { ...process.env, HOME: profile, XDG_RUNTIME_DIR: runtime, XDG_CONFIG_HOME: configuration, NOMIFUN_DATA_DIR: data,
  ...(process.platform === 'win32' ? { LOCALAPPDATA: join(profile, 'local'), APPDATA: join(profile, 'roaming') } : {}) };
for (const key of ['DISPLAY', 'WAYLAND_DISPLAY', 'DBUS_SESSION_BUS_ADDRESS', 'NOMI_API_KEY', 'NOMIFUN_API_KEY']) delete environment[key];
const descriptor = join(process.platform === 'win32' ? environment.LOCALAPPDATA : runtime, 'NomiFun/keyboard-service/owner.json');
const reportPath = process.argv[3] ? resolve(process.argv[3]) : join(profile, 'result.json');
const pause = (milliseconds) => new Promise((done) => setTimeout(done, milliseconds));
const children = [];
const start = (argument) => {
  const processHandle = spawn(binary, [argument], { env: environment, stdio: ['ignore', 'ignore', 'pipe'], windowsHide: true });
  let error = '';
  processHandle.stderr.on('data', (bytes) => { if (error.length < 8192) error += bytes.toString(); });
  const exited = new Promise((done) => { processHandle.once('error', (failure) => done({ code: null, error: failure.message })); processHandle.once('exit', (code) => done({ code, error })); });
  const child = { process: processHandle, exited }; children.push(child); return child;
};
const wait = async (child, milliseconds) => {
  let timer;
  try { return await Promise.race([child.exited, new Promise((_, fail) => { timer = setTimeout(() => fail(new Error('Native executable did not exit within the deadline')), milliseconds); })]); }
  finally { clearTimeout(timer); }
};
const requireSuccess = (result, stage) => { if (result.code !== 0) throw new Error(`${stage} failed: ${result.error || result.code}`); };
const containsBuildId = (id) => {
  const file = openSync(binary, 'r'); const needle = Buffer.from(id); const buffer = Buffer.alloc(1024 * 1024); let tail = Buffer.alloc(0);
  try {
    for (;;) { const count = readSync(file, buffer); if (!count) return false; const chunk = Buffer.concat([tail, buffer.subarray(0, count)]); if (chunk.includes(needle)) return true; tail = chunk.subarray(Math.max(0, chunk.length - needle.length)); }
  } finally { closeSync(file); }
};

const report = { schema: 1, platform: process.platform === 'win32' ? 'Windows normal executable in private LOCALAPPDATA' : 'Linux executable without DISPLAY or WAYLAND_DISPLAY', binary,
  profileIsolated: true, frontendBuildIdCurrent: null, singleOwner: false, authenticatedShutdown: false,
  ownerExited: false, descriptorRemoved: false, loginUnregistered: false, ok: false };
let owner;
try {
  const manifest = join(process.cwd(), 'ui/dist/nomifun-build.json');
  if (existsSync(manifest)) report.frontendBuildIdCurrent = containsBuildId(JSON.parse(readFileSync(manifest, 'utf8')).frontend_build_id);
  owner = start('--keyboard-service');
  const deadline = Date.now() + 6000;
  while (!existsSync(descriptor)) {
    if (owner.process.exitCode !== null) throw new Error('Service exited before publishing an owner');
    if (Date.now() >= deadline) throw new Error('Service did not publish an owner within the deadline');
    await pause(30);
  }
  // Only compare the PID. Never print the private IPC token or descriptor.
  const first = JSON.parse(readFileSync(descriptor, 'utf8')).pid;
  const duplicate = start('--keyboard-service'); requireSuccess(await wait(duplicate, 4000), 'Duplicate owner exclusion');
  if (JSON.parse(readFileSync(descriptor, 'utf8')).pid !== first) throw new Error('Duplicate owner replaced the descriptor');
  report.singleOwner = true;
  const shutdown = start('--keyboard-stop'); requireSuccess(await wait(shutdown, 18000), 'Authenticated shutdown');
  report.authenticatedShutdown = true;
  requireSuccess(await wait(owner, 4000), 'Serial owner exit'); report.ownerExited = true;
  report.descriptorRemoved = !existsSync(descriptor);
  if (!report.descriptorRemoved) throw new Error('Stopped owner did not remove its descriptor');
  if (process.platform === 'linux') {
    const autostart = join(configuration, 'autostart/nomifun-keyboard.desktop');
    mkdirSync(join(configuration, 'autostart'));
    writeFileSync(autostart, `[Desktop Entry]\nType=Application\nName=NomiFun keyboard smoke fixture\nExec="${binary}" --keyboard-service\n`);
    requireSuccess(await wait(start('--keyboard-unregister'), 8000), 'Native login unregister');
    report.loginUnregistered = !existsSync(autostart);
    if (!report.loginUnregistered) throw new Error('Native unregister left its login item');
  } else {
    // HKCU is not redirected by LOCALAPPDATA. Do not delete the real user's
    // existing Run value merely to exercise an isolated native executable.
    report.loginUnregistered = null;
    report.loginCheck = 'not run: real HKCU unchanged; installer hook tested separately';
  }
  report.ok = true;
} catch (error) {
  report.error = error instanceof Error ? error.message : String(error);
} finally {
  for (const child of children) {
    if (child.process.exitCode === null && child.process.signalCode === null) child.process.kill('SIGTERM');
  }
  mkdirSync(resolve(reportPath, '..'), { recursive: true });
  writeFileSync(reportPath, JSON.stringify(report, null, 2) + '\n');
  console.log(JSON.stringify({ ok: report.ok, report: reportPath, frontendBuildIdCurrent: report.frontendBuildIdCurrent }));
}
process.exitCode = report.ok ? 0 : 1;
