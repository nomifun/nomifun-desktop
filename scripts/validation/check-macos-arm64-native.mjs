#!/usr/bin/env node

/**
 * Target-specific macOS arm64 engineering preflight (not full CAR acceptance).
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
 * Optional Nomi-core live checks:
 *   --host-binary /abs/nomicore --run-startup
 *   --endpoint http://127.0.0.1:25808 --binding-id <id> --run-lifecycle
 *
 * Engines are compiled into the host. External executor and credential-file
 * probe options are retired and rejected before reading artifacts or running tools.
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
export const CANONICAL_CAPABILITY_INVENTORY_RELATIVE_PATH =
  'crates/backend/nomifun-agent-contracts/contracts/generated/target-first-party-contributions.envelope.json';

const RETIRED_EXECUTOR_OPTIONS = Object.freeze({
  sidecar: '--sidecar',
  hello: '--hello',
  sidecarDir: '--sidecar-dir',
  credentialFile: '--credential-file',
  runSidecarRpc: '--run-sidecar-rpc',
});

function rejectRetiredOptions(options) {
  for (const [key, flag] of Object.entries(RETIRED_EXECUTOR_OPTIONS)) {
    if (Object.hasOwn(options, key)) {
      throw new Error(`${flag} was retired; Engines are compiled into the host`);
    }
  }
}

export function parseArgs(argv) {
  const options = {
    releaseLock: process.env.NOMIFUN_RELEASE_LOCK_PATH || null,
    artifactRoot: process.env.NOMIFUN_RELEASE_ARTIFACT_ROOT || REPO_ROOT,
    capabilityInventory:
      process.env.NOMIFUN_CAPABILITY_INVENTORY_PATH ||
      join(REPO_ROOT, CANONICAL_CAPABILITY_INVENTORY_RELATIVE_PATH),
    app: null,
    dmg: null,
    hostBinary: null,
    endpoint: null,
    bindingId: null,
    token: process.env.NOMIFUN_ACCESS_TOKEN || null,
    report: null,
    logs: [],
    runStartup: false,
    runLifecycle: false,
    selfTest: false,
  };
  for (let index = 0; index < argv.length; index += 1) {
    const token = argv[index];
    const flag = token.split('=', 1)[0];
    if (Object.values(RETIRED_EXECUTOR_OPTIONS).includes(flag)) {
      throw new Error(`${flag} was retired; Engines are compiled into the host`);
    }
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
    const match = token.match(/^--([^=]+)(?:=(.*))?$/);
    if (!match) throw new Error(`unknown argument: ${token}`);
    const key = match[1].replaceAll('-', '');
    let value = match[2];
    if (value === undefined) {
      value = argv[++index];
      if (!value || value.startsWith('--')) throw new Error(`${token} requires a value`);
    }
    const mapping = {
      app: 'app',
      dmg: 'dmg',
      hostbinary: 'hostBinary',
      endpoint: 'endpoint',
      bindingid: 'bindingId',
      token: 'token',
      report: 'report',
      releaselock: 'releaseLock',
      artifactroot: 'artifactRoot',
      capabilityinventory: 'capabilityInventory',
      log: 'logs',
    };
    if (!(key in mapping)) throw new Error(`unknown argument: ${token}`);
    if (mapping[key] === 'logs') options.logs.push(value);
    else options[mapping[key]] = value;
  }
  if (options.runLifecycle && (!options.endpoint || !options.bindingId)) {
    throw new Error('--run-lifecycle requires --endpoint and --binding-id');
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

function canonicalInventoryPayload(artifact) {
  const payload = artifact?.payload ?? artifact;
  if (!payload || typeof payload !== 'object' || Array.isArray(payload)) {
    throw new Error('canonical capability inventory payload must be an object');
  }
  return payload;
}

export function readCanonicalCapabilityInventory(
  path = join(REPO_ROOT, CANONICAL_CAPABILITY_INVENTORY_RELATIVE_PATH),
) {
  const absolutePath = resolve(path);
  const artifact = readJson(absolutePath);
  const payload = canonicalInventoryPayload(artifact);
  if (!Array.isArray(payload.packages) || payload.packages.length === 0) {
    throw new Error('canonical capability inventory must contain a non-empty packages array');
  }

  const ids = [];
  for (const [packageIndex, packageEntry] of payload.packages.entries()) {
    if (!Array.isArray(packageEntry?.capabilities)) {
      throw new Error(
        `canonical capability inventory package ${packageIndex} must contain a capabilities array`,
      );
    }
    for (const [capabilityIndex, capabilityEntry] of packageEntry.capabilities.entries()) {
      const id = capabilityEntry?.capability?.id;
      if (typeof id !== 'string' || id.length === 0) {
        throw new Error(
          `canonical capability inventory entry ${packageIndex}:${capabilityIndex} has no capability id`,
        );
      }
      ids.push(id);
    }
  }

  const uniqueIds = new Set(ids);
  if (uniqueIds.size !== ids.length) {
    const duplicates = [...new Set(ids.filter((id, index) => ids.indexOf(id) !== index))].sort();
    throw new Error(
      `canonical capability inventory contains duplicate ids: ${duplicates.join(', ')}`,
    );
  }

  return {
    path: absolutePath,
    payloadDigest: typeof artifact?.payload_digest === 'string' ? artifact.payload_digest : null,
    packageCount: payload.packages.length,
    capabilityIds: new Set([...uniqueIds].sort()),
  };
}

export function readCanonicalCapabilityIds(
  path = join(REPO_ROOT, CANONICAL_CAPABILITY_INVENTORY_RELATIVE_PATH),
) {
  return readCanonicalCapabilityInventory(path).capabilityIds;
}

export function compareCapabilityInventory(body, canonicalInventory) {
  const expectedIds =
    canonicalInventory instanceof Set
      ? canonicalInventory
      : canonicalInventory?.capabilityIds instanceof Set
        ? canonicalInventory.capabilityIds
        : new Set();
  const items = Array.isArray(body?.data) ? body.data : null;
  if (!items) {
    return {
      status: 'fail',
      expectedCount: expectedIds.size,
      observedCount: null,
      missing: [...expectedIds].sort(),
      unexpected: [],
      duplicates: [],
      malformed: ['response.data must be an array'],
    };
  }

  const observedIds = [];
  const malformed = [];
  for (const [index, item] of items.entries()) {
    const id = item?.capability?.id;
    if (typeof id !== 'string' || id.length === 0) {
      malformed.push(`response.data[${index}].capability.id`);
      continue;
    }
    observedIds.push(id);
  }

  const observedSet = new Set(observedIds);
  const duplicates = [...new Set(
    observedIds.filter((id, index) => observedIds.indexOf(id) !== index),
  )].sort();
  const missing = [...expectedIds].filter((id) => !observedSet.has(id)).sort();
  const unexpected = [...observedSet].filter((id) => !expectedIds.has(id)).sort();
  const status =
    malformed.length === 0 &&
    duplicates.length === 0 &&
    missing.length === 0 &&
    unexpected.length === 0
      ? 'pass'
      : 'fail';
  return {
    status,
    expectedCount: expectedIds.size,
    observedCount: observedIds.length,
    uniqueObservedCount: observedSet.size,
    missing,
    unexpected,
    duplicates,
    malformed,
  };
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

async function startupSmoke(binary, root, report, label, canonicalInventory) {
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
    const inventory = compareCapabilityInventory(body, canonicalInventory);
    check(report, `${label}:health`, 'pass', { status_code: health.response.status });
    check(
      report,
      `${label}:capability_inventory`,
      capabilities.status === 200 && body?.success === true && inventory.status === 'pass'
        ? 'pass'
        : 'fail',
      {
      source: canonicalInventory?.path || null,
      payload_digest: canonicalInventory?.payloadDigest || null,
      expected_count: inventory.expectedCount,
      observed_count: inventory.observedCount,
      unique_observed_count: inventory.uniqueObservedCount ?? null,
      missing: inventory.missing,
      unexpected: inventory.unexpected,
      duplicates: inventory.duplicates,
      malformed: inventory.malformed,
      status_code: capabilities.status,
      },
    );
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


export async function runValidation(
  options = parseArgs(process.argv.slice(2)),
  execution = {},
) {
  rejectRetiredOptions(options);
  const hostPlatform = execution.platform || process.platform;
  const hostArch = execution.arch || process.arch;
  const runCommand = execution.command || command;
  const inspectPath = execution.validatePathShape || validatePathShape;
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
    gate_name: 'macos-arm64-host-preflight',
    execution_kind: 'native',
    target_cell: TARGET_ID,
    execution_host: {
      platform: hostPlatform,
      arch: hostArch,
      uname: runCommand('uname', ['-s', '-m']),
      translated: runCommand('sysctl', ['-in', 'sysctl.proc_translated']),
      rustc: runCommand('rustc', ['-Vv']),
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

  if (hostPlatform !== 'darwin' || hostArch !== 'arm64') {
    check(report, 'native-host', 'fail', {
      reason: 'requires native macOS arm64',
      observed: { platform: hostPlatform, arch: hostArch },
    });
  } else {
    const translated = report.execution_host.translated.stdout.trim();
    const rustcHost = report.execution_host.rustc.stdout.match(/^host:\s*(.+)$/m)?.[1]?.trim();
    check(report, 'native-host', translated !== '1' && rustcHost === EXPECTED_TARGET ? 'pass' : 'fail', {
      expected: { platform: 'darwin', arch: 'arm64', translated: '0', rustc_host: EXPECTED_TARGET },
      observed: { platform: hostPlatform, arch: hostArch, translated, rustc_host: rustcHost || null },
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

  const worktree = runCommand('git', ['status', '--porcelain', '--untracked-files=no']);
  const head = runCommand('git', ['rev-parse', 'HEAD']);
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

  let lockedHostPath;
  let lockedPackagePath;
  try {
    lockedHostPath = resolveReleaseArtifactPath(artifactRoot, lock.host.path);
    lockedPackagePath = resolveReleaseArtifactPath(artifactRoot, lock.package.path);
  } catch (error) {
    check(report, 'release-lock:artifact-paths', 'fail', { reason: error.message });
    return finish();
  }

  const lockedAppPath = appFromHostBinary(lockedHostPath);
  checkOptionalArtifactOverride(report, 'override:app', options.app, lockedAppPath);
  checkOptionalArtifactOverride(report, 'override:dmg', options.dmg, lockedPackagePath);

  const appPath = lockedAppPath;
  report.artifacts.app = appPath;
  if (appPath) {
    const appShape = inspectPath(appPath, { kind: 'directory' });
    check(report, 'macos-app:path-case-permissions', appShape.status === 'pass' ? 'pass' : 'fail', appShape);
    const executable = join(appPath, 'Contents/MacOS/nomifun-desktop');
    const executableShape = inspectPath(executable, { requireExecutable: true });
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
      const archs = runCommand('lipo', ['-archs', executable]);
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
      const signature = runCommand('codesign', ['--verify', '--deep', '--strict', '--verbose=2', appPath], 120_000);
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
  const dmgShape = inspectPath(dmgPath);
  check(report, 'macos-package:path-case-permissions', dmgShape.status, dmgShape);
  if (dmgShape.status === 'pass') {
    const verify = runCommand('hdiutil', ['verify', dmgPath], 120_000);
    check(report, 'macos-package:hdiutil-verify', verify.status === 0 ? 'pass' : 'fail', {
      command: verify.command,
      exit_code: verify.status,
      stdout_tail: verify.stdout.slice(-2_000),
      stderr_tail: verify.stderr.slice(-2_000),
    });
  }


  const hostBinary = options.hostBinary ? resolve(options.hostBinary) : null;
  let canonicalInventory = null;
  const capabilityInventoryPath =
    options.capabilityInventory ||
    join(REPO_ROOT, CANONICAL_CAPABILITY_INVENTORY_RELATIVE_PATH);
  try {
    canonicalInventory = readCanonicalCapabilityInventory(capabilityInventoryPath);
    report.artifacts.capability_inventory = canonicalInventory.path;
    check(report, 'canonical-capability-inventory', 'pass', {
      source: canonicalInventory.path,
      payload_digest: canonicalInventory.payloadDigest,
      package_count: canonicalInventory.packageCount,
      capability_count: canonicalInventory.capabilityIds.size,
    });
  } catch (error) {
    check(report, 'canonical-capability-inventory', 'blocked', {
      reason: error.message,
      path: capabilityInventoryPath,
    });
    report.blockers.push(`missing/invalid canonical capability inventory: ${capabilityInventoryPath}`);
  }
  if (options.runStartup) {
    if (!hostBinary || !existingFile(hostBinary)?.isFile()) {
      check(report, 'startup:host-binary', 'blocked', {
        reason: '--run-startup requires an explicit existing --host-binary built from the tested source',
        path: hostBinary,
      });
      report.blockers.push('missing explicit host binary for requested startup validation');
    } else if (!canonicalInventory) {
      check(report, 'startup:capability-inventory', 'blocked', {
        reason: 'startup inventory comparison cannot run without the canonical capability inventory',
      });
    } else {
      const rootParent = mkdtempSync(join(tmpdir(), 'nomifun-c8-ma-'));
      try {
        const absentRoot = join(rootParent, 'absent-root');
        const emptyRoot = join(rootParent, 'precreated-empty-root');
        mkdirSync(emptyRoot);
        await startupSmoke(
          hostBinary,
          absentRoot,
          report,
          'startup:absent-root',
          canonicalInventory,
        );
        await startupSmoke(
          hostBinary,
          emptyRoot,
          report,
          'startup:precreated-empty-root',
          canonicalInventory,
        );
      } finally {
        rmSync(rootParent, { recursive: true, force: true });
      }
    }
  } else {
    check(report, 'startup:absent-root', 'not_required', {
      reason: 'package/native baseline does not run Host startup without --run-startup',
    });
    check(report, 'startup:precreated-empty-root', 'not_required', {
      reason: 'package/native baseline does not run Host startup without --run-startup',
    });
  }

  if (options.runLifecycle) {
    await remoteLifecycle(options, report);
  } else {
    check(report, 'lifecycle:open-ready-turn-observe-cancel-dispose', 'not_required', {
      reason: 'package/native baseline does not run product lifecycle without --run-lifecycle',
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
