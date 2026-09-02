#!/usr/bin/env node

/**
 * Target-specific C8-MA preflight.
 *
 * This helper is deliberately independent from scripts/gate-agent-v2.mjs.  It
 * only produces engineering evidence from the current host and supplied
 * artifacts; it never manufactures PlatformCellEvidence or upgrades a
 * pending/blocked check to pass.
 *
 * Usage:
 *   bun scripts/validation/check-macos-arm64-native.mjs \
 *     --release-lock /abs/release-lock.json
 *
 * Optional live checks:
 *   --host-binary /abs/nomicore --run-startup
 *   --endpoint http://127.0.0.1:25808 --binding-id <id> --run-lifecycle
 *   --credential-file /abs/credential --run-sidecar-rpc
 */

import { spawn, spawnSync } from 'node:child_process';
import {
  chmodSync,
  lstatSync,
  mkdirSync,
  mkdtempSync,
  readFileSync,
  readdirSync,
  rmSync,
  symlinkSync,
  writeFileSync,
} from 'node:fs';
import { tmpdir } from 'node:os';
import { dirname, join, parse, resolve, sep } from 'node:path';
import { fileURLToPath } from 'node:url';

import {
  readAndVerifyReleaseLock,
  resolveReleaseArtifactPath,
  sha256File,
} from '../release/release-lock.mjs';

export const REPO_ROOT = resolve(dirname(fileURLToPath(import.meta.url)), '../..');
export const TARGET_ID = 'macos_desktop_arm64';
export const EXPECTED_TARGET = 'aarch64-apple-darwin';
export const EXPECTED_CAPABILITY_COUNT = 137;
export const EXPECTED_PROFILES = ['coding_native', 'managed_minimal'];
export const EXPECTED_RPC_METHODS = [
  'create',
  'resume',
  'fork',
  'start_turn',
  'steer',
  'follow_up',
  'cancel',
  'session_dispose',
];
export const EXPECTED_FORK_COMMIT = 'dc2ccc6843abb09c9d297862dc10b6bd12a3935d';
export const EXPECTED_PROTOCOL_VERSION = '1.0.0';
export const EXPECTED_PROTOCOL_SCHEMA_DIGEST =
  'f1c0422f04c9de923e18c7df40d814d3c9f5b2db5f1c5fef2745e77e6d62590f';
const SHA256_PATTERN = /^[0-9a-f]{64}$/;

function parseArgs(argv) {
  const options = {
    releaseLock: process.env.NOMIFUN_RELEASE_LOCK_PATH || null,
    artifactRoot: process.env.NOMIFUN_RELEASE_ARTIFACT_ROOT || REPO_ROOT,
    sidecar: null,
    hello: null,
    sidecarDir: null,
    app: null,
    dmg: null,
    hostBinary: null,
    endpoint: null,
    bindingId: null,
    token: process.env.NOMIFUN_ACCESS_TOKEN || null,
    credentialFile: null,
    report: null,
    logs: [],
    runStartup: false,
    runLifecycle: false,
    runSidecarRpc: false,
    selfTest: false,
  };
  for (let index = 0; index < argv.length; index += 1) {
    const token = argv[index];
    if (token === '--self-test') {
      options.selfTest = true;
      continue;
    }
    if (token === '--run-startup') {
      options.runStartup = true;
      continue;
    }
    if (token === '--run-lifecycle') {
      options.runLifecycle = true;
      continue;
    }
    if (token === '--run-sidecar-rpc') {
      options.runSidecarRpc = true;
      continue;
    }
    const match = token.match(/^--([^=]+)(?:=(.*))?$/);
    if (!match) throw new Error(`unknown argument: ${token}`);
    const key = match[1].replaceAll('-', '');
    let value = match[2];
    if (value === undefined) {
      value = argv[++index];
      if (!value || value.startsWith('--')) throw new Error(`${token} requires a value`);
    }
    const mapping = {
      sidecar: 'sidecar',
      hello: 'hello',
      sidecardir: 'sidecarDir',
      app: 'app',
      dmg: 'dmg',
      hostbinary: 'hostBinary',
      endpoint: 'endpoint',
      bindingid: 'bindingId',
      token: 'token',
      credentialfile: 'credentialFile',
      report: 'report',
      releaselock: 'releaseLock',
      artifactroot: 'artifactRoot',
      log: 'logs',
    };
    if (!(key in mapping)) throw new Error(`unknown argument: ${token}`);
    if (mapping[key] === 'logs') options.logs.push(value);
    else options[mapping[key]] = value;
  }
  if (options.runLifecycle && (!options.endpoint || !options.bindingId)) {
    throw new Error('--run-lifecycle requires --endpoint and --binding-id');
  }
  if (options.runSidecarRpc && !options.credentialFile) {
    throw new Error('--run-sidecar-rpc requires --credential-file');
  }
  return options;
}

function command(command, args, timeout = 10_000) {
  const result = spawnSync(command, args, {
    cwd: REPO_ROOT,
    encoding: 'utf8',
    timeout,
    shell: false,
    stdio: 'pipe',
  });
  return {
    command: [command, ...args].join(' '),
    status: result.status,
    stdout: String(result.stdout || ''),
    stderr: String(result.stderr || ''),
    error: result.error?.message || null,
    timedOut: result.error?.code === 'ETIMEDOUT',
  };
}

function readJson(path) {
  return JSON.parse(readFileSync(path, 'utf8'));
}

function check(report, id, status, details = {}) {
  report.checks.push({ id, status, ...details });
  if (status === 'fail' || status === 'blocked') report.failures.push({ id, ...details });
}

function finishReport(report, options) {
  report.suite.checks = report.checks.map((entry) => entry.id);
  report.status = report.checks.some((entry) => entry.status === 'fail')
    ? 'fail'
    : report.checks.some((entry) => entry.status === 'blocked')
      ? 'blocked'
      : 'pass';
  if (options.report) {
    mkdirSync(dirname(resolve(options.report)), { recursive: true });
    writeFileSync(resolve(options.report), `${JSON.stringify(report, null, 2)}\n`);
  }
  return report;
}

function existingFile(path) {
  try {
    return lstatSync(path);
  } catch {
    return null;
  }
}

function exactCasePath(path) {
  const absolute = resolve(path);
  const root = parse(absolute).root;
  let current = root;
  const components = absolute
    .slice(root.length)
    .split(sep)
    .filter(Boolean);
  for (const component of components) {
    if (!component) continue;
    let entries;
    try {
      entries = readdirSync(current);
    } catch {
      return { status: 'unknown', path: absolute };
    }
    if (!entries.includes(component)) {
      return {
        status: 'fail',
        path: absolute,
        parent: current,
        component,
        sibling_match: entries.find((entry) => entry.toLowerCase() === component.toLowerCase()) || null,
      };
    }
    current = join(current, component);
  }
  return { status: 'pass', path: absolute };
}

function isExecutableMode(mode) {
  return (mode & 0o111) !== 0 && (mode & 0o022) === 0;
}

function appFromHostBinary(hostBinary) {
  const macosDirectory = dirname(hostBinary);
  const contentsDirectory = dirname(macosDirectory);
  const app = dirname(contentsDirectory);
  return macosDirectory === join(contentsDirectory, 'MacOS') &&
    contentsDirectory === join(app, 'Contents')
    ? app
    : null;
}

function checkOptionalArtifactOverride(report, id, override, lockedPath) {
  if (!override) return;
  const observed = resolve(override);
  check(report, id, observed === lockedPath ? 'pass' : 'fail', {
    expected_from_release_lock: lockedPath,
    observed,
  });
}

function logReference(report, path) {
  const absolute = resolve(path);
  const shape = validatePathShape(absolute);
  check(report, `log:${absolute}`, shape.status, shape);
  if (shape.status === 'pass') {
    report.logs.push({ kind: 'file', path: absolute, sha256: sha256File(absolute) });
  }
}

async function waitForHttp(url, timeoutMs = 30_000) {
  const deadline = Date.now() + timeoutMs;
  let lastError = null;
  while (Date.now() < deadline) {
    try {
      const response = await fetch(url);
      return { response, body: await response.text() };
    } catch (error) {
      lastError = error;
      await new Promise((resolvePromise) => setTimeout(resolvePromise, 250));
    }
  }
  throw new Error(`timed out waiting for ${url}: ${lastError?.message || 'no response'}`);
}

function descendantsOf(pid) {
  const ps = command('ps', ['-axo', 'pid=,ppid=,comm=']);
  if (ps.status !== 0) return { error: ps.stderr || ps.error || 'ps failed', pids: [] };
  const rows = ps.stdout
    .split(/\r?\n/)
    .map((line) => line.trim().split(/\s+/, 3))
    .filter((parts) => parts.length >= 2 && /^\d+$/.test(parts[0]) && /^\d+$/.test(parts[1]))
    .map(([child, parent, comm]) => ({ pid: Number(child), ppid: Number(parent), comm }));
  const rootPid = Number(pid);
  const found = new Set(
    rows.some((row) => row.pid === rootPid) ? [rootPid] : [],
  );
  let changed = true;
  while (changed) {
    changed = false;
    for (const row of rows) {
      if (found.has(row.ppid) && !found.has(row.pid)) {
        found.add(row.pid);
        changed = true;
      }
    }
  }
  return { error: null, pids: [...found].sort((a, b) => a - b), rows };
}

async function waitForProcessTreeGone(pid, timeoutMs = 5_000) {
  const deadline = Date.now() + timeoutMs;
  let observed = descendantsOf(pid);
  while (observed.pids.length > 0 && Date.now() < deadline) {
    await new Promise((resolvePromise) => setTimeout(resolvePromise, 250));
    observed = descendantsOf(pid);
  }
  return observed;
}

async function stopChild(child) {
  if (!child) return;
  if (child.exitCode !== null) return;
  const exited = new Promise((resolvePromise) => {
    child.once('exit', resolvePromise);
  });
  child.kill('SIGTERM');
  await Promise.race([
    exited,
    new Promise((resolvePromise) => setTimeout(resolvePromise, 2_000)),
  ]);
  if (child.exitCode === null) {
    child.kill('SIGKILL');
    await Promise.race([
      exited,
      new Promise((resolvePromise) => setTimeout(resolvePromise, 2_000)),
    ]);
  }
}

async function startupSmoke(binary, root, report, label) {
  const port = 28000 + Math.floor(Math.random() * 1000);
  const child = spawn(
    binary,
    ['--data-dir', root, '--work-dir', root, '--port', String(port), '--local', '--log-level', 'error'],
    { cwd: REPO_ROOT, stdio: ['ignore', 'pipe', 'pipe'] },
  );
  let stderr = '';
  child.stderr.on('data', (chunk) => {
    stderr += String(chunk);
  });
  try {
    const health = await waitForHttp(`http://127.0.0.1:${port}/health`);
    if (health.response.status !== 200) {
      check(report, label, 'fail', { reason: 'health_status', status_code: health.response.status });
      return;
    }
    const capabilities = await fetch(`http://127.0.0.1:${port}/api/capabilities`);
    const body = await capabilities.json().catch(() => null);
    const count = Array.isArray(body?.data) ? body.data.length : null;
    check(report, `${label}:health`, 'pass', { status_code: health.response.status });
    check(report, `${label}:capability_inventory`, count === EXPECTED_CAPABILITY_COUNT ? 'pass' : 'fail', {
      expected: EXPECTED_CAPABILITY_COUNT,
      observed: count,
      status_code: capabilities.status,
      response: body,
    });
  } catch (error) {
    check(report, label, 'fail', { reason: error.message, stderr_tail: stderr.slice(-2_000) });
  } finally {
    const before = descendantsOf(child.pid);
    await stopChild(child);
    const after = await waitForProcessTreeGone(child.pid);
    check(report, `${label}:process_cleanup`, after.pids.length === 0 ? 'pass' : 'fail', {
      root_pid: child.pid,
      descendants_before: before.pids,
      remaining_pids: after.pids,
    });
  }
}

async function remoteLifecycle(options, report) {
  const base = options.endpoint.replace(/\/+$/, '');
  const headers = {
    'content-type': 'application/json',
    ...(options.token ? { authorization: `Bearer ${options.token}` } : {}),
  };
  const request = async (method, path, body) => {
    const response = await fetch(`${base}${path}`, {
      method,
      headers,
      body: body === undefined ? undefined : JSON.stringify(body),
    });
    const text = await response.text();
    let value = null;
    try {
      value = JSON.parse(text);
    } catch {
      // Preserve the raw response in evidence; malformed JSON is a failure.
    }
    return { response, text, value };
  };
  const id = `macos-arm64-validation-${Date.now()}`;
  const opened = await request('POST', '/api/remote/open', {
    binding_id: options.bindingId,
    idempotency_key: `${id}-open`,
    initial_input: { text: 'native C8-MA lifecycle validation' },
  });
  const sessionId = opened.value?.agent_session_id;
  check(report, 'lifecycle:open', opened.response.ok && typeof sessionId === 'string' ? 'pass' : 'fail', {
    status_code: opened.response.status,
    response: opened.value || opened.text.slice(0, 2_000),
  });
  if (!sessionId) return;

  let cursor = Number(opened.value?.cursor?.seq || 0);
  let ready = opened.value?.open_state?.state === 'ready';
  let lastObserve = null;
  for (let attempt = 0; !ready && attempt < 40; attempt += 1) {
    lastObserve = await request(
      'GET',
      `/api/remote/observe?agent_session_id=${encodeURIComponent(sessionId)}&after_seq=${cursor}&limit=100`,
    );
    if (!lastObserve.response.ok) break;
    cursor = Number(lastObserve.value?.next_cursor?.seq || cursor);
    const events = Array.isArray(lastObserve.value?.events) ? lastObserve.value.events : [];
    ready = events.some((event) => event?.kind === 'session/ready');
    if (!ready) await new Promise((resolvePromise) => setTimeout(resolvePromise, 250));
  }
  check(report, 'lifecycle:ready', ready ? 'pass' : 'fail', {
    status_code: lastObserve?.response?.status || opened.response.status,
    open_state: opened.value?.open_state || null,
    observed_ready_event: ready,
    response: lastObserve?.value || lastObserve?.text?.slice(0, 2_000) || null,
  });
  if (!ready) return;

  const turn = await request('POST', '/api/remote/turn', {
    agent_session_id: sessionId,
    idempotency_key: `${id}-turn`,
    input: { text: 'cancel this turn after admission' },
  });
  check(report, 'lifecycle:turn', turn.response.ok ? 'pass' : 'fail', {
    status_code: turn.response.status,
    response: turn.value || turn.text.slice(0, 2_000),
  });
  if (!turn.response.ok) return;

  const observe = await request(
    'GET',
    `/api/remote/observe?agent_session_id=${encodeURIComponent(sessionId)}&after_seq=${cursor}&limit=100`,
  );
  check(report, 'lifecycle:observe', observe.response.ok ? 'pass' : 'fail', {
    status_code: observe.response.status,
    response: observe.value || observe.text.slice(0, 2_000),
  });
  if (!observe.response.ok) return;
  cursor = Number(observe.value?.next_cursor?.seq || cursor);

  const cancel = await request('POST', '/api/remote/cancel', {
    agent_session_id: sessionId,
    idempotency_key: `${id}-cancel`,
  });
  check(report, 'lifecycle:cancel', cancel.response.ok ? 'pass' : 'fail', {
    status_code: cancel.response.status,
    response: cancel.value || cancel.text.slice(0, 2_000),
  });
  if (!cancel.response.ok) return;

  const dispose = await request('DELETE', `/api/agent-sessions/${encodeURIComponent(sessionId)}`);
  check(report, 'lifecycle:dispose', dispose.response.ok ? 'pass' : 'fail', {
    status_code: dispose.response.status,
    response: dispose.value || dispose.text.slice(0, 2_000),
  });
}

async function sidecarRpc(options, sidecar, report, hello) {
  const credential = readFileSync(options.credentialFile);
  if (credential.length === 0) {
    check(report, 'sidecar:credential', 'fail', { reason: 'credential file is empty' });
    return;
  }
  const child = spawn(sidecar, ['app-server', '--listen', 'stdio://'], {
    cwd: dirname(sidecar),
    stdio: ['pipe', 'pipe', 'pipe', 'pipe'],
  });
  let stdout = '';
  let stderr = '';
  child.stdout.on('data', (chunk) => {
    stdout += String(chunk);
  });
  child.stderr.on('data', (chunk) => {
    stderr += String(chunk);
  });
  try {
    child.stdio[3].write(Buffer.from('NOMIFUN-CODEX-CREDENTIAL-V1\0'));
    const frame = Buffer.alloc(4);
    frame.writeUInt32BE(credential.length);
    child.stdio[3].write(frame);
    child.stdio[3].write(credential);
    child.stdio[3].end();
    child.stdin.write(`${JSON.stringify({
      id: 1,
      method: 'runtime/hello',
      params: {
        credential_protocol: 'nomifun-inherited-handle-v1',
        credential_handle: { kind: 'unix_fd', fd: 3 },
      },
    })}\n`);
    child.stdin.end();
    const deadline = Date.now() + 10_000;
    while (!stdout.includes('\n') && Date.now() < deadline) {
      await new Promise((resolvePromise) => setTimeout(resolvePromise, 50));
    }
    const line = stdout.split(/\r?\n/).find(Boolean);
    const actual = line ? JSON.parse(line) : null;
    const payload = actual?.result;
    const matches =
      actual?.id === 1 &&
      payload &&
      payload.runtime_release_digest === hello.runtime_release_digest &&
      payload.fork_commit === EXPECTED_FORK_COMMIT &&
      payload.tracked_upstream_commit === EXPECTED_FORK_COMMIT &&
      payload.protocol_version === EXPECTED_PROTOCOL_VERSION &&
      JSON.stringify(payload.rpc_allowlist?.methods || []) === JSON.stringify(EXPECTED_RPC_METHODS) &&
      Array.isArray(payload.rpc_allowlist?.experimental_methods) &&
      payload.rpc_allowlist.experimental_methods.length === 0;
    check(report, 'sidecar:live_hello_rpc', matches ? 'pass' : 'fail', {
      observed_id: actual?.id ?? null,
      observed_hello: payload || null,
      stderr_tail: stderr.slice(-2_000),
    });
  } catch (error) {
    check(report, 'sidecar:live_hello_rpc', 'fail', { reason: error.message, stderr_tail: stderr.slice(-2_000) });
  } finally {
    await stopChild(child);
    await new Promise((resolvePromise) => setTimeout(resolvePromise, 250));
    const remaining = descendantsOf(child.pid);
    check(report, 'sidecar:process_cleanup', remaining.pids.length === 0 ? 'pass' : 'fail', {
      root_pid: child.pid,
      remaining_pids: remaining.pids,
    });
  }
}

export function validatePathShape(path, { kind = 'file', requireExecutable = false } = {}) {
  const metadata = existingFile(path);
  if (!metadata) return { status: 'blocked', reason: 'missing', path };
  if (metadata.isSymbolicLink()) {
    return { status: 'fail', reason: 'symlink_not_allowed', path };
  }
  if (kind === 'file' && !metadata.isFile()) return { status: 'fail', reason: 'not_regular_file', path };
  if (kind === 'directory' && !metadata.isDirectory()) return { status: 'fail', reason: 'not_directory', path };
  const casing = exactCasePath(path);
  if (casing.status !== 'pass') return { status: 'fail', reason: 'path_case_mismatch', path, casing };
  if (requireExecutable && !isExecutableMode(metadata.mode)) {
    return { status: 'fail', reason: 'permissions_not_executable_or_writable', path, mode: metadata.mode.toString(8) };
  }
  return { status: 'pass', path, mode: metadata.mode.toString(8) };
}

export function validateHelloPayload(hello) {
  const mismatches = [];
  if (!SHA256_PATTERN.test(hello?.runtime_release_digest || '')) {
    mismatches.push('runtime_release_digest');
  }
  if (!SHA256_PATTERN.test(hello?.runtime_build_digest || '')) {
    mismatches.push('runtime_build_digest');
  }
  if (hello?.fork_commit !== EXPECTED_FORK_COMMIT) mismatches.push('fork_commit');
  if (hello?.tracked_upstream_commit !== EXPECTED_FORK_COMMIT) mismatches.push('tracked_upstream_commit');
  if (hello?.protocol_version !== EXPECTED_PROTOCOL_VERSION) mismatches.push('protocol_version');
  if (hello?.protocol_schema_digest !== EXPECTED_PROTOCOL_SCHEMA_DIGEST) mismatches.push('protocol_schema_digest');
  if (hello?.runtime_target !== EXPECTED_TARGET) mismatches.push('runtime_target');
  if (JSON.stringify(hello?.supported_profiles || []) !== JSON.stringify(EXPECTED_PROFILES)) mismatches.push('supported_profiles');
  if (hello?.full_auto?.ask_for_approval !== 'never') mismatches.push('full_auto.ask_for_approval');
  if (hello?.full_auto?.sandbox_policy !== 'danger-full-access') mismatches.push('full_auto.sandbox_policy');
  if (JSON.stringify(hello?.rpc_allowlist?.methods || []) !== JSON.stringify(EXPECTED_RPC_METHODS)) mismatches.push('rpc_allowlist.methods');
  if (!Array.isArray(hello?.rpc_allowlist?.experimental_methods) || hello.rpc_allowlist.experimental_methods.length !== 0) {
    mismatches.push('rpc_allowlist.experimental_methods');
  }
  return { status: mismatches.length === 0 ? 'pass' : 'fail', mismatches };
}

export async function runValidation(options = parseArgs(process.argv.slice(2))) {
  const report = {
    schema_version: '1.0.0',
    source_commit: null,
    platform: null,
    target: TARGET_ID,
    suite: {
      name: 'macos-arm64-native',
      checks: [],
    },
    status: 'blocked',
    release_lock: null,
    logs: [{ kind: 'embedded_checks', reference: '#/checks' }],
    gate_name: 'c8-ma-macos-arm64-helper',
    execution_kind: 'native',
    target_cell: TARGET_ID,
    execution_host: {
      platform: process.platform,
      arch: process.arch,
      uname: command('uname', ['-s', '-m']),
      translated: command('sysctl', ['-in', 'sysctl.proc_translated']),
      rustc: command('rustc', ['-Vv']),
    },
    checks: [],
    failures: [],
    blockers: [],
    artifacts: {},
  };
  const finish = () => {
    for (const path of options.logs || []) logReference(report, path);
    return finishReport(report, options);
  };

  if (process.platform !== 'darwin' || process.arch !== 'arm64') {
    check(report, 'native-host', 'fail', {
      reason: 'requires native macOS arm64',
      observed: { platform: process.platform, arch: process.arch },
    });
  } else {
    const translated = report.execution_host.translated.stdout.trim();
    const rustcHost = report.execution_host.rustc.stdout.match(/^host:\s*(.+)$/m)?.[1]?.trim();
    check(report, 'native-host', translated !== '1' && rustcHost === EXPECTED_TARGET ? 'pass' : 'fail', {
      expected: { platform: 'darwin', arch: 'arm64', translated: '0', rustc_host: EXPECTED_TARGET },
      observed: { platform: process.platform, arch: process.arch, translated, rustc_host: rustcHost || null },
    });
  }

  if (!options.releaseLock) {
    check(report, 'release-lock', 'blocked', {
      reason: 'A real release-lock.json is required; provide --release-lock or NOMIFUN_RELEASE_LOCK_PATH',
    });
    report.blockers.push('missing release-lock.json');
    return finish();
  }

  const artifactRoot = resolve(options.artifactRoot || REPO_ROOT);
  const release = readAndVerifyReleaseLock(options.releaseLock, { root: artifactRoot });
  report.release_lock = {
    path: release.lock_path || resolve(options.releaseLock),
    ...(release.lock_sha256 ? { sha256: release.lock_sha256 } : {}),
  };
  if (release.lock) {
    report.source_commit = release.lock.source_commit || null;
    report.platform = release.lock.platform || null;
  }
  check(report, 'release-lock:real-artifacts', release.status, {
    reason: release.reason || null,
    lock_path: report.release_lock.path,
    artifact_root: artifactRoot,
    artifact_checks: release.checks,
  });
  if (release.status !== 'pass') {
    if (release.status === 'blocked') report.blockers.push(release.reason || 'release lock blocked');
    return finish();
  }

  const lock = release.lock;
  const supportedPlatform =
    lock.platform === EXPECTED_TARGET || lock.platform === 'universal-apple-darwin';
  check(report, 'release-lock:platform', supportedPlatform ? 'pass' : 'fail', {
    expected: [EXPECTED_TARGET, 'universal-apple-darwin'],
    observed: lock.platform,
  });

  const worktree = command('git', ['status', '--porcelain', '--untracked-files=no']);
  const head = command('git', ['rev-parse', 'HEAD']);
  const trackedDirty = worktree.status === 0 && worktree.stdout.trim().length > 0;
  const observedHead = head.status === 0 ? head.stdout.trim() : null;
  check(
    report,
    'release-lock:source-commit',
    worktree.status !== 0 || head.status !== 0
      ? 'blocked'
      : trackedDirty || observedHead !== lock.source_commit
        ? 'fail'
        : 'pass',
    {
      expected: lock.source_commit,
      observed: observedHead,
      tracked_worktree_dirty: trackedDirty,
      status_error: worktree.stderr.trim() || null,
      head_error: head.stderr.trim() || null,
    },
  );

  const lockedSidecar = lock.sidecars[TARGET_ID];
  if (!lockedSidecar) {
    check(report, 'release-lock:arm64-sidecar', 'blocked', {
      reason: `release lock has no ${TARGET_ID} sidecar`,
      available_targets: Object.keys(lock.sidecars),
    });
    report.blockers.push(`release lock has no ${TARGET_ID} sidecar`);
    return finish();
  }

  let lockedHostPath;
  let lockedPackagePath;
  let lockedSidecarPath;
  try {
    lockedHostPath = resolveReleaseArtifactPath(artifactRoot, lock.host.path);
    lockedPackagePath = resolveReleaseArtifactPath(artifactRoot, lock.package.path);
    lockedSidecarPath = resolveReleaseArtifactPath(artifactRoot, lockedSidecar.path);
  } catch (error) {
    check(report, 'release-lock:artifact-paths', 'fail', { reason: error.message });
    return finish();
  }

  const lockedAppPath = appFromHostBinary(lockedHostPath);
  checkOptionalArtifactOverride(report, 'override:app', options.app, lockedAppPath);
  checkOptionalArtifactOverride(report, 'override:dmg', options.dmg, lockedPackagePath);
  checkOptionalArtifactOverride(report, 'override:sidecar', options.sidecar, lockedSidecarPath);
  if (options.sidecarDir) {
    checkOptionalArtifactOverride(
      report,
      'override:sidecar-dir',
      join(resolve(options.sidecarDir), 'runtime/macos/arm64/nomifun-codex-runtime'),
      lockedSidecarPath,
    );
  }

  const appPath = lockedAppPath;
  report.artifacts.app = appPath;
  if (appPath) {
    const appShape = validatePathShape(appPath, { kind: 'directory' });
    check(report, 'macos-app:path-case-permissions', appShape.status === 'pass' ? 'pass' : 'fail', appShape);
    const executable = join(appPath, 'Contents/MacOS/nomifun-desktop');
    const executableShape = validatePathShape(executable, { requireExecutable: true });
    check(
      report,
      'macos-app:locked-host',
      executableShape.status === 'pass' && executable === lockedHostPath ? 'pass' : 'fail',
      {
        ...executableShape,
        expected_from_release_lock: lockedHostPath,
      },
    );
    if (executableShape.status === 'pass') {
      const archs = command('lipo', ['-archs', executable]);
      const observedArchs = archs.stdout.trim().split(/\s+/).filter(Boolean).sort();
      const expectedArchs = lock.platform === 'universal-apple-darwin'
        ? ['arm64', 'x86_64']
        : ['arm64'];
      const architectureMatches = expectedArchs.every((arch) => observedArchs.includes(arch)) &&
        (lock.platform === 'universal-apple-darwin' || observedArchs.length === 1);
      check(report, 'macos-app:architectures', architectureMatches ? 'pass' : 'fail', {
        expected: expectedArchs,
        observed: observedArchs,
        stderr: archs.stderr.trim(),
      });
      const signature = command('codesign', ['--verify', '--deep', '--strict', '--verbose=2', appPath], 120_000);
      check(report, 'macos-app:codesign', signature.status === 0 ? 'pass' : 'fail', {
        exit_code: signature.status,
        stdout_tail: signature.stdout.slice(-2_000),
        stderr_tail: signature.stderr.slice(-2_000),
      });
    }
  } else {
    check(report, 'macos-app:artifact', 'blocked', {
      reason: 'release-lock host path is not inside NomiFun.app/Contents/MacOS',
      host_path: lockedHostPath,
    });
    report.blockers.push('release-lock host path does not identify a macOS app');
  }

  const dmgPath = lockedPackagePath;
  report.artifacts.dmg = dmgPath;
  const dmgShape = validatePathShape(dmgPath);
  check(report, 'macos-package:path-case-permissions', dmgShape.status, dmgShape);
  if (dmgShape.status === 'pass') {
    const verify = command('hdiutil', ['verify', dmgPath], 120_000);
    check(report, 'macos-package:hdiutil-verify', verify.status === 0 ? 'pass' : 'fail', {
      command: verify.command,
      exit_code: verify.status,
      stdout_tail: verify.stdout.slice(-2_000),
      stderr_tail: verify.stderr.slice(-2_000),
    });
  }

  report.artifacts.sidecar = lockedSidecarPath;
  const sidecarShape = validatePathShape(lockedSidecarPath, { requireExecutable: true });
  check(report, 'sidecar:artifact-target-permissions', sidecarShape.status, sidecarShape);
  if (sidecarShape.status === 'pass') {
    const observedSha = sha256File(lockedSidecarPath);
    check(report, 'sidecar:release-lock-sha256', observedSha === lockedSidecar.sha256 ? 'pass' : 'fail', {
      expected: lockedSidecar.sha256,
      observed: observedSha,
      path: lockedSidecarPath,
    });
    const fileType = command('file', [lockedSidecarPath]);
    const archs = command('lipo', ['-archs', lockedSidecarPath]);
    check(report, 'sidecar:native-arm64', fileType.stdout.includes('arm64') && archs.stdout.trim() === 'arm64' ? 'pass' : 'fail', {
      file: fileType.stdout.trim(),
      lipo_archs: archs.stdout.trim(),
    });

    const helloPath = options.hello || `${lockedSidecarPath}.hello.json`;
    report.artifacts.hello = helloPath;
    try {
      const helloShape = validatePathShape(helloPath);
      if (helloShape.status !== 'pass') {
        throw new Error(`hello metadata path rejected: ${JSON.stringify(helloShape)}`);
      }
      const hello = readJson(helloPath);
      const helloResult = validateHelloPayload(hello);
      check(report, 'sidecar:hello-profile-rpc-contract', helloResult.status, {
        mismatches: helloResult.mismatches,
        path: helloPath,
      });
      if (options.runSidecarRpc) await sidecarRpc(options, lockedSidecarPath, report, hello);
      else check(report, 'sidecar:live-hello-rpc', 'blocked', {
        reason: 'live hello/RPC probe not run; rerun with --run-sidecar-rpc --credential-file <path>',
      });
    } catch (error) {
      check(report, 'sidecar:hello-profile-rpc-contract', 'blocked', {
        reason: `hello metadata unavailable or invalid: ${error.message}`,
        path: helloPath,
      });
      report.blockers.push(`missing/invalid hello metadata: ${helloPath}`);
    }
  }

  const hostBinary = options.hostBinary || join(REPO_ROOT, 'target/debug/nomicore');
  if (options.runStartup || existingFile(hostBinary)?.isFile()) {
    const rootParent = mkdtempSync(join(tmpdir(), 'nomifun-c8-ma-'));
    try {
      const absentRoot = join(rootParent, 'absent-root');
      const emptyRoot = join(rootParent, 'precreated-empty-root');
      mkdirSync(emptyRoot);
      await startupSmoke(hostBinary, absentRoot, report, 'startup:absent-root');
      await startupSmoke(hostBinary, emptyRoot, report, 'startup:precreated-empty-root');
    } finally {
      rmSync(rootParent, { recursive: true, force: true });
    }
  } else {
    check(report, 'startup:host-binary', 'blocked', {
      reason: 'nomicore host binary not found; provide --host-binary or build target/debug/nomicore',
      path: hostBinary,
    });
    report.blockers.push(`missing host binary: ${hostBinary}`);
  }

  if (options.runLifecycle) {
    await remoteLifecycle(options, report);
  } else {
    check(report, 'lifecycle:open-ready-turn-observe-cancel-dispose', 'blocked', {
      reason: 'live lifecycle not run; provide --endpoint and --binding-id --run-lifecycle',
    });
  }

  return finish();
}

export function assertSelfTest() {
  const temporary = mkdtempSync(join(tmpdir(), 'nomifun-validation-test-'));
  try {
    const file = join(temporary, 'ExactName');
    writeFileSync(file, 'fixture');
    chmodSync(file, 0o755);
    const valid = validatePathShape(file, {
      requireExecutable: process.platform !== 'win32',
    });
    if (valid.status !== 'pass') throw new Error(`path fixture should pass: ${JSON.stringify(valid)}`);
    if (
      !isExecutableMode(0o100755) ||
      isExecutableMode(0o100644) ||
      isExecutableMode(0o100777)
    ) {
      throw new Error('executable mode predicate must require execute bits and reject writable artifacts');
    }
    const wrongCase = validatePathShape(join(temporary, 'exactname'));
    if (wrongCase.status !== 'fail' || wrongCase.reason !== 'path_case_mismatch') {
      throw new Error('wrong path casing must fail closed');
    }
    const linkTarget = join(temporary, 'link-target');
    mkdirSync(linkTarget);
    const link = join(temporary, 'link');
    symlinkSync(linkTarget, link, process.platform === 'win32' ? 'junction' : 'dir');
    const symlink = validatePathShape(link, { kind: 'directory' });
    if (symlink.reason !== 'symlink_not_allowed') throw new Error('symlink must fail closed');
    const hello = validateHelloPayload({
      runtime_release_digest: 'a'.repeat(64),
      runtime_build_digest: 'b'.repeat(64),
      fork_commit: EXPECTED_FORK_COMMIT,
      tracked_upstream_commit: EXPECTED_FORK_COMMIT,
      protocol_version: EXPECTED_PROTOCOL_VERSION,
      protocol_schema_digest: EXPECTED_PROTOCOL_SCHEMA_DIGEST,
      runtime_target: EXPECTED_TARGET,
      supported_profiles: EXPECTED_PROFILES,
      full_auto: { ask_for_approval: 'never', sandbox_policy: 'danger-full-access' },
      rpc_allowlist: { methods: EXPECTED_RPC_METHODS, experimental_methods: [] },
    });
    if (hello.status !== 'pass') throw new Error(`hello fixture should pass: ${JSON.stringify(hello)}`);
    return { status: 'pass' };
  } finally {
    rmSync(temporary, { recursive: true, force: true });
  }
}

const isMain = process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url);
if (isMain) {
  try {
    const options = parseArgs(process.argv.slice(2));
    if (options.selfTest) {
      console.log(JSON.stringify(assertSelfTest()));
      process.exit(0);
    }
    const report = await runValidation(options);
    console.log(JSON.stringify(report, null, 2));
    process.exit(report.status === 'pass' ? 0 : report.status === 'blocked' ? 3 : 1);
  } catch (error) {
    console.error(`macOS arm64 validation helper error: ${error.message}`);
    process.exit(2);
  }
}
