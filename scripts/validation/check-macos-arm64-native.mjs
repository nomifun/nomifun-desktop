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
 *   bun scripts/validation/check-macos-arm64-native.mjs
 *   bun scripts/validation/check-macos-arm64-native.mjs --sidecar /abs/runtime
 *
 * Optional live checks:
 *   --host-binary /abs/nomicore --run-startup
 *   --endpoint http://127.0.0.1:25808 --binding-id <id> --run-lifecycle
 *   --credential-file /abs/credential --run-sidecar-rpc
 */

import { spawn, spawnSync } from 'node:child_process';
import { createHash } from 'node:crypto';
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

export const REPO_ROOT = resolve(dirname(fileURLToPath(import.meta.url)), '../..');
export const TARGET_ID = 'macos_desktop_arm64';
export const EXPECTED_TARGET = 'aarch64-apple-darwin';
export const EXPECTED_SIDECAR_SHA256 =
  '7863db3a77545eec8966483f26fb5b493aea6e285ac35b5c29d0920342438060';
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
export const EXPECTED_RUNTIME_RELEASE_DIGEST =
  '7c0c297dd0dd7c11c71cd589965e930ddec0008bebaaf510eabcd0c597358838';

const RUNTIME_INPUT = join(
  REPO_ROOT,
  'crates/backend/nomifun-agent-contracts/contracts/runtime/codex-runtime-release-input.json',
);
const PLATFORM_INPUT = join(
  REPO_ROOT,
  'crates/backend/nomifun-agent-contracts/contracts/validation/platform-validation-manifest.payload.json',
);
const APP_CANDIDATES = [
  join(REPO_ROOT, 'target/universal-apple-darwin/release/bundle/macos/NomiFun.app'),
  join(REPO_ROOT, 'target/aarch64-apple-darwin/release/bundle/macos/NomiFun.app'),
  join(REPO_ROOT, 'target/x86_64-apple-darwin/release/bundle/macos/NomiFun.app'),
];
const DMG_CANDIDATES = [
  join(REPO_ROOT, 'target/universal-apple-darwin/release/bundle/dmg'),
  join(REPO_ROOT, 'target/aarch64-apple-darwin/release/bundle/dmg'),
  join(REPO_ROOT, 'target/x86_64-apple-darwin/release/bundle/dmg'),
  join(REPO_ROOT, 'dist/desktop'),
];

function parseArgs(argv) {
  const options = {
    sidecar: process.env.NOMIFUN_CODEX_RUNTIME_PATH || null,
    hello: process.env.NOMIFUN_CODEX_RUNTIME_HELLO_PATH || null,
    sidecarDir: process.env.NOMIFUN_CODEX_RUNTIME_DIR || null,
    app: null,
    dmg: null,
    hostBinary: null,
    endpoint: null,
    bindingId: null,
    token: process.env.NOMIFUN_ACCESS_TOKEN || null,
    credentialFile: null,
    report: null,
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
    };
    if (!(key in mapping)) throw new Error(`unknown argument: ${token}`);
    options[mapping[key]] = value;
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

function sha256File(path) {
  const hash = createHash('sha256');
  hash.update(readFileSync(path));
  return hash.digest('hex');
}

function readJson(path) {
  return JSON.parse(readFileSync(path, 'utf8'));
}

function check(report, id, status, details = {}) {
  report.checks.push({ id, status, ...details });
  if (status === 'fail' || status === 'blocked') report.failures.push({ id, ...details });
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

function findSidecar(options, appPath) {
  const candidates = [];
  if (options.sidecar) candidates.push(resolve(options.sidecar));
  if (options.sidecarDir) {
    candidates.push(join(resolve(options.sidecarDir), 'runtime/macos/arm64/nomifun-codex-runtime'));
  }
  if (appPath) {
    candidates.push(join(appPath, 'Contents/Resources/runtime/macos/arm64/nomifun-codex-runtime'));
  }
  candidates.push(
    join(REPO_ROOT, 'target/universal-apple-darwin/release/bundle/macos/NomiFun.app/Contents/Resources/runtime/macos/arm64/nomifun-codex-runtime'),
    join(REPO_ROOT, 'target/aarch64-apple-darwin/release/bundle/macos/NomiFun.app/Contents/Resources/runtime/macos/arm64/nomifun-codex-runtime'),
  );
  const unique = [...new Set(candidates)];
  return {
    candidates: unique,
    selected: unique.find((candidate) => existingFile(candidate)),
  };
}

function findApp(options) {
  if (options.app) return resolve(options.app);
  return APP_CANDIDATES.find((candidate) => existingFile(candidate)?.isDirectory()) || null;
}

function findDmg(options) {
  if (options.dmg) return resolve(options.dmg);
  for (const directory of DMG_CANDIDATES) {
    if (!existingFile(directory)?.isDirectory()) continue;
    const file = readdirSync(directory)
      .filter((name) => name.toLowerCase().endsWith('.dmg'))
      .sort()
      .map((name) => join(directory, name))
      .find((candidate) => existingFile(candidate)?.isFile());
    if (file) return file;
  }
  return null;
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

export function validateHelloPayload(hello, expectedRuntimeReleaseDigest = EXPECTED_RUNTIME_RELEASE_DIGEST) {
  const mismatches = [];
  if (hello?.runtime_release_digest !== expectedRuntimeReleaseDigest) mismatches.push('runtime_release_digest');
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

  let runtimeInput = null;
  try {
    runtimeInput = readJson(RUNTIME_INPUT);
    const target = runtimeInput.target_matrix?.[TARGET_ID];
    check(report, 'frozen-runtime-input', target?.sidecar_artifact?.digest === EXPECTED_SIDECAR_SHA256 &&
      target?.runtime_target === EXPECTED_TARGET ? 'pass' : 'fail', {
      expected: { sidecar_sha256: EXPECTED_SIDECAR_SHA256, runtime_target: EXPECTED_TARGET },
      observed: {
        sidecar_sha256: target?.sidecar_artifact?.digest || null,
        runtime_target: target?.runtime_target || null,
      },
    });
  } catch (error) {
    check(report, 'frozen-runtime-input', 'fail', { reason: error.message, path: RUNTIME_INPUT });
  }
  try {
    const platform = readJson(PLATFORM_INPUT);
    const target = platform.platform_matrix?.target_cells?.[TARGET_ID];
    check(report, 'frozen-platform-input', target?.host_target === EXPECTED_TARGET &&
      target?.runtime_target === EXPECTED_TARGET ? 'pass' : 'fail', {
      expected: { host_target: EXPECTED_TARGET, runtime_target: EXPECTED_TARGET },
      observed: target || null,
    });
  } catch (error) {
    check(report, 'frozen-platform-input', 'fail', { reason: error.message, path: PLATFORM_INPUT });
  }

  const appPath = findApp(options);
  report.artifacts.app = appPath;
  if (appPath) {
    const appShape = validatePathShape(appPath, { kind: 'directory' });
    check(report, 'universal-app:path-case-permissions', appShape.status === 'pass' ? 'pass' : 'fail', appShape);
    const executable = join(appPath, 'Contents/MacOS/nomifun-desktop');
    const executableShape = validatePathShape(executable, { requireExecutable: true });
    check(report, 'universal-app:executable', executableShape.status === 'pass' ? 'pass' : 'fail', executableShape);
    if (executableShape.status === 'pass') {
      const archs = command('lipo', ['-archs', executable]);
      const observedArchs = archs.stdout.trim().split(/\s+/).filter(Boolean).sort();
      check(report, 'universal-app:architectures', observedArchs.includes('arm64') && observedArchs.includes('x86_64') ? 'pass' : 'fail', {
        expected: ['arm64', 'x86_64'],
        observed: observedArchs,
        stderr: archs.stderr.trim(),
      });
      const signature = command('codesign', ['--verify', '--deep', '--strict', '--verbose=2', appPath], 120_000);
      check(report, 'universal-app:codesign', signature.status === 0 ? 'pass' : 'fail', {
        exit_code: signature.status,
        stdout_tail: signature.stdout.slice(-2_000),
        stderr_tail: signature.stderr.slice(-2_000),
      });
    }
  } else {
    check(report, 'universal-app:artifact', 'blocked', {
      reason: 'No macOS app artifact found; provide --app /absolute/path/to/NomiFun.app',
      candidates: APP_CANDIDATES,
    });
    report.blockers.push('missing macOS app artifact');
  }

  const dmgPath = findDmg(options);
  report.artifacts.dmg = dmgPath;
  if (dmgPath) {
    const dmgShape = validatePathShape(dmgPath);
    check(report, 'universal-dmg:path-case-permissions', dmgShape.status === 'pass' ? 'pass' : 'fail', dmgShape);
    if (dmgShape.status === 'pass') {
      const verify = command('hdiutil', ['verify', dmgPath], 120_000);
      check(report, 'universal-dmg:hdiutil-verify', verify.status === 0 ? 'pass' : 'fail', {
        command: verify.command,
        exit_code: verify.status,
        stdout_tail: verify.stdout.slice(-2_000),
        stderr_tail: verify.stderr.slice(-2_000),
      });
    }
  } else {
    check(report, 'universal-dmg:artifact', 'blocked', {
      reason: 'Universal DMG not found; provide --dmg /absolute/path/to/NomiFun_universal.dmg',
      candidates: DMG_CANDIDATES,
    });
    report.blockers.push('missing Universal DMG artifact');
  }

  const sidecar = findSidecar(options, appPath);
  report.artifacts.sidecar_candidates = sidecar.candidates;
  report.artifacts.sidecar = sidecar.selected || null;
  if (!sidecar.selected) {
    const blocker = {
      code: 'MACOS_ARM64_SIDECAR_MISSING',
      expected_sha256: EXPECTED_SIDECAR_SHA256,
      expected_target: EXPECTED_TARGET,
      logical_path: 'runtime/macos/arm64/nomifun-codex-runtime',
      remediation: 'provide the real signed/pinned arm64 sidecar via --sidecar or NOMIFUN_CODEX_RUNTIME_PATH',
      searched: sidecar.candidates,
    };
    check(report, 'sidecar:artifact-sha-target-permissions', 'blocked', blocker);
    report.blockers.push(blocker);
  } else {
    const sidecarShape = validatePathShape(sidecar.selected, { requireExecutable: true });
    check(report, 'sidecar:artifact-sha-target-permissions', sidecarShape.status === 'pass' ? 'pass' : 'fail', sidecarShape);
    if (sidecarShape.status === 'pass') {
      const observedSha = sha256File(sidecar.selected);
      check(report, 'sidecar:sha256', observedSha === EXPECTED_SIDECAR_SHA256 ? 'pass' : 'fail', {
        expected: EXPECTED_SIDECAR_SHA256,
        observed: observedSha,
        path: sidecar.selected,
      });
      const fileType = command('file', [sidecar.selected]);
      const archs = command('lipo', ['-archs', sidecar.selected]);
      check(report, 'sidecar:native-arm64', fileType.stdout.includes('arm64') && archs.stdout.trim() === 'arm64' ? 'pass' : 'fail', {
        file: fileType.stdout.trim(),
        lipo_archs: archs.stdout.trim(),
      });

      const helloPath = options.hello || `${sidecar.selected}.hello.json`;
      report.artifacts.hello = helloPath;
      try {
        const helloShape = validatePathShape(helloPath);
        if (helloShape.status !== 'pass') {
          throw new Error(`hello metadata path rejected: ${JSON.stringify(helloShape)}`);
        }
        const hello = readJson(helloPath);
        const helloResult = validateHelloPayload(hello, EXPECTED_RUNTIME_RELEASE_DIGEST);
        check(report, 'sidecar:hello-profile-rpc-contract', helloResult.status, {
          mismatches: helloResult.mismatches,
          path: helloPath,
        });
        if (options.runSidecarRpc) await sidecarRpc(options, sidecar.selected, report, hello);
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

  report.status = report.failures.some((failure) => report.checks.find((entry) => entry.id === failure.id)?.status === 'fail')
    ? 'fail'
    : report.failures.length > 0
      ? 'blocked'
      : 'pass';
  if (options.report) {
    mkdirSync(dirname(resolve(options.report)), { recursive: true });
    writeFileSync(resolve(options.report), `${JSON.stringify(report, null, 2)}\n`);
  }
  return report;
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
      runtime_release_digest: EXPECTED_RUNTIME_RELEASE_DIGEST,
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
