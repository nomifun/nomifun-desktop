// The direct child is a public LaunchServices controller, never the App.
// Only its authenticated App cleanup receipt + kernel exit status can succeed.
import { spawn } from 'node:child_process';
import { randomBytes } from 'node:crypto';
import { chmod, mkdtemp, realpath, stat, unlink, writeFile } from 'node:fs/promises';
import { dirname, isAbsolute, join, resolve } from 'node:path';
import { constants as osConstants } from 'node:os';

const MAX_FRAME_BYTES = 1024 * 1024;
export const MACOS_DEVELOPMENT_DESCRIPTOR_ARGUMENT = '--nomifun-development-launch-descriptor';

// Only the adapter may produce this before it calls spawnController. Other
// launch errors do not prove that no native application was started.
export class MacosApplicationLaunchCancelled extends Error {
  constructor(evidenceDirectory) {
    super('macOS application launch was cancelled before controller start');
    this.name = 'MacosApplicationLaunchCancelled';
    this.evidenceDirectory = evidenceDirectory;
  }
}

export function validateApplicationLaunch({ program, controllerProgram, args, environment, cwd, applicationScope = 'desktop_development' }) {
  if (!isAbsolute(program ?? '') || !isAbsolute(controllerProgram ?? '') || !isAbsolute(cwd ?? '')
    || !program.endsWith(join('.app', 'Contents', 'MacOS', 'nomifun-desktop'))
    || !Array.isArray(args) || !args.every(value => typeof value === 'string' && !value.includes('\0') && value !== MACOS_DEVELOPMENT_DESCRIPTOR_ARGUMENT)
    || !environment || typeof environment !== 'object' || Object.entries(environment).some(([key, value]) => !key || key.includes('=') || key.includes('\0') || typeof value !== 'string' || value.includes('\0'))
    || !isAbsolute(environment.NOMIFUN_DATA_DIR ?? '')
    || !['desktop_development', 'browser_media_audit', 'browser_compatibility_audit', 'appkit_main_loop_audit'].includes(applicationScope)) {
    throw new Error('macOS application launch requires an exact App, compiled controller, cwd and isolated environment');
  }
  if (applicationScope === 'browser_media_audit' && !args.includes('--media-permissions')) {
    throw new Error('macOS media audit launch requires its fixed permission fixture entry');
  }
  if (applicationScope === 'browser_compatibility_audit' && args.some(arg => ['--media-permissions', '--media-user-ready'].includes(arg))) {
    throw new Error('macOS compatibility audit cannot enter the media audit');
  }
  if (applicationScope === 'appkit_main_loop_audit' && !args.some((arg, index) => arg === '--report' && isAbsolute(args[index + 1] ?? ''))) {
    throw new Error('macOS AppKit audit requires its fixed absolute report entry');
  }
  return { program: resolve(program), controllerProgram: resolve(controllerProgram), cwd: resolve(cwd), args, environment, applicationScope };
}

export async function launchMacosApplication(request, { spawnController = spawn, isLaunchAllowed = () => true } = {}) {
  const launch = validateApplicationLaunch(request);
  if (isLaunchAllowed() !== true) throw new MacosApplicationLaunchCancelled();
  // Receipt identity and the application's actual chdir use the same physical
  // directory. A legitimate symlink cwd must not become a protocol mismatch.
  launch.cwd = await realpath(launch.cwd);
  if (!(await stat(launch.cwd)).isDirectory()) throw new Error('macOS application working directory is not a directory');
  const directory = await mkdtemp('/tmp/nomifun-app-launch-');
  await chmod(directory, 0o700);
  const descriptorPath = join(directory, 'launch.json');
  const stdoutPath = join(directory, 'application.stdout.log');
  const stderrPath = join(directory, 'application.stderr.log');
  for (const path of [stdoutPath, stderrPath]) await writeFile(path, '', { flag: 'wx', mode: 0o600 });
  const descriptor = {
    protocol_version: 1, nonce: randomBytes(32).toString('hex'), application_scope: launch.applicationScope,
    app_path: dirname(dirname(dirname(launch.program))), program: launch.program, cwd: launch.cwd,
    args: launch.args, environment: launch.environment, socket_path: join(directory, 'application.sock'),
    stdout_path: stdoutPath, stderr_path: stderrPath,
  };
  await writeFile(descriptorPath, `${JSON.stringify(descriptor)}\n`, { flag: 'wx', mode: 0o600 });
  const finishDescriptor = () => unlink(descriptorPath).catch(error => { if (error.code !== 'ENOENT') throw error; });
  if (isLaunchAllowed() !== true) {
    await finishDescriptor();
    throw new MacosApplicationLaunchCancelled(directory);
  }
  // The argv contains only a private descriptor path. Neither its nonce nor
  // environment values become command-line diagnostics or process arguments.
  // No await may separate the final cancellation check from this submission.
  const child = spawnController(launch.controllerProgram, [descriptorPath], {
    cwd: launch.cwd, env: Object.fromEntries(['PATH', 'HOME', 'TMPDIR', 'LANG', 'DEVELOPER_DIR']
      .filter(key => process.env[key] !== undefined).map(key => [key, process.env[key]])),
    stdio: ['ignore', 'pipe', 'inherit'], detached: true,
  });
  let buffered = '';
  let ready;
  let terminal;
  let protocolError;
  let discardedBytes = 0;
  const exit = new Promise((accept, reject) => {
    const fail = message => { protocolError ??= new Error(message); buffered = ''; };
    child.stdout.setEncoding('utf8');
    child.stdout.on('data', data => {
      if (protocolError) { discardedBytes += Buffer.byteLength(data); return; }
      buffered += data;
      if (Buffer.byteLength(buffered) > MAX_FRAME_BYTES) { fail('macOS application controller response exceeded its bound'); return; }
      let end;
      while ((end = buffered.indexOf('\n')) >= 0) {
        const line = buffered.slice(0, end); buffered = buffered.slice(end + 1);
        let frame;
        try { frame = JSON.parse(line); } catch { fail('macOS application controller response was malformed'); return; }
        if (!frame || typeof frame !== 'object' || Array.isArray(frame)) {
          fail('macOS application controller response must be an object'); return;
        }
        if (frame.event === 'ready' && !ready && !terminal
          && Number.isInteger(frame.pid) && frame.pid > 1 && frame.cwd === launch.cwd && frame.data_dir === launch.environment.NOMIFUN_DATA_DIR) {
          ready = frame;
          finishDescriptor().catch(() => fail('macOS private launch descriptor could not be removed'));
        } else if (frame.event === 'exit' && ready && !terminal && frame.pid === ready.pid
          && (frame.code === null || Number.isInteger(frame.code) && frame.code >= 0 && frame.code <= 255)
          && (frame.signal === null || Number.isInteger(frame.signal) && frame.signal > 0)
          && typeof frame.cleanup_verified === 'boolean'
          && typeof frame.restart_requested === 'boolean') {
          if (launch.applicationScope !== 'desktop_development' && frame.restart_requested) {
            fail('macOS media audit cannot request an application restart; all audit scopes reject restart'); return;
          }
          terminal = frame;
        } else { fail('macOS application controller response was out of sequence or did not match admission'); return; }
      }
    });
    child.once('error', async () => {
      await finishDescriptor().catch(() => {});
      reject(new Error('macOS application controller could not start; application cleanup is unconfirmed'));
    });
    child.once('close', async (code, signal) => {
      await finishDescriptor().catch(() => fail('macOS private launch descriptor could not be removed'));
      if (protocolError) { reject(protocolError); return; }
      if (buffered.trim() || !ready || !terminal || signal || code !== 0
        || terminal.code !== 0 || terminal.signal !== null || !terminal.cleanup_verified) {
        reject(new Error('macOS application exited without authenticated cleanup and exact normal exit; new launches must remain blocked'));
        return;
      }
      const signalName = terminal.signal === null ? null : Object.entries(osConstants.signals).find(([, value]) => value === terminal.signal)?.[0] ?? 'UNKNOWN';
      accept({ code: terminal.code, signal: signalName, pid: terminal.pid, cleanupVerified: true,
        restartRequested: terminal.restart_requested, evidenceDirectory: directory });
    });
  });
  exit.catch(() => {});
  return { exit, evidenceDirectory: directory, stdoutPath, stderrPath,
    diagnostics: () => ({ bufferedBytes: Buffer.byteLength(buffered), discardedBytes }) };
}
