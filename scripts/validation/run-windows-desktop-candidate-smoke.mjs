#!/usr/bin/env bun

/**
 * Native Windows x64 package smoke for a frozen NomiFun Desktop candidate.
 *
 * Operational usage:
 *   bun scripts/validation/run-windows-desktop-candidate-smoke.mjs \
 *     --installer <NSIS-exe> \
 *     --source-commit <40hex> \
 *     --work-root <repo-build.noindex-dir>
 *
 * Non-destructive harness validation:
 *   bun scripts/validation/run-windows-desktop-candidate-smoke.mjs --self-test
 */

import { spawn, spawnSync } from 'node:child_process';
import { createHash, randomBytes } from 'node:crypto';
import {
  closeSync,
  createReadStream,
  existsSync,
  lstatSync,
  mkdirSync,
  openSync,
  readFileSync,
  readSync,
  realpathSync,
  statSync,
} from 'node:fs';
import { createServer } from 'node:net';
import {
  dirname,
  extname,
  isAbsolute,
  join,
  relative,
  resolve,
  sep,
} from 'node:path';
import { fileURLToPath } from 'node:url';

export const REPO_ROOT = resolve(dirname(fileURLToPath(import.meta.url)), '../..');
export const TARGET_ID = 'windows_desktop_x64';
export const TARGET_TRIPLE = 'x86_64-pc-windows-msvc';
export const MAIN_BINARY_NAME = 'nomifun-desktop.exe';
export const UNINSTALLER_NAME = 'uninstall.exe';

export const CHECK_IDS = Object.freeze([
  'native-host',
  'source-checkpoint',
  'work-root',
  'installer-artifact',
  'installation-preflight',
  'install',
  'installed-binary',
  'launch',
  'port-announcement',
  'backend-health',
  'webview2-cdp',
  'process-tree-cleanup',
  'uninstall',
  'uninstall-verification',
]);

export const CHECK_TIMEOUTS_MS = Object.freeze({
  'native-host': 1_000,
  'source-checkpoint': 10_000,
  'work-root': 5_000,
  'installer-artifact': 60_000,
  'installation-preflight': 10_000,
  install: 300_000,
  'installed-binary': 60_000,
  launch: 15_000,
  'port-announcement': 120_000,
  'backend-health': 30_000,
  'webview2-cdp': 60_000,
  'process-tree-cleanup': 20_000,
  uninstall: 180_000,
  'uninstall-verification': 30_000,
});

const SOURCE_COMMIT_PATTERN = /^[0-9a-f]{40}$/i;
const SECRET_ENVIRONMENT_NAME =
  /(?:api[_-]?key|access[_-]?key|private[_-]?key|token|secret|password|passwd|credential|authorization)/i;
const POLL_INTERVAL_MS = 250;
const COMMAND_CLEANUP_GRACE_MS = 5_000;
const PE_MACHINE_AMD64 = 0x8664;
const INSTALL_REGISTRY_KEYS = Object.freeze([
  'HKCU\\Software\\Microsoft\\Windows\\CurrentVersion\\Uninstall\\NomiFun',
  'HKCU\\Software\\nomifun\\NomiFun',
  'HKCU\\Software\\Classes\\nomifun',
]);

export class SmokeFailure extends Error {
  constructor(code, message, details = {}) {
    super(message);
    this.name = 'SmokeFailure';
    this.code = code;
    this.details = details;
  }
}

function fail(code, message, details = {}) {
  throw new SmokeFailure(code, message, details);
}

function sleep(milliseconds) {
  return new Promise((resolvePromise) => setTimeout(resolvePromise, milliseconds));
}

function normalizedPath(path) {
  return String(path).replaceAll('\\', '/');
}

function reportPath(path) {
  const absolute = resolve(path);
  const repoRelative = relative(REPO_ROOT, absolute);
  return isPathWithin(REPO_ROOT, absolute)
    ? normalizedPath(repoRelative || '.')
    : normalizedPath(absolute);
}

function compactRunId(now = Date.now(), pid = process.pid) {
  return `${now.toString(36)}-${pid.toString(36)}`;
}

export function parseArgs(argv) {
  if (argv.length === 1 && argv[0] === '--self-test') {
    return { selfTest: true };
  }
  if (argv.includes('--self-test')) {
    throw new Error('--self-test cannot be combined with operational arguments');
  }

  const values = new Map();
  const allowed = new Set(['--installer', '--source-commit', '--work-root']);
  for (let index = 0; index < argv.length; index += 1) {
    const flag = argv[index];
    if (!allowed.has(flag)) {
      throw new Error(`unknown argument: ${flag}`);
    }
    if (values.has(flag)) {
      throw new Error(`duplicate argument: ${flag}`);
    }
    const value = argv[index + 1];
    if (typeof value !== 'string' || value.length === 0 || value.startsWith('--')) {
      throw new Error(`${flag} requires a value`);
    }
    values.set(flag, value);
    index += 1;
  }

  for (const flag of allowed) {
    if (!values.has(flag)) throw new Error(`missing required argument: ${flag}`);
  }
  const sourceCommit = values.get('--source-commit');
  if (!SOURCE_COMMIT_PATTERN.test(sourceCommit)) {
    throw new Error('--source-commit must be exactly 40 hexadecimal characters');
  }

  return {
    selfTest: false,
    installer: values.get('--installer'),
    sourceCommit: sourceCommit.toLowerCase(),
    workRoot: values.get('--work-root'),
  };
}

export function isPathWithin(parent, candidate) {
  const base = resolve(parent);
  const target = resolve(candidate);
  const pathFromBase = relative(base, target);
  return (
    pathFromBase === '' ||
    (!pathFromBase.startsWith(`..${sep}`) &&
      pathFromBase !== '..' &&
      !isAbsolute(pathFromBase))
  );
}

export function validateWorkRootPath(repoRoot, workRoot) {
  const repository = resolve(repoRoot);
  const expectedRoot = resolve(repository, 'build.noindex');
  const observed = resolve(workRoot);
  const status = isPathWithin(expectedRoot, observed) ? 'pass' : 'fail';
  return {
    status,
    repository,
    expected_root: expectedRoot,
    observed,
    ...(status === 'fail'
      ? { reason: 'work_root_must_be_within_repository_build_noindex' }
      : {}),
  };
}

export function buildNsisInstallArgs(installDirectory) {
  const absolute = resolve(installDirectory);
  if (!isAbsolute(absolute)) {
    throw new Error('NSIS install directory must be absolute');
  }
  if (/[\r\n"]/.test(absolute)) {
    throw new Error('NSIS install directory contains an unsupported character');
  }
  return ['/S', '/NS', `/D=${absolute}`];
}

export function evaluateRegistryProbe(results) {
  const existing = [];
  const probeErrors = [];
  for (const result of results || []) {
    if (result?.status === 0) existing.push(result.key);
    else if (result?.status !== 1) probeErrors.push(result?.key || 'unknown');
  }
  return {
    status:
      existing.length === 0 && probeErrors.length === 0 ? 'pass' : 'fail',
    existing,
    probe_errors: probeErrors,
  };
}

export function evaluateSourceCheckpoint({ expected, head, statusOutput, headStatus, statusStatus }) {
  const observed = typeof head === 'string' ? head.trim().toLowerCase() : null;
  const worktree = typeof statusOutput === 'string' ? statusOutput.trim() : '';
  const errors = [];
  if (headStatus !== 0) errors.push('head_probe_failed');
  if (statusStatus !== 0) errors.push('worktree_probe_failed');
  if (observed !== String(expected || '').toLowerCase()) errors.push('head_mismatch');
  if (worktree.length > 0) errors.push('dirty_worktree');
  return {
    status: errors.length === 0 ? 'pass' : 'fail',
    expected: String(expected || '').toLowerCase() || null,
    observed,
    clean: worktree.length === 0,
    errors,
  };
}

export function environmentWithoutSecrets(source = process.env) {
  const environment = {};
  for (const [name, value] of Object.entries(source)) {
    if (SECRET_ENVIRONMENT_NAME.test(name)) continue;
    if (typeof value === 'string') environment[name] = value;
  }
  return environment;
}

export function evaluateRegularExeArtifact({
  resolvedPath,
  realPath,
  allowedRoots,
  isFile,
  isSymbolicLink,
  sizeBytes,
}) {
  const errors = [];
  const resolvedFile = resolve(resolvedPath);
  const realFile = resolve(realPath);
  const roots = (allowedRoots || []).map((root) => resolve(root));
  if (extname(resolvedFile).toLowerCase() !== '.exe') errors.push('extension_not_exe');
  if (!isFile) errors.push('not_regular_file');
  if (isSymbolicLink) errors.push('symlink_not_allowed');
  if (!Number.isSafeInteger(sizeBytes) || sizeBytes <= 0) errors.push('empty_or_invalid_size');
  if (!roots.some((root) => isPathWithin(root, realFile))) {
    errors.push('artifact_outside_allowed_roots');
  }
  return {
    status: errors.length === 0 ? 'pass' : 'fail',
    resolved_path: resolvedFile,
    real_path: realFile,
    size_bytes: sizeBytes,
    errors,
  };
}

function isLoopbackHost(host) {
  const value = String(host || '').trim().toLowerCase();
  return value === '127.0.0.1' || value === 'localhost' || value === '::1';
}

export function validatePortAnnouncement(value, expectedPid = null) {
  const errors = [];
  if (!value || typeof value !== 'object' || Array.isArray(value)) {
    return { status: 'fail', errors: ['announcement_not_object'], value: null };
  }
  if (!isLoopbackHost(value.host)) errors.push('host_not_loopback');
  if (!Number.isInteger(value.port) || value.port < 1 || value.port > 65_535) {
    errors.push('port_invalid');
  }
  if (typeof value.channel !== 'string' || value.channel.trim().length === 0) {
    errors.push('channel_invalid');
  }
  if (!Number.isInteger(value.pid) || value.pid <= 0) errors.push('pid_invalid');
  if (expectedPid !== null && value.pid !== expectedPid) errors.push('pid_mismatch');
  return {
    status: errors.length === 0 ? 'pass' : 'fail',
    errors,
    value: {
      host: typeof value.host === 'string' ? value.host : null,
      port: Number.isInteger(value.port) ? value.port : null,
      channel: typeof value.channel === 'string' ? value.channel : null,
      pid: Number.isInteger(value.pid) ? value.pid : null,
    },
  };
}

function loopbackBaseUrl(host, port) {
  const address = host === '::1' ? '[::1]' : host;
  return `http://${address}:${port}`;
}

export function evaluateHealthResponse(statusCode, body) {
  let parsed = null;
  try {
    parsed = JSON.parse(body);
  } catch {
    // The status and body shape below fail closed.
  }
  const healthy = statusCode === 200 && parsed?.status === 'ok';
  return {
    status: healthy ? 'pass' : 'fail',
    status_code: statusCode,
    response_status: typeof parsed?.status === 'string' ? parsed.status : null,
    body_size_bytes: Buffer.byteLength(body),
    body_sha256: createHash('sha256').update(body).digest('hex'),
  };
}

function isNomiFunDocumentUrl(value) {
  try {
    const url = new URL(value);
    const protocol = url.protocol.toLowerCase();
    const hostname = url.hostname.toLowerCase();
    return (
      ((protocol === 'http:' || protocol === 'https:') && hostname === 'tauri.localhost') ||
      (protocol === 'tauri:' && (hostname === 'localhost' || hostname === 'tauri.localhost'))
    );
  } catch {
    return false;
  }
}

export function findNomiFunCdpTarget(targets) {
  if (!Array.isArray(targets)) return null;
  for (const target of targets) {
    if (!target || typeof target !== 'object' || Array.isArray(target)) continue;
    const title = typeof target.title === 'string' ? target.title.trim() : '';
    const url = typeof target.url === 'string' ? target.url : '';
    if (!/^NomiFun(?:\b|$)/i.test(title) || !isNomiFunDocumentUrl(url)) continue;
    return {
      id: typeof target.id === 'string' ? target.id : null,
      type: typeof target.type === 'string' ? target.type : null,
      title,
      url,
    };
  }
  return null;
}

export function parsePeMachine(bytes) {
  const buffer = Buffer.isBuffer(bytes) ? bytes : Buffer.from(bytes);
  if (buffer.length < 64 || buffer[0] !== 0x4d || buffer[1] !== 0x5a) {
    return { status: 'fail', reason: 'invalid_dos_header', machine: null };
  }
  const peOffset = buffer.readUInt32LE(0x3c);
  if (peOffset < 64 || peOffset + 6 > buffer.length) {
    return { status: 'fail', reason: 'invalid_pe_offset', machine: null };
  }
  if (
    buffer[peOffset] !== 0x50 ||
    buffer[peOffset + 1] !== 0x45 ||
    buffer[peOffset + 2] !== 0 ||
    buffer[peOffset + 3] !== 0
  ) {
    return { status: 'fail', reason: 'invalid_pe_signature', machine: null };
  }
  const machine = buffer.readUInt16LE(peOffset + 4);
  return {
    status: machine === PE_MACHINE_AMD64 ? 'pass' : 'fail',
    reason: machine === PE_MACHINE_AMD64 ? null : 'machine_not_amd64',
    machine,
    machine_hex: `0x${machine.toString(16).padStart(4, '0')}`,
  };
}

export function createInitialResult(sourceCommit = null) {
  return {
    schema_version: '1.0.0',
    source_commit: sourceCommit,
    target: TARGET_ID,
    platform: TARGET_TRIPLE,
    status: 'fail',
    suite: {
      name: 'windows-desktop-candidate-install-smoke',
      checks: [...CHECK_IDS],
    },
    checks: [],
    logs: [],
    artifacts: {
      result_root: null,
      installer: null,
      host: null,
      uninstaller: null,
    },
    install: {
      root: null,
      silent: true,
      directory_override_last: true,
      installer_exit_code: null,
      application_pid: null,
      process_tree_cleanup: null,
      uninstaller_exit_code: null,
      main_binary_removed: false,
    },
    data: {
      root: null,
      port_file: null,
      announcement: null,
    },
    backend: {
      base_url: null,
      health_url: null,
      status_code: null,
      response_status: null,
      body_size_bytes: null,
      body_sha256: null,
    },
    cdp: {
      port: null,
      endpoint: null,
      target: null,
      observed_target_count: null,
    },
  };
}

function errorEvidence(error) {
  if (error instanceof SmokeFailure) {
    return {
      code: error.code,
      reason: error.message,
      ...error.details,
    };
  }
  return {
    code: 'unexpected_internal_error',
    reason: 'unexpected internal error; inspect the stage logs',
  };
}

function withTimeout(promise, timeoutMs, checkId) {
  let timer;
  const timeout = new Promise((_, reject) => {
    timer = setTimeout(
      () =>
        reject(
          new SmokeFailure(
            'stage_timeout',
            `${checkId} exceeded its bounded timeout`,
            { timed_out: true },
          ),
        ),
      timeoutMs + COMMAND_CLEANUP_GRACE_MS,
    );
  });
  return Promise.race([promise, timeout]).finally(() => clearTimeout(timer));
}

async function runCheck(report, id, operation, timeoutOverrideMs = null) {
  const timeoutMs = timeoutOverrideMs ?? CHECK_TIMEOUTS_MS[id];
  if (!Number.isSafeInteger(timeoutMs) || timeoutMs <= 0) {
    throw new Error(`check ${id} requires a positive bounded timeout`);
  }
  const startedAt = Date.now();
  try {
    const details = await withTimeout(
      Promise.resolve().then(operation),
      timeoutMs,
      id,
    );
    report.checks.push({
      id,
      status: 'pass',
      timeout_ms: timeoutMs,
      duration_ms: Date.now() - startedAt,
      ...(details || {}),
    });
    return true;
  } catch (error) {
    report.checks.push({
      id,
      status: 'fail',
      timeout_ms: timeoutMs,
      duration_ms: Date.now() - startedAt,
      ...errorEvidence(error),
    });
    return false;
  }
}

function markMissingChecksSkipped(report, plannedCheckIds = CHECK_IDS) {
  const observed = new Set(report.checks.map((entry) => entry.id));
  for (const id of plannedCheckIds) {
    if (observed.has(id)) continue;
    report.checks.push({
      id,
      status: 'skipped',
      timeout_ms: CHECK_TIMEOUTS_MS[id] ?? null,
      reason: 'prerequisite_failed',
    });
  }
  const order = new Map(plannedCheckIds.map((id, index) => [id, index]));
  report.checks.sort(
    (left, right) =>
      (order.get(left.id) ?? Number.MAX_SAFE_INTEGER) -
      (order.get(right.id) ?? Number.MAX_SAFE_INTEGER),
  );
}

function gitProbe(args, timeoutMs) {
  const result = spawnSync('git', args, {
    cwd: REPO_ROOT,
    encoding: 'utf8',
    shell: false,
    windowsHide: true,
    stdio: 'pipe',
    timeout: timeoutMs,
  });
  return {
    status: typeof result.status === 'number' ? result.status : 1,
    stdout: String(result.stdout || ''),
    stderr: String(result.stderr || ''),
    timedOut: result.error?.code === 'ETIMEDOUT',
  };
}

function registryProbe(key, timeoutMs) {
  const result = spawnSync('reg.exe', ['query', key], {
    cwd: REPO_ROOT,
    encoding: 'utf8',
    shell: false,
    windowsHide: true,
    stdio: 'pipe',
    timeout: timeoutMs,
  });
  return {
    key,
    status: typeof result.status === 'number' ? result.status : 2,
    timed_out: result.error?.code === 'ETIMEDOUT',
  };
}

function probeInstallRegistry(timeoutMs) {
  return evaluateRegistryProbe(
    INSTALL_REGISTRY_KEYS.map((key) => registryProbe(key, timeoutMs)),
  );
}

function assertNoSymlinkComponents(base, candidate) {
  const pathFromBase = relative(resolve(base), resolve(candidate));
  if (
    pathFromBase === '..' ||
    pathFromBase.startsWith(`..${sep}`) ||
    isAbsolute(pathFromBase)
  ) {
    fail('path_escape', 'path escaped its required parent');
  }
  let current = resolve(base);
  for (const component of pathFromBase.split(sep).filter(Boolean)) {
    current = join(current, component);
    const shape = lstatSync(current);
    if (shape.isSymbolicLink()) {
      fail('symlink_not_allowed', 'work-root components must not be symbolic links', {
        path: reportPath(current),
      });
    }
  }
}

function prepareWorkRoot(input) {
  const requested = resolve(REPO_ROOT, input);
  const shape = validateWorkRootPath(REPO_ROOT, requested);
  if (shape.status !== 'pass') {
    fail(shape.reason, 'work-root must be inside the repository build.noindex directory', {
      expected_root: reportPath(shape.expected_root),
      observed: reportPath(shape.observed),
    });
  }

  const buildRoot = resolve(REPO_ROOT, 'build.noindex');
  mkdirSync(requested, { recursive: true });
  assertNoSymlinkComponents(REPO_ROOT, requested);
  if (!statSync(requested).isDirectory()) {
    fail('work_root_not_directory', 'work-root is not a directory');
  }

  const repositoryReal = realpathSync.native(REPO_ROOT);
  const buildRootReal = realpathSync.native(buildRoot);
  const workRootReal = realpathSync.native(requested);
  if (
    !isPathWithin(repositoryReal, buildRootReal) ||
    !isPathWithin(buildRootReal, workRootReal)
  ) {
    fail('work_root_realpath_escape', 'work-root real path escaped build.noindex');
  }
  return {
    path: requested,
    realPath: workRootReal,
    buildRoot: buildRootReal,
    repository: repositoryReal,
  };
}

async function sha256File(path) {
  const hash = createHash('sha256');
  await new Promise((resolvePromise, reject) => {
    const stream = createReadStream(path);
    stream.on('data', (chunk) => hash.update(chunk));
    stream.once('error', reject);
    stream.once('end', resolvePromise);
  });
  return hash.digest('hex');
}

async function inspectRegularExe(path, allowedRoots) {
  const resolvedPath = resolve(path);
  let linkShape;
  let realPath;
  let realShape;
  try {
    linkShape = lstatSync(resolvedPath);
    realPath = realpathSync.native(resolvedPath);
    realShape = statSync(realPath);
  } catch {
    fail('artifact_missing', 'required executable artifact is missing', {
      path: reportPath(resolvedPath),
    });
  }
  const evaluation = evaluateRegularExeArtifact({
    resolvedPath,
    realPath,
    allowedRoots,
    isFile: linkShape.isFile() && realShape.isFile(),
    isSymbolicLink: linkShape.isSymbolicLink(),
    sizeBytes: realShape.size,
  });
  if (evaluation.status !== 'pass') {
    fail('artifact_shape_invalid', 'executable artifact failed the regular-file contract', {
      path: reportPath(resolvedPath),
      errors: evaluation.errors,
    });
  }
  const mostSpecificRoot = [...allowedRoots]
    .map((root) => resolve(root))
    .filter((root) => isPathWithin(root, realPath))
    .sort((left, right) => right.length - left.length)[0];
  return {
    path: reportPath(resolvedPath),
    real_path: reportPath(realPath),
    provenance_root: reportPath(mostSpecificRoot),
    size_bytes: realShape.size,
    sha256: await sha256File(realPath),
  };
}

function readPeMachineFromFile(path) {
  const descriptor = Buffer.alloc(64);
  const file = openSync(path, 'r');
  try {
    const descriptorBytes = readSync(file, descriptor, 0, descriptor.length, 0);
    if (descriptorBytes !== descriptor.length) {
      return { status: 'fail', reason: 'short_dos_header', machine: null };
    }
    const peOffset = descriptor.readUInt32LE(0x3c);
    if (peOffset < 64 || peOffset > 1024 * 1024) {
      return { status: 'fail', reason: 'invalid_pe_offset', machine: null };
    }
    const header = Buffer.alloc(peOffset + 6);
    descriptor.copy(header);
    const remaining = header.length - descriptor.length;
    const read = readSync(file, header, descriptor.length, remaining, descriptor.length);
    if (read !== remaining) {
      return { status: 'fail', reason: 'short_pe_header', machine: null };
    }
    return parsePeMachine(header);
  } finally {
    closeSync(file);
  }
}

function openChildLog(path) {
  mkdirSync(dirname(path), { recursive: true });
  return openSync(path, 'a');
}

function terminateProcessTreeSync(pid, timeoutMs = 5_000) {
  if (!Number.isInteger(pid) || pid <= 0) {
    return { attempted: false, status: null, timed_out: false };
  }
  const result = spawnSync(
    'taskkill.exe',
    ['/PID', String(pid), '/T', '/F'],
    {
      shell: false,
      windowsHide: true,
      stdio: 'ignore',
      timeout: timeoutMs,
    },
  );
  return {
    attempted: true,
    status: typeof result.status === 'number' ? result.status : null,
    timed_out: result.error?.code === 'ETIMEDOUT',
  };
}

async function runBoundedCommand(command, args, options) {
  const stdoutFile = openChildLog(options.stdoutLog);
  const stderrFile = openChildLog(options.stderrLog);
  let child;
  try {
    child = spawn(command, args, {
      cwd: options.cwd,
      env: options.env,
      shell: false,
      windowsHide: true,
      stdio: ['ignore', stdoutFile, stderrFile],
    });
  } finally {
    closeSync(stdoutFile);
    closeSync(stderrFile);
  }

  return new Promise((resolvePromise, reject) => {
    let settled = false;
    const startedAt = Date.now();
    const finish = (callback, value) => {
      if (settled) return;
      settled = true;
      clearTimeout(timer);
      callback(value);
    };
    const timer = setTimeout(() => {
      const cleanup = terminateProcessTreeSync(child.pid);
      finish(
        reject,
        new SmokeFailure('command_timeout', 'child command exceeded its bounded timeout', {
          timed_out: true,
          process_tree_cleanup: cleanup,
        }),
      );
    }, options.timeoutMs);

    child.once('error', () => {
      finish(
        reject,
        new SmokeFailure('command_spawn_failed', 'failed to spawn child command'),
      );
    });
    child.once('close', (code, signal) => {
      finish(resolvePromise, {
        exit_code: code,
        signal: signal || null,
        duration_ms: Date.now() - startedAt,
      });
    });
  });
}

async function launchInstalledApplication(binary, options) {
  const stdoutFile = openChildLog(options.stdoutLog);
  const stderrFile = openChildLog(options.stderrLog);
  let child;
  try {
    child = spawn(binary, [], {
      cwd: options.cwd,
      env: options.env,
      shell: false,
      windowsHide: true,
      stdio: ['ignore', stdoutFile, stderrFile],
    });
  } finally {
    closeSync(stdoutFile);
    closeSync(stderrFile);
  }

  await new Promise((resolvePromise, reject) => {
    let settled = false;
    const finish = (callback, value) => {
      if (settled) return;
      settled = true;
      clearTimeout(timer);
      callback(value);
    };
    const timer = setTimeout(() => {
      terminateProcessTreeSync(child.pid);
      finish(
        reject,
        new SmokeFailure('application_spawn_timeout', 'desktop application did not spawn'),
      );
    }, options.timeoutMs);
    child.once('spawn', () => finish(resolvePromise));
    child.once('error', () =>
      finish(
        reject,
        new SmokeFailure('application_spawn_failed', 'desktop application failed to spawn'),
      ),
    );
  });
  return child;
}

function processIsRunning(pid) {
  if (!Number.isInteger(pid) || pid <= 0) return false;
  try {
    process.kill(pid, 0);
    return true;
  } catch (error) {
    return error?.code === 'EPERM';
  }
}

function descendantProcessIdsSync(rootPid, timeoutMs = 5_000) {
  if (!Number.isInteger(rootPid) || rootPid <= 0) return [];
  const script = [
    `$rootPid = ${rootPid}`,
    '$rootProcess = Get-CimInstance Win32_Process -Filter "ProcessId = $rootPid"',
    'if ($null -eq $rootProcess) { ConvertTo-Json -Compress -InputObject @(); exit 0 }',
    '$rootCreated = $rootProcess.CreationDate',
    '$all = @(Get-CimInstance Win32_Process | Where-Object { $_.CreationDate -ge $rootCreated } | Select-Object ProcessId, ParentProcessId, CreationDate)',
    '$frontier = @($rootPid)',
    '$result = @()',
    'while ($frontier.Count -gt 0) {',
    '  $next = @()',
    '  foreach ($parentPid in $frontier) {',
    '    $children = @($all | Where-Object { $_.ParentProcessId -eq $parentPid })',
    '    foreach ($childProcess in $children) {',
    '      $result += [int]$childProcess.ProcessId',
    '      $next += [int]$childProcess.ProcessId',
    '    }',
    '  }',
    '  $frontier = $next',
    '}',
    'ConvertTo-Json -Compress -InputObject @($result)',
  ].join('; ');
  const attempts = [];
  for (let attempt = 1; attempt <= 3; attempt += 1) {
    const result = spawnSync(
      'powershell.exe',
      ['-NoLogo', '-NoProfile', '-NonInteractive', '-Command', script],
      {
        shell: false,
        windowsHide: true,
        encoding: 'utf8',
        stdio: 'pipe',
        timeout: timeoutMs,
      },
    );
    attempts.push({
      attempt,
      status: typeof result.status === 'number' ? result.status : null,
      timed_out: result.error?.code === 'ETIMEDOUT',
    });
    if (result.status !== 0 || result.error) continue;
    try {
      const parsed = JSON.parse(String(result.stdout || '[]'));
      const values = Array.isArray(parsed) ? parsed : [parsed];
      if (values.every((pid) => Number.isInteger(pid) && pid > 0)) {
        return [...new Set(values)];
      }
    } catch {
      // A concurrent process exit can make a CIM projection transiently
      // incomplete; retry the bounded, read-only snapshot.
    }
  }
  fail('process_tree_snapshot_failed', 'failed to snapshot the Desktop process tree', {
    snapshot_attempts: attempts,
  });
}

function terminateSingleProcessSync(pid, timeoutMs = 5_000) {
  const result = spawnSync(
    'taskkill.exe',
    ['/PID', String(pid), '/F'],
    {
      shell: false,
      windowsHide: true,
      stdio: 'ignore',
      timeout: timeoutMs,
    },
  );
  return {
    pid,
    status: typeof result.status === 'number' ? result.status : null,
    timed_out: result.error?.code === 'ETIMEDOUT',
  };
}

async function waitForProcessesAbsent(processIds, timeoutMs) {
  const deadline = Date.now() + timeoutMs;
  let remaining = processIds.filter(processIsRunning);
  while (remaining.length > 0 && Date.now() < deadline) {
    await sleep(POLL_INTERVAL_MS);
    remaining = processIds.filter(processIsRunning);
  }
  return remaining;
}

async function terminateApplicationTree(child, timeoutMs) {
  const rootPid = child?.pid;
  const runningBefore = processIsRunning(rootPid);
  const descendants = runningBefore
    ? descendantProcessIdsSync(rootPid, Math.min(timeoutMs, 5_000))
    : [];
  const processIds = [...descendants.reverse(), rootPid].filter(
    (pid) => Number.isInteger(pid) && pid > 0,
  );
  const terminations = [];
  for (const pid of processIds) {
    if (!processIsRunning(pid)) continue;
    terminations.push(
      terminateSingleProcessSync(pid, Math.min(timeoutMs, 5_000)),
    );
  }
  const remaining = await waitForProcessesAbsent(processIds, timeoutMs);
  if (remaining.length > 0) {
    fail('process_tree_cleanup_failed', 'desktop application process tree remained alive', {
      root_pid: rootPid || null,
      remaining_process_ids: remaining,
      terminations,
    });
  }
  return {
    root_pid: rootPid || null,
    running_before: runningBefore,
    root_exited: !processIsRunning(rootPid),
    descendant_count: descendants.length,
    remaining_process_ids: remaining,
    taskkill: {
      attempted: terminations.length > 0,
      timed_out: terminations.some((termination) => termination.timed_out),
      results: terminations,
    },
  };
}

async function reserveFreeTcpPort() {
  const server = createServer();
  await new Promise((resolvePromise, reject) => {
    server.once('error', reject);
    server.listen(0, '127.0.0.1', resolvePromise);
  });
  const address = server.address();
  await new Promise((resolvePromise, reject) =>
    server.close((error) => (error ? reject(error) : resolvePromise())),
  );
  if (!address || typeof address === 'string' || !Number.isInteger(address.port)) {
    fail('free_port_probe_failed', 'failed to reserve a WebView2 debugging port');
  }
  return address.port;
}

function assertApplicationStillRunning(child, phase) {
  if (child.exitCode !== null || child.signalCode !== null) {
    fail('application_exited_early', `desktop application exited before ${phase}`, {
      exit_code: child.exitCode,
      signal: child.signalCode || null,
    });
  }
}

async function waitForInstalledFiles(paths, timeoutMs) {
  const deadline = Date.now() + timeoutMs;
  while (Date.now() < deadline) {
    if (
      paths.every((path) => {
        try {
          const shape = lstatSync(path);
          return shape.isFile() && !shape.isSymbolicLink();
        } catch {
          return false;
        }
      })
    ) {
      return;
    }
    await sleep(POLL_INTERVAL_MS);
  }
  fail('installed_artifact_timeout', 'installed binary or uninstaller did not appear');
}

async function waitForPortAnnouncement(portFile, child, timeoutMs) {
  const deadline = Date.now() + timeoutMs;
  let lastErrors = ['port_file_missing'];
  while (Date.now() < deadline) {
    assertApplicationStillRunning(child, 'port announcement');
    try {
      const shape = lstatSync(portFile);
      if (!shape.isFile() || shape.isSymbolicLink()) {
        lastErrors = ['port_file_not_regular'];
      } else {
        const value = JSON.parse(readFileSync(portFile, 'utf8'));
        const evaluation = validatePortAnnouncement(value, child.pid);
        if (evaluation.status === 'pass') return evaluation.value;
        lastErrors = evaluation.errors;
      }
    } catch {
      lastErrors = ['port_file_missing_or_incomplete'];
    }
    await sleep(POLL_INTERVAL_MS);
  }
  fail('port_announcement_timeout', 'data root did not publish a valid port.json', {
    errors: lastErrors,
  });
}

async function fetchText(url, timeoutMs) {
  const controller = new AbortController();
  const timer = setTimeout(() => controller.abort(), timeoutMs);
  try {
    const response = await fetch(url, {
      method: 'GET',
      redirect: 'error',
      signal: controller.signal,
    });
    return {
      status: response.status,
      body: await response.text(),
    };
  } finally {
    clearTimeout(timer);
  }
}

async function waitForBackendHealth(url, child, timeoutMs) {
  const deadline = Date.now() + timeoutMs;
  let last = null;
  while (Date.now() < deadline) {
    assertApplicationStillRunning(child, 'backend health');
    try {
      const remaining = Math.max(1, deadline - Date.now());
      const response = await fetchText(url, Math.min(1_500, remaining));
      last = evaluateHealthResponse(response.status, response.body);
      if (last.status === 'pass') return last;
    } catch {
      last = null;
    }
    await sleep(POLL_INTERVAL_MS);
  }
  fail('backend_health_timeout', 'backend /health did not become healthy', {
    last_status_code: last?.status_code ?? null,
    last_response_status: last?.response_status ?? null,
  });
}

async function waitForNomiFunCdpTarget(endpoint, child, timeoutMs) {
  const deadline = Date.now() + timeoutMs;
  let observedTargetCount = null;
  while (Date.now() < deadline) {
    assertApplicationStillRunning(child, 'WebView2 CDP target');
    try {
      const remaining = Math.max(1, deadline - Date.now());
      const response = await fetchText(endpoint, Math.min(1_500, remaining));
      if (response.status === 200) {
        const targets = JSON.parse(response.body);
        observedTargetCount = Array.isArray(targets) ? targets.length : null;
        const target = findNomiFunCdpTarget(targets);
        if (target) return { target, observedTargetCount };
      }
    } catch {
      // WebView2 may not have opened its debugger endpoint yet.
    }
    await sleep(POLL_INTERVAL_MS);
  }
  fail('cdp_target_timeout', 'WebView2 CDP did not expose the NomiFun page', {
    observed_target_count: observedTargetCount,
  });
}

async function waitForUninstallCompletion(path, timeoutMs) {
  const deadline = Date.now() + timeoutMs;
  let registry = probeInstallRegistry(timeoutMs);
  while (
    (existsSync(path) || registry.status !== 'pass') &&
    Date.now() < deadline
  ) {
    await sleep(POLL_INTERVAL_MS);
    registry = probeInstallRegistry(Math.min(2_000, timeoutMs));
  }
  return {
    removed: !existsSync(path),
    registry,
  };
}

async function collectLogEvidence(logSpecs) {
  const entries = [];
  for (const { kind, path } of logSpecs) {
    try {
      const shape = lstatSync(path);
      if (!shape.isFile() || shape.isSymbolicLink()) {
        entries.push({ kind, path: reportPath(path), status: 'invalid' });
        continue;
      }
      entries.push({
        kind,
        path: reportPath(path),
        status: 'available',
        size_bytes: shape.size,
        sha256: await sha256File(path),
      });
    } catch {
      entries.push({ kind, path: reportPath(path), status: 'missing' });
    }
  }
  return entries;
}

export async function runCandidateSmoke(options) {
  if (!options || options.selfTest) {
    throw new Error('runCandidateSmoke requires operational arguments');
  }
  if (!SOURCE_COMMIT_PATTERN.test(options.sourceCommit || '')) {
    throw new Error('sourceCommit must be exactly 40 hexadecimal characters');
  }

  const sourceCommit = options.sourceCommit.toLowerCase();
  const report = createInitialResult(sourceCommit);
  const productChecks = options.productChecks ?? [];
  if (!Array.isArray(productChecks)) {
    throw new Error('productChecks must be an array when provided');
  }
  const productCheckIds = new Set();
  for (const check of productChecks) {
    if (
      !check ||
      typeof check.id !== 'string' ||
      !/^[a-z0-9][a-z0-9-]{0,95}$/.test(check.id) ||
      CHECK_IDS.includes(check.id) ||
      productCheckIds.has(check.id) ||
      typeof check.run !== 'function' ||
      !Number.isSafeInteger(check.timeoutMs) ||
      check.timeoutMs <= 0
    ) {
      throw new Error('each product check requires a unique id, run function, and bounded timeout');
    }
    productCheckIds.add(check.id);
  }
  const cleanupIndex = CHECK_IDS.indexOf('process-tree-cleanup');
  const plannedCheckIds = [
    ...CHECK_IDS.slice(0, cleanupIndex),
    ...productChecks.map((check) => check.id),
    ...CHECK_IDS.slice(cleanupIndex),
  ];
  report.suite.checks = plannedCheckIds;
  const logSpecs = [];
  let canProceed = true;
  let work = null;
  let runRoot = null;
  let installDirectory = null;
  let dataRoot = null;
  let mainBinary = null;
  let uninstaller = null;
  let appChild = null;
  let applicationEnvironment = null;
  let installationToken = null;
  let launchOrdinal = 0;
  let installAttempted = false;

  const launchProductApplication = async () => {
    launchOrdinal += 1;
    const cdpPort = await reserveFreeTcpPort();
    applicationEnvironment = {
      ...applicationEnvironment,
      WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS: `--remote-debugging-port=${cdpPort}`,
    };
    const suffix = launchOrdinal === 1 ? 'application' : `application-restart-${launchOrdinal}`;
    const stdoutLog = join(runRoot, 'logs', `${suffix}.stdout.log`);
    const stderrLog = join(runRoot, 'logs', `${suffix}.stderr.log`);
    logSpecs.push(
      { kind: `${suffix}_stdout`, path: stdoutLog },
      { kind: `${suffix}_stderr`, path: stderrLog },
    );
    appChild = await launchInstalledApplication(mainBinary, {
      cwd: installDirectory,
      env: applicationEnvironment,
      stdoutLog,
      stderrLog,
      timeoutMs: CHECK_TIMEOUTS_MS.launch,
    });
    report.install.application_pid = appChild.pid;
    report.cdp.port = cdpPort;
    report.cdp.endpoint = `http://127.0.0.1:${cdpPort}/json/list`;

    const portFile = join(dataRoot, 'port.json');
    const announcement = await waitForPortAnnouncement(
      portFile,
      appChild,
      CHECK_TIMEOUTS_MS['port-announcement'],
    );
    report.data.announcement = announcement;
    report.backend.base_url = loopbackBaseUrl(announcement.host, announcement.port);
    report.backend.health_url = `${report.backend.base_url}/health`;
    const health = await waitForBackendHealth(
      report.backend.health_url,
      appChild,
      CHECK_TIMEOUTS_MS['backend-health'],
    );
    Object.assign(report.backend, health);
    const cdp = await waitForNomiFunCdpTarget(
      report.cdp.endpoint,
      appChild,
      CHECK_TIMEOUTS_MS['webview2-cdp'],
    );
    report.cdp.target = cdp.target;
    report.cdp.observed_target_count = cdp.observedTargetCount;
    return {
      pid: appChild.pid,
      baseUrl: report.backend.base_url,
      cdpEndpoint: report.cdp.endpoint,
      cdpTarget: cdp.target,
      announcement,
      health,
    };
  };

  try {
    canProceed =
      canProceed &&
      (await runCheck(report, 'native-host', () => {
        if (process.platform !== 'win32' || process.arch !== 'x64') {
          fail('native_windows_x64_required', 'smoke requires native Windows x64', {
            observed_platform: process.platform,
            observed_arch: process.arch,
          });
        }
        return {
          observed_platform: process.platform,
          observed_arch: process.arch,
          target_triple: TARGET_TRIPLE,
        };
      }));

    if (canProceed) {
      canProceed = await runCheck(report, 'source-checkpoint', () => {
        const timeoutMs = CHECK_TIMEOUTS_MS['source-checkpoint'];
        const head = gitProbe(['rev-parse', 'HEAD'], timeoutMs);
        const status = gitProbe(
          ['status', '--porcelain=v1', '--untracked-files=all'],
          timeoutMs,
        );
        const checkpoint = evaluateSourceCheckpoint({
          expected: sourceCommit,
          head: head.stdout,
          statusOutput: status.stdout,
          headStatus: head.status,
          statusStatus: status.status,
        });
        if (head.timedOut || status.timedOut) {
          fail('git_probe_timeout', 'git source checkpoint probe timed out');
        }
        if (checkpoint.status !== 'pass') {
          fail(
            'source_checkpoint_failed',
            'clean HEAD does not match the declared source commit',
            checkpoint,
          );
        }
        return checkpoint;
      });
    }

    if (canProceed) {
      canProceed = await runCheck(report, 'work-root', () => {
        work = prepareWorkRoot(resolve(REPO_ROOT, options.workRoot));
        runRoot = join(
          work.path,
          compactRunId(),
        );
        installDirectory = join(runRoot, 'install');
        dataRoot = join(runRoot, 'data');
        mainBinary = join(installDirectory, MAIN_BINARY_NAME);
        uninstaller = join(installDirectory, UNINSTALLER_NAME);
        mkdirSync(join(runRoot, 'logs'), { recursive: true });
        mkdirSync(dataRoot, { recursive: true });
        assertNoSymlinkComponents(work.path, runRoot);

        report.artifacts.result_root = reportPath(runRoot);
        report.install.root = reportPath(installDirectory);
        report.data.root = reportPath(dataRoot);
        report.data.port_file = reportPath(join(dataRoot, 'port.json'));
        return {
          work_root: reportPath(work.path),
          run_root: reportPath(runRoot),
          install_root: reportPath(installDirectory),
          data_root: reportPath(dataRoot),
        };
      });
    }

    if (canProceed) {
      canProceed = await runCheck(report, 'installer-artifact', async () => {
        const installerPath = resolve(REPO_ROOT, options.installer);
        const artifact = await inspectRegularExe(installerPath, [
          work.realPath,
          work.repository,
        ]);
        report.artifacts.installer = artifact;
        return artifact;
      });
    }

    if (canProceed) {
      canProceed = await runCheck(report, 'installation-preflight', () => {
        const registry = probeInstallRegistry(
          CHECK_TIMEOUTS_MS['installation-preflight'],
        );
        if (registry.status !== 'pass') {
          fail(
            'existing_installation_detected',
            'a user-level NomiFun installation already exists; candidate smoke refuses to overwrite it',
            registry,
          );
        }
        return registry;
      });
    }

    if (canProceed) {
      canProceed = await runCheck(report, 'install', async () => {
        if (existsSync(installDirectory)) {
          fail('install_root_not_absent', 'isolated install directory already exists');
        }
        installAttempted = true;
        const stdoutLog = join(runRoot, 'logs', 'installer.stdout.log');
        const stderrLog = join(runRoot, 'logs', 'installer.stderr.log');
        logSpecs.push(
          { kind: 'installer_stdout', path: stdoutLog },
          { kind: 'installer_stderr', path: stderrLog },
        );
        const args = buildNsisInstallArgs(installDirectory);
        if (args.at(-1) !== `/D=${resolve(installDirectory)}`) {
          fail('nsis_directory_argument_order', 'NSIS /D override must be the last argument');
        }
        const outcome = await runBoundedCommand(
          resolve(REPO_ROOT, options.installer),
          args,
          {
            cwd: REPO_ROOT,
            env: environmentWithoutSecrets(process.env),
            stdoutLog,
            stderrLog,
            timeoutMs: CHECK_TIMEOUTS_MS.install,
          },
        );
        report.install.installer_exit_code = outcome.exit_code;
        if (outcome.exit_code !== 0) {
          fail('installer_nonzero_exit', 'NSIS installer returned a non-zero exit code', {
            exit_code: outcome.exit_code,
            signal: outcome.signal,
          });
        }
        return {
          exit_code: outcome.exit_code,
          arguments: ['/S', '/NS', `/D=${reportPath(installDirectory)}`],
          directory_override_is_last: true,
        };
      });
    }

    if (canProceed) {
      canProceed = await runCheck(report, 'installed-binary', async () => {
        await waitForInstalledFiles(
          [mainBinary, uninstaller],
          CHECK_TIMEOUTS_MS['installed-binary'],
        );
        const host = await inspectRegularExe(mainBinary, [resolve(installDirectory)]);
        const uninstallArtifact = await inspectRegularExe(uninstaller, [
          resolve(installDirectory),
        ]);
        const pe = readPeMachineFromFile(mainBinary);
        if (pe.status !== 'pass') {
          fail('installed_binary_not_x64', 'installed desktop binary is not Windows x64', {
            pe,
          });
        }
        report.artifacts.host = { ...host, pe };
        report.artifacts.uninstaller = uninstallArtifact;
        return {
          host: report.artifacts.host,
          uninstaller: uninstallArtifact,
        };
      });
    }

    if (canProceed) {
      canProceed = await runCheck(report, 'launch', async () => {
        const cdpPort = await reserveFreeTcpPort();
        const stdoutLog = join(runRoot, 'logs', 'application.stdout.log');
        const stderrLog = join(runRoot, 'logs', 'application.stderr.log');
        logSpecs.push(
          { kind: 'application_stdout', path: stdoutLog },
          { kind: 'application_stderr', path: stderrLog },
        );
        const environment = environmentWithoutSecrets(process.env);
        environment.NOMIFUN_DATA_DIR = dataRoot;
        environment.WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS =
          `--remote-debugging-port=${cdpPort}`;
        if (productChecks.length > 0) {
          installationToken = randomBytes(32).toString('hex');
          environment.NOMIFUN_ACCESS_TOKEN = installationToken;
        }
        applicationEnvironment = environment;
        appChild = await launchInstalledApplication(mainBinary, {
          cwd: installDirectory,
          env: environment,
          stdoutLog,
          stderrLog,
          timeoutMs: CHECK_TIMEOUTS_MS.launch,
        });
        report.install.application_pid = appChild.pid;
        report.cdp.port = cdpPort;
        report.cdp.endpoint = `http://127.0.0.1:${cdpPort}/json/list`;
        launchOrdinal = 1;
        return {
          pid: appChild.pid,
          executable: reportPath(mainBinary),
          cdp_port: cdpPort,
          data_root: reportPath(dataRoot),
        };
      });
    }

    if (canProceed) {
      canProceed = await runCheck(report, 'port-announcement', async () => {
        const portFile = join(dataRoot, 'port.json');
        const announcement = await waitForPortAnnouncement(
          portFile,
          appChild,
          CHECK_TIMEOUTS_MS['port-announcement'],
        );
        report.data.announcement = announcement;
        report.backend.base_url = loopbackBaseUrl(
          announcement.host,
          announcement.port,
        );
        report.backend.health_url = `${report.backend.base_url}/health`;
        return {
          port_file: reportPath(portFile),
          announcement,
        };
      });
    }

    if (canProceed) {
      canProceed = await runCheck(report, 'backend-health', async () => {
        const health = await waitForBackendHealth(
          report.backend.health_url,
          appChild,
          CHECK_TIMEOUTS_MS['backend-health'],
        );
        Object.assign(report.backend, health);
        return health;
      });
    }

    if (canProceed) {
      canProceed = await runCheck(report, 'webview2-cdp', async () => {
        const observed = await waitForNomiFunCdpTarget(
          report.cdp.endpoint,
          appChild,
          CHECK_TIMEOUTS_MS['webview2-cdp'],
        );
        report.cdp.target = observed.target;
        report.cdp.observed_target_count = observed.observedTargetCount;
        return {
          endpoint: report.cdp.endpoint,
          observed_target_count: observed.observedTargetCount,
          target: observed.target,
        };
      });
    }

    for (const productCheck of productChecks) {
      if (!canProceed) break;
      canProceed = await runCheck(
        report,
        productCheck.id,
        async () => {
          const restart = async () => {
            const cleanup = await terminateApplicationTree(
              appChild,
              CHECK_TIMEOUTS_MS['process-tree-cleanup'],
            );
            const launched = await launchProductApplication();
            return { cleanup, ...launched };
          };
          return productCheck.run({
            sourceCommit,
            runRoot,
            installDirectory,
            dataRoot,
            mainBinary,
            installationToken,
            getBaseUrl: () => report.backend.base_url,
            getCdpEndpoint: () => report.cdp.endpoint,
            getCdpTarget: () => report.cdp.target,
            assertApplicationRunning: (phase) =>
              assertApplicationStillRunning(appChild, phase),
            restart,
          });
        },
        productCheck.timeoutMs,
      );
    }
  } finally {
    if (appChild) {
      const cleanupPassed = await runCheck(
        report,
        'process-tree-cleanup',
        async () => {
          const cleanup = await terminateApplicationTree(
            appChild,
            CHECK_TIMEOUTS_MS['process-tree-cleanup'],
          );
          report.install.process_tree_cleanup = cleanup;
          return cleanup;
        },
      );
      if (!cleanupPassed) canProceed = false;
    }

    const isolatedUninstallerExists =
      Boolean(uninstaller) &&
      existsSync(uninstaller) &&
      isPathWithin(installDirectory, uninstaller);
    if (isolatedUninstallerExists) {
      const uninstallPassed = await runCheck(report, 'uninstall', async () => {
        const stdoutLog = join(runRoot, 'logs', 'uninstaller.stdout.log');
        const stderrLog = join(runRoot, 'logs', 'uninstaller.stderr.log');
        logSpecs.push(
          { kind: 'uninstaller_stdout', path: stdoutLog },
          { kind: 'uninstaller_stderr', path: stderrLog },
        );
        const shape = lstatSync(uninstaller);
        if (!shape.isFile() || shape.isSymbolicLink()) {
          fail('uninstaller_shape_invalid', 'isolated uninstaller is not a regular file');
        }
        const outcome = await runBoundedCommand(uninstaller, ['/S'], {
          cwd: runRoot,
          env: environmentWithoutSecrets(process.env),
          stdoutLog,
          stderrLog,
          timeoutMs: CHECK_TIMEOUTS_MS.uninstall,
        });
        report.install.uninstaller_exit_code = outcome.exit_code;
        if (outcome.exit_code !== 0) {
          fail('uninstaller_nonzero_exit', 'NSIS uninstaller returned a non-zero exit code', {
            exit_code: outcome.exit_code,
            signal: outcome.signal,
          });
        }
        return {
          executable: reportPath(uninstaller),
          arguments: ['/S'],
          exit_code: outcome.exit_code,
        };
      });
      if (!uninstallPassed) canProceed = false;
    } else if (installAttempted && mainBinary && existsSync(mainBinary)) {
      report.checks.push({
        id: 'uninstall',
        status: 'fail',
        timeout_ms: CHECK_TIMEOUTS_MS.uninstall,
        code: 'isolated_uninstaller_missing',
        reason: 'installed main binary remains but isolated uninstaller is missing',
      });
      canProceed = false;
    }

    if (installAttempted && mainBinary) {
      const verificationPassed = await runCheck(
        report,
        'uninstall-verification',
        async () => {
          const completion = await waitForUninstallCompletion(
            mainBinary,
            CHECK_TIMEOUTS_MS['uninstall-verification'],
          );
          const { removed, registry } = completion;
          report.install.main_binary_removed = removed;
          if (!removed) {
            fail('main_binary_remains', 'installed main binary remains after uninstall');
          }
          if (registry.status !== 'pass') {
            fail(
              'installer_registry_remains',
              'NomiFun installer registry state remains after uninstall',
              registry,
            );
          }
          return {
            main_binary: reportPath(mainBinary),
            removed,
            install_directory_exists: existsSync(installDirectory),
            registry,
          };
        },
      );
      if (!verificationPassed) canProceed = false;
    }
  }

  markMissingChecksSkipped(report, plannedCheckIds);
  report.logs = await collectLogEvidence(logSpecs);
  report.status =
    canProceed &&
    report.checks.length === plannedCheckIds.length &&
    report.checks.every((entry) => entry.status === 'pass')
      ? 'pass'
      : 'fail';
  return report;
}

export function assertSelfTest() {
  const sourceCommit = 'a'.repeat(40);
  const args = parseArgs([
    '--installer',
    'target/release/bundle/nsis/NomiFun.exe',
    '--source-commit',
    sourceCommit,
    '--work-root',
    'build.noindex/candidate',
  ]);
  if (args.sourceCommit !== sourceCommit) throw new Error('argument parser failed');

  const work = validateWorkRootPath('/repo', '/repo/build.noindex/candidate');
  const escaped = validateWorkRootPath('/repo', '/repo/target');
  if (work.status !== 'pass' || escaped.status !== 'fail') {
    throw new Error('work-root boundary self-test failed');
  }

  const nsisInstallDirectory = resolve('/repo/build.noindex/candidate/install');
  const nsisArgs = buildNsisInstallArgs(nsisInstallDirectory);
  if (
    nsisArgs.length !== 3 ||
    nsisArgs[1] !== '/NS' ||
    nsisArgs.at(-1) !== `/D=${nsisInstallDirectory}`
  ) {
    throw new Error('NSIS argument ordering self-test failed');
  }
  const registry = evaluateRegistryProbe(
    INSTALL_REGISTRY_KEYS.map((key) => ({ key, status: 1 })),
  );
  if (registry.status !== 'pass') {
    throw new Error('installation registry preflight self-test failed');
  }

  const checkpoint = evaluateSourceCheckpoint({
    expected: sourceCommit,
    head: `${sourceCommit}\n`,
    statusOutput: '',
    headStatus: 0,
    statusStatus: 0,
  });
  if (checkpoint.status !== 'pass') throw new Error('source checkpoint self-test failed');

  const artifact = evaluateRegularExeArtifact({
    resolvedPath: '/repo/build.noindex/candidate/installer.exe',
    realPath: '/repo/build.noindex/candidate/installer.exe',
    allowedRoots: ['/repo'],
    isFile: true,
    isSymbolicLink: false,
    sizeBytes: 128,
  });
  if (artifact.status !== 'pass') throw new Error('artifact self-test failed');

  const announcement = validatePortAnnouncement(
    { host: '127.0.0.1', port: 25808, channel: 'stable', pid: 42 },
    42,
  );
  if (announcement.status !== 'pass') throw new Error('port announcement self-test failed');

  const cdp = findNomiFunCdpTarget([
    {
      id: 'page-1',
      type: 'page',
      title: 'NomiFun',
      url: 'http://tauri.localhost/index.html',
    },
  ]);
  if (!cdp) throw new Error('CDP target self-test failed');

  const pe = Buffer.alloc(128);
  pe.write('MZ', 0, 'ascii');
  pe.writeUInt32LE(64, 0x3c);
  pe.write('PE\0\0', 64, 'binary');
  pe.writeUInt16LE(PE_MACHINE_AMD64, 68);
  if (parsePeMachine(pe).status !== 'pass') throw new Error('PE machine self-test failed');

  const environment = environmentWithoutSecrets({
    PATH: 'safe',
    NOMIFUN_LIVE_STEPFUN_API_KEY: 'must-not-survive',
  });
  if (environment.PATH !== 'safe' || 'NOMIFUN_LIVE_STEPFUN_API_KEY' in environment) {
    throw new Error('environment redaction self-test failed');
  }

  return {
    schema_version: '1.0.0',
    target: TARGET_ID,
    status: 'pass',
    suite: {
      name: 'windows-desktop-candidate-install-smoke-self-test',
      checks: [
        'arguments',
        'work-root-boundary',
        'nsis-arguments',
        'installation-registry',
        'source-checkpoint',
        'artifact-shape',
        'port-announcement',
        'cdp-target',
        'pe-machine',
        'environment-redaction',
      ],
    },
  };
}

function fatalResult(error, sourceCommit = null) {
  const result = createInitialResult(sourceCommit);
  result.checks = [
    {
      id: 'runner',
      status: 'fail',
      code: error instanceof Error ? 'runner_error' : 'runner_failure',
      reason: error instanceof Error ? error.message : 'runner failed',
    },
  ];
  result.suite.checks = ['runner'];
  return result;
}

const isMain =
  process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url);
if (isMain) {
  let options = null;
  try {
    options = parseArgs(process.argv.slice(2));
    if (options.selfTest) {
      const result = assertSelfTest();
      console.log(JSON.stringify(result, null, 2));
      process.exitCode = 0;
    } else {
      const result = await runCandidateSmoke(options);
      console.log(JSON.stringify(result, null, 2));
      process.exitCode = result.status === 'pass' ? 0 : 1;
    }
  } catch (error) {
    const sourceCommit =
      options && !options.selfTest ? options.sourceCommit || null : null;
    const result = fatalResult(error, sourceCommit);
    console.log(JSON.stringify(result, null, 2));
    process.exitCode = 2;
  }
}
