#!/usr/bin/env node

/**
 * SL-S2-10 Codex official app-server upstream spike.
 *
 * This tool deliberately has no release-gate PASS state.  Static source
 * observations use "observed"; missing prerequisites use "blocked"; only the
 * harness self-test may return "self-test-pass".  It never reads a credential
 * file or performs a network retry.
 *
 * Safe static review:
 *   bun scripts/validation/codex-app-server-spike.mjs
 *
 * Optional live protocol smoke (initialize + initialized + ephemeral thread):
 *   bun scripts/validation/codex-app-server-spike.mjs \
 *     --run-live --binary /absolute/path/to/codex
 *
 * Explicit model turn/cancel smoke.  This may consume model quota and must be
 * opt-in.  Use an isolated CODEX_HOME whose authentication is managed by
 * Codex; never pass a token on the command line:
 *   bun scripts/validation/codex-app-server-spike.mjs \
 *     --run-live --run-turn-cancel --allow-live-model \
 *     --codex-home /absolute/path/to/isolated/codex-home \
 *     --binary /absolute/path/to/codex
 */

import { spawn, spawnSync } from 'node:child_process';
import { createInterface } from 'node:readline';
import {
  existsSync,
  mkdtempSync,
  mkdirSync,
  readFileSync,
  rmSync,
  writeFileSync,
} from 'node:fs';
import { tmpdir } from 'node:os';
import { dirname, join, relative, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

export const REPO_ROOT = resolve(dirname(fileURLToPath(import.meta.url)), '../..');
export const TASK_ID = 'SL-S2-10';
export const DEFAULT_UPSTREAM_DIR = resolve(REPO_ROOT, '..', 'codex');
export const DEFAULT_PINNED_COMMIT =
  'dc2ccc6843abb09c9d297862dc10b6bd12a3935d';
export const APP_SERVER_ARGS = ['app-server', '--listen', 'stdio://'];

export const UPSTREAM_SOURCE_PATHS = {
  readme: 'codex-rs/app-server/README.md',
  clientRequestSchema:
    'codex-rs/app-server-protocol/schema/json/ClientRequest.json',
  clientNotificationSchema:
    'codex-rs/app-server-protocol/schema/json/ClientNotification.json',
  serverNotificationSchema:
    'codex-rs/app-server-protocol/schema/json/ServerNotification.json',
  serverRequestSchema:
    'codex-rs/app-server-protocol/schema/json/ServerRequest.json',
  stdioTransport:
    'codex-rs/app-server-transport/src/transport/stdio.rs',
};

export const REQUIRED_CLIENT_METHODS = [
  'initialize',
  'thread/start',
  'thread/resume',
  'thread/fork',
  'turn/start',
  'turn/steer',
  'turn/interrupt',
];

export const REQUIRED_CLIENT_NOTIFICATIONS = ['initialized'];

export const REQUIRED_SERVER_NOTIFICATIONS = [
  'thread/started',
  'turn/started',
  'turn/completed',
  'item/started',
  'item/completed',
];

export const REQUIRED_SERVER_REQUESTS = ['item/tool/call'];

export const SUPERSEDED_CUSTOM_METHODS = [
  'runtime/hello',
  'runtime/session/dispose',
  'native_action/start',
];

const SHA1_PATTERN = /^[0-9a-f]{40}$/;
const SENSITIVE_KEY_PATTERN =
  /(?:token|secret|password|credential|api[-_]?key|authorization|cookie|auth|email)/i;
const PATH_KEY_PATTERN =
  /(?:^|[_-])(path|cwd|home|directory|file|executable|workingdir|codexhome)(?:$|[_-])/i;

function nowIso() {
  return new Date().toISOString();
}

function isObject(value) {
  return value !== null && typeof value === 'object' && !Array.isArray(value);
}

function uniqueStrings(values) {
  return [...new Set(values.filter((value) => typeof value === 'string'))].sort();
}

function parseInteger(value, name) {
  const parsed = Number(value);
  if (!Number.isInteger(parsed) || parsed < 1) {
    throw new Error(`${name} must be a positive integer`);
  }
  return parsed;
}

function requireValue(argv, index, token) {
  const value = argv[index + 1];
  if (!value || value.startsWith('--')) {
    throw new Error(`${token} requires a value`);
  }
  return value;
}

export function parseArgs(argv) {
  const options = {
    upstreamDir:
      process.env.CODEX_APP_SERVER_UPSTREAM_DIR || DEFAULT_UPSTREAM_DIR,
    pinnedCommit: process.env.CODEX_APP_SERVER_PINNED_COMMIT || null,
    binary: process.env.CODEX_APP_SERVER_BIN || null,
    runLive: false,
    runThreadStart: true,
    runTurnCancel: false,
    allowLiveModel: false,
    codexHome: process.env.CODEX_APP_SERVER_CODEX_HOME || null,
    timeoutMs: 10_000,
    report: null,
    selfTest: false,
    help: false,
  };

  for (let index = 0; index < argv.length; index += 1) {
    const token = argv[index];
    if (token === '--self-test') {
      options.selfTest = true;
      continue;
    }
    if (token === '--help' || token === '-h') {
      options.help = true;
      continue;
    }
    if (token === '--run-live') {
      options.runLive = true;
      continue;
    }
    if (token === '--skip-thread-start') {
      options.runThreadStart = false;
      continue;
    }
    if (token === '--run-turn-cancel') {
      options.runLive = true;
      options.runTurnCancel = true;
      continue;
    }
    if (token === '--allow-live-model') {
      options.allowLiveModel = true;
      continue;
    }

    const equal = token.match(/^--([^=]+)=(.*)$/);
    const key = equal ? equal[1] : token;
    const value = equal ? equal[2] : requireValue(argv, index, token);
    if (!equal) index += 1;

    switch (key) {
      case '--upstream-dir':
        options.upstreamDir = value;
        break;
      case '--pinned-commit':
        options.pinnedCommit = value;
        break;
      case '--binary':
        options.binary = value;
        break;
      case '--codex-home':
        options.codexHome = value;
        break;
      case '--timeout-ms':
        options.timeoutMs = parseInteger(value, '--timeout-ms');
        break;
      case '--report':
        options.report = value;
        break;
      default:
        throw new Error(`unknown argument: ${token}`);
    }
  }

  if (options.runTurnCancel && !options.allowLiveModel) {
    throw new Error(
      '--run-turn-cancel requires --allow-live-model; this is an explicit quota-consuming action',
    );
  }
  if (options.timeoutMs < 500) {
    throw new Error('--timeout-ms must be at least 500');
  }
  return options;
}

export function extractTopLevelMethodValues(schema) {
  const methods = [];
  const branches = Array.isArray(schema?.oneOf) ? schema.oneOf : [];
  for (const branch of branches) {
    const method = branch?.properties?.method;
    if (typeof method?.const === 'string') methods.push(method.const);
    if (Array.isArray(method?.enum)) methods.push(...method.enum);
  }

  const topLevelMethod = schema?.properties?.method;
  if (typeof topLevelMethod?.const === 'string') {
    methods.push(topLevelMethod.const);
  }
  if (Array.isArray(topLevelMethod?.enum)) methods.push(...topLevelMethod.enum);
  return uniqueStrings(methods);
}

export function redactValue(value, key = '', depth = 0) {
  if (SENSITIVE_KEY_PATTERN.test(key)) return '<redacted>';
  if (depth > 6) return '<depth-limited>';
  if (value === null || value === undefined) return value;
  if (typeof value === 'string') {
    if (PATH_KEY_PATTERN.test(key)) return '<path>';
    return value.length > 800 ? `${value.slice(0, 800)}…` : value;
  }
  if (typeof value !== 'object') return value;
  if (Array.isArray(value)) {
    return value.slice(0, 100).map((entry) => redactValue(entry, key, depth + 1));
  }
  return Object.fromEntries(
    Object.entries(value).map(([entryKey, entryValue]) => [
      entryKey,
      redactValue(entryValue, entryKey, depth + 1),
    ]),
  );
}

export function parseJsonLine(line) {
  const trimmed = String(line).trim();
  if (!trimmed) return null;
  const value = JSON.parse(trimmed);
  if (!isObject(value)) {
    throw new Error('app-server frame must be a JSON object');
  }
  return value;
}

function check(report, id, status, details = {}) {
  report.checks.push({ id, status, ...details });
}

function addBlocker(report, id, reason, details = {}) {
  report.blockers.push({ id, reason, ...details });
}

function finalStatus(report) {
  if (report.checks.some((entry) => entry.status === 'fail')) return 'fail';
  if (
    report.blockers.length > 0 ||
    report.checks.some((entry) => entry.status === 'blocked')
  ) {
    return 'blocked';
  }
  return 'observed';
}

function safeRelativePath(path) {
  const absolute = resolve(path);
  const rel = relative(REPO_ROOT, absolute);
  if (rel && !rel.startsWith('..') && !rel.includes(':')) {
    return rel.replaceAll('\\', '/');
  }
  if (absolute === REPO_ROOT) return '.';
  return '<external>';
}

function commandResult(command, args, cwd, timeout = 10_000) {
  const result = spawnSync(command, args, {
    cwd,
    encoding: 'utf8',
    timeout,
    windowsHide: true,
    shell: false,
    maxBuffer: 12 * 1024 * 1024,
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

function gitResult(upstreamDir, args, timeout = 10_000) {
  return commandResult('git', ['-C', upstreamDir, ...args], REPO_ROOT, timeout);
}

function gitText(upstreamDir, args, timeout = 10_000) {
  const result = gitResult(upstreamDir, args, timeout);
  if (result.status !== 0) {
    throw new Error(result.stderr.trim() || result.error || `git exited ${result.status}`);
  }
  return result.stdout;
}

function readSourceLock() {
  const path = join(REPO_ROOT, 'vendor/codex-runtime/source-lock.json');
  try {
    const value = JSON.parse(readFileSync(path, 'utf8'));
    return {
      path: safeRelativePath(path),
      pinnedCommit:
        typeof value.pinned_commit === 'string' ? value.pinned_commit : null,
      trackedUpstreamCommit:
        typeof value.tracked_upstream_commit === 'string'
          ? value.tracked_upstream_commit
          : null,
      vendoredUpstreamSource: value.vendored_upstream_source === true,
    };
  } catch (error) {
    return {
      path: safeRelativePath(path),
      pinnedCommit: null,
      trackedUpstreamCommit: null,
      vendoredUpstreamSource: false,
      error: error.message,
    };
  }
}

function requestedPin(options, sourceLock) {
  if (options.pinnedCommit) {
    return { value: options.pinnedCommit, source: 'cli-or-environment' };
  }
  if (sourceLock.pinnedCommit) {
    return { value: sourceLock.pinnedCommit, source: sourceLock.path };
  }
  return { value: null, source: null };
}

function sourceAtPin(upstreamDir, pin, path) {
  return gitText(upstreamDir, ['show', `${pin}:${path}`], 15_000);
}

function hasText(text, pattern) {
  return pattern instanceof RegExp ? pattern.test(text) : text.includes(pattern);
}

function requiredMethodCheck(
  report,
  id,
  method,
  methods,
  sourcePath,
  description,
) {
  const present = methods.includes(method);
  check(report, id, present ? 'observed' : 'blocked', {
    method,
    source: sourcePath,
    description,
    present,
  });
  return present;
}

export function inspectUpstream(options = {}) {
  const upstreamDir = resolve(options.upstreamDir || DEFAULT_UPSTREAM_DIR);
  const sourceLock = readSourceLock();
  const pinInfo = requestedPin(options, sourceLock);
  const report = {
    schema_version: '1.0.0',
    task_id: TASK_ID,
    generated_at: nowIso(),
    mode: 'static',
    status: 'blocked',
    upstream: {
      directory: safeRelativePath(upstreamDir),
      requested_pin: pinInfo.value,
      requested_pin_source: pinInfo.source,
      checkout_head: null,
      pin_commit: null,
      pin_is_head: null,
      pin_is_ancestor_of_head: null,
      working_tree_clean: null,
      source_lock: sourceLock,
    },
    protocol: {
      transport: null,
      client_requests: [],
      client_notifications: [],
      server_notifications: [],
      server_requests: [],
      source_files: {},
    },
    decision: {
      static_patch_conclusion: 'not-needed-by-observed-upstream-seams',
      final_patch_conclusion: 'blocked-until-live-smoke',
      custom_methods: Object.fromEntries(
        SUPERSEDED_CUSTOM_METHODS.map((method) => [method, 'not-inspected']),
      ),
    },
    checks: [],
    blockers: [],
    trace: [],
  };

  if (!existsSync(upstreamDir)) {
    check(report, 'upstream:checkout', 'blocked', {
      reason: 'directory_missing',
      path: safeRelativePath(upstreamDir),
    });
    addBlocker(report, 'upstream-checkout-missing', 'official Codex checkout was not found', {
      expected_path: safeRelativePath(upstreamDir),
    });
    report.status = finalStatus(report);
    return report;
  }

  const inside = gitResult(upstreamDir, ['rev-parse', '--is-inside-work-tree']);
  if (inside.status !== 0 || inside.stdout.trim() !== 'true') {
    check(report, 'upstream:git-repository', 'blocked', {
      reason: 'not_a_git_worktree',
      stderr: inside.stderr.trim() || inside.error,
    });
    addBlocker(report, 'upstream-not-git', 'upstream directory is not a Git worktree');
    report.status = finalStatus(report);
    return report;
  }

  const head = gitResult(upstreamDir, ['rev-parse', 'HEAD']);
  report.upstream.checkout_head = head.stdout.trim() || null;
  check(report, 'upstream:head', head.status === 0 ? 'observed' : 'blocked', {
    value: report.upstream.checkout_head,
    stderr: head.stderr.trim() || head.error || null,
  });

  const status = gitResult(upstreamDir, [
    'status',
    '--porcelain=v1',
    '--untracked-files=all',
  ]);
  report.upstream.working_tree_clean =
    status.status === 0 && status.stdout.trim() === '';
  check(
    report,
    'upstream:working-tree',
    status.status === 0 ? 'observed' : 'blocked',
    {
      clean: report.upstream.working_tree_clean,
      note: report.upstream.working_tree_clean
        ? 'clean'
        : 'dirty_or_unavailable; pinned Git objects are still inspected directly',
    },
  );

  if (!pinInfo.value || !SHA1_PATTERN.test(pinInfo.value)) {
    check(report, 'upstream:pinned-commit', 'blocked', {
      reason: 'missing_or_invalid_pin',
      requested_pin: pinInfo.value,
    });
    addBlocker(
      report,
      'pinned-commit-missing',
      'a 40-character lowercase pinned upstream commit is required',
    );
    report.status = finalStatus(report);
    return report;
  }

  const pin = pinInfo.value;
  const pinObject = gitResult(upstreamDir, ['cat-file', '-e', `${pin}^{commit}`]);
  const pinExists = pinObject.status === 0;
  check(report, 'upstream:pinned-object', pinExists ? 'observed' : 'blocked', {
    commit: pin,
    present: pinExists,
    stderr: pinObject.stderr.trim() || pinObject.error || null,
  });
  if (!pinExists) {
    addBlocker(
      report,
      'pinned-commit-unavailable',
      'the requested upstream commit is not present locally; network fetch was intentionally not attempted',
      { commit: pin },
    );
    report.status = finalStatus(report);
    return report;
  }

  const pinMetadata = gitResult(upstreamDir, [
    'show',
    '-s',
    '--format=%H%n%ad%n%s',
    '--date=iso-strict',
    pin,
  ]);
  const metadataLines = pinMetadata.stdout.trim().split(/\r?\n/);
  report.upstream.pin_commit = metadataLines[0] || pin;
  report.upstream.pin_metadata = {
    committed_at: metadataLines[1] || null,
    subject: metadataLines.slice(2).join('\n') || null,
  };
  check(report, 'upstream:pinned-metadata', pinMetadata.status === 0 ? 'observed' : 'blocked', {
    commit: report.upstream.pin_commit,
    committed_at: report.upstream.pin_metadata.committed_at,
    subject: report.upstream.pin_metadata.subject,
  });

  report.upstream.pin_is_head =
    report.upstream.checkout_head === report.upstream.pin_commit;
  const ancestor = gitResult(upstreamDir, ['merge-base', '--is-ancestor', pin, 'HEAD']);
  report.upstream.pin_is_ancestor_of_head = ancestor.status === 0;
  check(report, 'upstream:pin-relation', ancestor.status === 0 ? 'observed' : 'warning', {
    pin_is_head: report.upstream.pin_is_head,
    pin_is_ancestor_of_head: report.upstream.pin_is_ancestor_of_head,
    note: report.upstream.pin_is_head
      ? 'checkout is exactly at the requested pin'
      : 'inspection uses git show at the requested pin; current checkout HEAD is not treated as the pin',
  });

  const contents = {};
  for (const [name, path] of Object.entries(UPSTREAM_SOURCE_PATHS)) {
    try {
      contents[name] = sourceAtPin(upstreamDir, pin, path);
      report.protocol.source_files[name] = {
        path,
        status: 'observed',
        bytes: Buffer.byteLength(contents[name], 'utf8'),
      };
    } catch (error) {
      report.protocol.source_files[name] = {
        path,
        status: 'blocked',
        reason: error.message,
      };
      addBlocker(report, `source-file:${name}`, 'required source file could not be read at pin', {
        path,
      });
    }
  }

  if (
    !contents.clientRequestSchema ||
    !contents.clientNotificationSchema ||
    !contents.serverNotificationSchema ||
    !contents.serverRequestSchema
  ) {
    check(report, 'protocol:schemas', 'blocked', {
      reason: 'one_or_more_protocol_schemas_missing',
    });
    report.status = finalStatus(report);
    return report;
  }

  let schemas;
  try {
    schemas = {
      clientRequest: JSON.parse(contents.clientRequestSchema),
      clientNotification: JSON.parse(contents.clientNotificationSchema),
      serverNotification: JSON.parse(contents.serverNotificationSchema),
      serverRequest: JSON.parse(contents.serverRequestSchema),
    };
    check(report, 'protocol:schema-json', 'observed', {
      source: 'official app-server protocol schema at pinned commit',
    });
  } catch (error) {
    check(report, 'protocol:schema-json', 'fail', { reason: error.message });
    addBlocker(report, 'schema-parse-failed', 'official protocol schema is not valid JSON');
    report.status = finalStatus(report);
    return report;
  }

  const clientMethods = extractTopLevelMethodValues(schemas.clientRequest);
  const clientNotifications = extractTopLevelMethodValues(
    schemas.clientNotification,
  );
  const serverNotifications = extractTopLevelMethodValues(
    schemas.serverNotification,
  );
  const serverRequests = extractTopLevelMethodValues(schemas.serverRequest);
  report.protocol.client_requests = clientMethods;
  report.protocol.client_notifications = clientNotifications;
  report.protocol.server_notifications = serverNotifications;
  report.protocol.server_requests = serverRequests;

  for (const method of REQUIRED_CLIENT_METHODS) {
    requiredMethodCheck(
      report,
      `protocol:client:${method}`,
      method,
      clientMethods,
      UPSTREAM_SOURCE_PATHS.clientRequestSchema,
      'client request method',
    );
  }
  for (const method of REQUIRED_CLIENT_NOTIFICATIONS) {
    requiredMethodCheck(
      report,
      `protocol:client-notification:${method}`,
      method,
      clientNotifications,
      UPSTREAM_SOURCE_PATHS.clientNotificationSchema,
      'client notification method',
    );
  }
  for (const method of REQUIRED_SERVER_NOTIFICATIONS) {
    requiredMethodCheck(
      report,
      `protocol:server-notification:${method}`,
      method,
      serverNotifications,
      UPSTREAM_SOURCE_PATHS.serverNotificationSchema,
      'server event notification',
    );
  }
  for (const method of REQUIRED_SERVER_REQUESTS) {
    requiredMethodCheck(
      report,
      `protocol:server-request:${method}`,
      method,
      serverRequests,
      UPSTREAM_SOURCE_PATHS.serverRequestSchema,
      'server-to-host request seam',
    );
  }

  const readme = contents.readme || '';
  const transport = contents.stdioTransport || '';
  report.protocol.transport = {
    kind: 'stdio',
    framing: 'newline-delimited-json',
    jsonrpc_header_omitted:
      hasText(readme, /"jsonrpc":"2\.0".*omitted on the wire/i) ||
      hasText(transport, /JSONRPC.*header|jsonrpc/i),
    websocket_is_not_required: hasText(
      readme,
      /websocket.*experimental\s*\/\s*unsupported/i,
    ),
  };
  check(report, 'protocol:stdio-jsonl', 'observed', {
    source: [
      UPSTREAM_SOURCE_PATHS.readme,
      UPSTREAM_SOURCE_PATHS.stdioTransport,
    ],
    transport: report.protocol.transport,
  });

  check(report, 'protocol:initialize-order', 'observed', {
    sequence: ['initialize request', 'initialize response', 'initialized notification'],
    source: UPSTREAM_SOURCE_PATHS.readme,
    note: 'the pinned README requires one initialize per connection before other requests',
  });

  const hasVersionRpc = clientMethods.includes('version');
  check(report, 'protocol:version-identity', 'observed', {
    dedicated_version_rpc: hasVersionRpc,
    initialize_response_fields: [
      'userAgent',
      'codexHome',
      'platformFamily',
      'platformOs',
    ],
    binary_identity: 'codex --version',
    note: hasVersionRpc
      ? 'a dedicated version method exists in the pinned schema'
      : 'no dedicated version RPC found; use initialize identity plus the exact binary version',
  });

  check(report, 'protocol:host-managed-tool', 'observed', {
    method: 'item/tool/call',
    registration: 'thread/start.dynamicTools',
    experimental_api_required: hasText(
      readme,
      /dynamicTools.*experimental APIs|experimental APIs.*dynamicTools/i,
    ),
    lifecycle: ['item/started', 'item/tool/call', 'client response', 'item/completed'],
    source: [
      UPSTREAM_SOURCE_PATHS.readme,
      UPSTREAM_SOURCE_PATHS.serverRequestSchema,
    ],
    note: 'this is the observed upstream callback seam; no native_action/start is required by the pin',
  });

  check(report, 'protocol:process-close', 'observed', {
    connection_close: hasText(transport, /ConnectionClosed/),
    stdin_eof: hasText(transport, /stdin.*EOF/i),
    thread_data_delete: clientMethods.includes('thread/delete'),
    source: [
      UPSTREAM_SOURCE_PATHS.stdioTransport,
      UPSTREAM_SOURCE_PATHS.readme,
    ],
    note: 'close stdin and let the Host reap the process; thread/delete is data lifecycle, not a custom session-dispose ACK',
  });

  const allUpstreamText = Object.values(contents).join('\n');
  for (const method of SUPERSEDED_CUSTOM_METHODS) {
    const present = allUpstreamText.includes(method);
    report.decision.custom_methods[method] = present ? 'present_at_pin' : 'absent_at_pin';
    check(report, `protocol:custom-method:${method}`, present ? 'fail' : 'observed', {
      method,
      present,
      source_scope: 'official upstream source and schemas at pinned commit',
      note: present
        ? 'unexpected custom method found; patch decision must be revisited'
        : 'not an upstream method; do not add it to the minimum Host contract',
    });
  }

  check(report, 'protocol:static-seam-review', 'observed', {
    conclusion: 'upstream provides the required initialize/thread/turn/cancel/event/tool seams',
    patch: 'no narrow patch indicated by static review',
    remaining: [
      'exact binary provenance for the pinned commit',
      'live initialize/thread/turn/interrupt/event observation',
      'live dynamic tool callback observation',
      'process-tree cleanup observation on the target packaging path',
    ],
  });

  report.status = finalStatus(report);
  return report;
}

function withTimeout(promise, timeoutMs, label) {
  let timer;
  const timeout = new Promise((_, reject) => {
    timer = setTimeout(() => {
      reject(new Error(`timed out waiting for ${label} after ${timeoutMs}ms`));
    }, timeoutMs);
  });
  return Promise.race([promise, timeout]).finally(() => clearTimeout(timer));
}

function frameSummary(frame) {
  if (!isObject(frame)) return { kind: 'invalid' };
  return {
    id: frame.id ?? null,
    method: typeof frame.method === 'string' ? frame.method : null,
    has_result: Object.prototype.hasOwnProperty.call(frame, 'result'),
    has_error: Object.prototype.hasOwnProperty.call(frame, 'error'),
    params_keys: isObject(frame.params) ? Object.keys(frame.params).sort() : [],
    result_keys: isObject(frame.result) ? Object.keys(frame.result).sort() : [],
    error: frame.error
      ? redactValue(frame.error, 'error')
      : null,
  };
}

function childExitPromise(child) {
  if (child.exitCode !== null || child.signalCode !== null) {
    return Promise.resolve({
      code: child.exitCode,
      signal: child.signalCode,
    });
  }
  return new Promise((resolvePromise) => {
    child.once('exit', (code, signal) => resolvePromise({ code, signal }));
  });
}

async function terminateChild(child) {
  if (!child || (child.exitCode !== null && child.signalCode !== null)) {
    return { attempted: false, forced: false };
  }
  if (child.exitCode !== null || child.signalCode !== null) {
    return { attempted: false, forced: false };
  }

  if (process.platform === 'win32' && child.pid) {
    const result = commandResult(
      'taskkill',
      ['/PID', String(child.pid), '/T', '/F'],
      REPO_ROOT,
      5_000,
    );
    await Promise.race([
      childExitPromise(child),
      new Promise((resolvePromise) => setTimeout(resolvePromise, 2_000)),
    ]);
    return {
      attempted: true,
      forced: true,
      taskkill_status: result.status,
      taskkill_error: result.error,
    };
  }

  try {
    child.kill('SIGTERM');
  } catch {
    // The process may have exited between the check and signal.
  }
  const exited = await Promise.race([
    childExitPromise(child),
    new Promise((resolvePromise) =>
      setTimeout(() => resolvePromise(null), 2_000),
    ),
  ]);
  if (exited) return { attempted: true, forced: false };

  try {
    child.kill('SIGKILL');
  } catch {
    // Best effort after the bounded graceful window.
  }
  await Promise.race([
    childExitPromise(child),
    new Promise((resolvePromise) => setTimeout(resolvePromise, 2_000)),
  ]);
  return { attempted: true, forced: true };
}

async function waitForFrame(iterator, timeoutMs, trace) {
  while (true) {
    const next = await withTimeout(iterator.next(), timeoutMs, 'app-server frame');
    if (next.done) throw new Error('app-server stdout closed before the expected frame');
    const line = String(next.value || '').trim();
    if (!line) continue;
    let frame;
    try {
      frame = parseJsonLine(line);
    } catch (error) {
      trace.push({
        direction: 'inbound',
        kind: 'malformed',
        error: error.message,
        line_prefix: line.slice(0, 240),
      });
      throw new Error(`invalid app-server JSONL frame: ${error.message}`);
    }
    trace.push({
      direction: 'inbound',
      frame: redactValue(frame),
      summary: frameSummary(frame),
    });
    return frame;
  }
}

async function requestFrame(
  child,
  iterator,
  trace,
  id,
  method,
  params,
  timeoutMs,
  notifications,
) {
  const request = { id, method, params };
  child.stdin.write(`${JSON.stringify(request)}\n`);
  trace.push({
    direction: 'outbound',
    frame: redactValue(request),
    summary: frameSummary(request),
  });
  while (true) {
    const frame = await waitForFrame(iterator, timeoutMs, trace);
    if (
      Object.prototype.hasOwnProperty.call(frame, 'id') &&
      String(frame.id) === String(id) &&
      (Object.prototype.hasOwnProperty.call(frame, 'result') ||
        Object.prototype.hasOwnProperty.call(frame, 'error'))
    ) {
      return frame;
    }
    notifications.push(frame);
  }
}

async function waitForNotification(
  iterator,
  trace,
  notifications,
  method,
  timeoutMs,
) {
  const existing = notifications.find(
    (frame) => frame?.method === method,
  );
  if (existing) return existing;
  while (true) {
    const frame = await waitForFrame(iterator, timeoutMs, trace);
    if (frame?.method === method) return frame;
    notifications.push(frame);
  }
}

function extractThreadId(response) {
  return (
    response?.result?.thread?.id ||
    response?.result?.threadId ||
    response?.result?.thread_id ||
    null
  );
}

function extractTurnId(response) {
  return response?.result?.turn?.id || response?.result?.turnId || null;
}

function responseSucceeded(response) {
  return (
    isObject(response) &&
    Object.prototype.hasOwnProperty.call(response, 'result') &&
    !Object.prototype.hasOwnProperty.call(response, 'error')
  );
}

function isLikelyCredentialOrConfigurationBlock(response) {
  const text = JSON.stringify(response || '').toLowerCase();
  return /auth|credential|login|unauthor|model|config|not configured|api key/.test(text);
}

export async function runLiveProbe(options, dependencies = {}) {
  const report = {
    mode: 'live',
    checks: [],
    blockers: [],
    trace: [],
    binary: {
      requested: options.binary || null,
      version: null,
      provenance: 'unverified',
    },
  };
  const command = dependencies.command || options.binary;
  const args = dependencies.args || APP_SERVER_ARGS;
  const spawnFn = dependencies.spawn || spawn;
  if (!command) {
    check(report, 'live:binary', 'blocked', {
      reason: 'binary_not_supplied',
      hint: '--binary <exact app-server-capable codex executable>',
    });
    addBlocker(
      report,
      'live-binary-missing',
      'no official app-server executable was supplied; static source review cannot replace live evidence',
    );
    return report;
  }

  const version = commandResult(command, ['--version'], REPO_ROOT, 5_000);
  report.binary.version = (version.stdout || version.stderr).trim().slice(0, 240) || null;
  check(report, 'live:binary-version', version.status === 0 ? 'observed' : 'warning', {
    output: report.binary.version,
    status: version.status,
    error: version.error,
    note: 'version output is descriptive only; it does not prove the requested source commit',
  });
  addBlocker(
    report,
    'live-binary-provenance',
    'the supplied binary is not cryptographically tied to the requested upstream commit by this harness',
    { version: report.binary.version },
  );

  const temporaryHome = options.codexHome
    ? null
    : mkdtempSync(join(tmpdir(), 'nomifun-codex-app-server-spike-'));
  const codexHome = resolve(options.codexHome || temporaryHome);
  const environment = { ...process.env, CODEX_HOME: codexHome, RUST_LOG: 'off' };
  for (const key of [
    'OPENAI_API_KEY',
    'CODEX_API_KEY',
    'CODEX_ACCESS_TOKEN',
    'OPENAI_ACCESS_TOKEN',
  ]) {
    delete environment[key];
  }

  let child;
  let readline;
  let iterator;
  let stderr = '';
  let forcedCleanup = false;
  const notifications = [];
  let nextId = 1;
  let threadId = null;
  try {
    child = spawnFn(command, args, {
      cwd: REPO_ROOT,
      env: environment,
      stdio: ['pipe', 'pipe', 'pipe'],
      windowsHide: true,
      shell: false,
    });
    child.stderr.on('data', (chunk) => {
      stderr += String(chunk);
    });
    readline = createInterface({ input: child.stdout, crlfDelay: Infinity });
    iterator = readline[Symbol.asyncIterator]();
    check(report, 'live:process-start', 'observed', {
      pid: child.pid || null,
      invocation: [command, ...args].join(' '),
    });

    const initialize = await requestFrame(
      child,
      iterator,
      report.trace,
      nextId++,
      'initialize',
      {
        clientInfo: {
          name: 'nomifun-sidecar-upstream-spike',
          title: 'NomiFun official app-server upstream spike',
          version: '0.1.0',
        },
        capabilities: {
          experimentalApi: Boolean(options.runTurnCancel),
        },
      },
      options.timeoutMs,
      notifications,
    );
    if (!responseSucceeded(initialize)) {
      check(report, 'live:initialize', isLikelyCredentialOrConfigurationBlock(initialize) ? 'blocked' : 'fail', {
        response: redactValue(initialize),
      });
      addBlocker(report, 'live-initialize-rejected', 'upstream rejected initialize', {
        response: redactValue(initialize),
      });
      return report;
    }
    check(report, 'live:initialize', 'observed', {
      response: redactValue(initialize),
      identity_fields: Object.keys(initialize.result || {}).sort(),
    });

    const initialized = { method: 'initialized' };
    child.stdin.write(`${JSON.stringify(initialized)}\n`);
    report.trace.push({
      direction: 'outbound',
      frame: initialized,
      summary: frameSummary(initialized),
    });
    check(report, 'live:initialized-notification', 'observed', {
      method: 'initialized',
    });

    if (options.runThreadStart) {
      const threadStartParams = {
        ephemeral: true,
        cwd: REPO_ROOT,
      };
      if (options.runTurnCancel) {
        threadStartParams.dynamicTools = [
          {
            type: 'namespace',
            name: 'nomifun_spike',
            description: 'Read-only protocol smoke tool',
            tools: [
              {
                type: 'function',
                name: 'lookup',
                description: 'Return a fixed read-only smoke value',
                inputSchema: {
                  type: 'object',
                  properties: { key: { type: 'string' } },
                  required: ['key'],
                },
              },
            ],
          },
        ];
      }
      const threadStart = await requestFrame(
        child,
        iterator,
        report.trace,
        nextId++,
        'thread/start',
        threadStartParams,
        options.timeoutMs,
        notifications,
      );
      if (!responseSucceeded(threadStart)) {
        const status = isLikelyCredentialOrConfigurationBlock(threadStart)
          ? 'blocked'
          : 'fail';
        check(report, 'live:thread-start', status, {
          response: redactValue(threadStart),
        });
        addBlocker(report, 'live-thread-start-rejected', 'upstream did not create an ephemeral thread', {
          response: redactValue(threadStart),
        });
      } else {
        threadId = extractThreadId(threadStart);
        check(report, 'live:thread-start', threadId ? 'observed' : 'warning', {
          response: redactValue(threadStart),
          thread_id_observed: Boolean(threadId),
        });
        if (threadId) {
          try {
            const event = await waitForNotification(
              iterator,
              report.trace,
              notifications,
              'thread/started',
              options.timeoutMs,
            );
            check(report, 'live:thread-started-event', 'observed', {
              summary: frameSummary(event),
            });
          } catch (error) {
            check(report, 'live:thread-started-event', 'blocked', {
              reason: error.message,
            });
            addBlocker(
              report,
              'live-thread-event-missing',
              'thread/start response was observed but thread/started was not observed before the deadline',
            );
          }
        }
      }
    }

    if (!options.runTurnCancel) {
      check(report, 'live:turn-cancel-event', 'blocked', {
        reason: 'model turn/cancel smoke was not requested',
        hint: '--run-turn-cancel --allow-live-model --codex-home <isolated-auth-home>',
      });
      addBlocker(
        report,
        'live-model-smoke-not-run',
        'turn/interrupt and turn/completed(interrupted) require an explicit live model authorization',
      );
      return report;
    }

    if (!options.codexHome) {
      check(report, 'live:credential-home', 'blocked', {
        reason: 'an isolated authenticated CODEX_HOME was not supplied',
      });
      addBlocker(
        report,
        'live-credential-home-missing',
        'no credential source was supplied; the harness will not inspect or infer credentials',
      );
      return report;
    }
    if (!threadId) {
      check(report, 'live:turn-cancel-event', 'blocked', {
        reason: 'thread id unavailable',
      });
      addBlocker(
        report,
        'live-turn-prerequisite-missing',
        'turn/cancel smoke cannot start without a live thread',
      );
      return report;
    }

    const turnStart = await requestFrame(
      child,
      iterator,
      report.trace,
      nextId++,
      'turn/start',
      {
        threadId,
        input: [
          {
            type: 'text',
            text: 'Use the nomifun_spike.lookup tool with key "smoke", then wait.',
          },
        ],
      },
      options.timeoutMs,
      notifications,
    );
    if (!responseSucceeded(turnStart)) {
      const status = isLikelyCredentialOrConfigurationBlock(turnStart)
        ? 'blocked'
        : 'fail';
      check(report, 'live:turn-start', status, {
        response: redactValue(turnStart),
      });
      addBlocker(report, 'live-turn-start-rejected', 'upstream did not accept the authorized live turn', {
        response: redactValue(turnStart),
      });
      return report;
    }
    const turnId = extractTurnId(turnStart);
    check(report, 'live:turn-start', turnId ? 'observed' : 'warning', {
      response: redactValue(turnStart),
      turn_id_observed: Boolean(turnId),
    });
    if (!turnId) {
      addBlocker(
        report,
        'live-turn-id-missing',
        'turn/start succeeded but did not expose a turn id needed for interruption',
      );
      return report;
    }

    try {
      const started = await waitForNotification(
        iterator,
        report.trace,
        notifications,
        'turn/started',
        options.timeoutMs,
      );
      check(report, 'live:turn-started-event', 'observed', {
        summary: frameSummary(started),
      });
    } catch (error) {
      check(report, 'live:turn-started-event', 'warning', {
        reason: error.message,
      });
    }

    const interrupt = await requestFrame(
      child,
      iterator,
      report.trace,
      nextId++,
      'turn/interrupt',
      { threadId, turnId },
      options.timeoutMs,
      notifications,
    );
    if (!responseSucceeded(interrupt)) {
      check(report, 'live:turn-interrupt', 'fail', {
        response: redactValue(interrupt),
      });
      addBlocker(
        report,
        'live-turn-interrupt-rejected',
        'upstream did not accept turn/interrupt for the active turn',
      );
      return report;
    }
    check(report, 'live:turn-interrupt', 'observed', {
      response: redactValue(interrupt),
    });

    try {
      const completed = await waitForNotification(
        iterator,
        report.trace,
        notifications,
        'turn/completed',
        options.timeoutMs,
      );
      const status = completed?.params?.turn?.status || completed?.params?.status || null;
      check(report, 'live:turn-completed', status === 'interrupted' ? 'observed' : 'warning', {
        summary: frameSummary(completed),
        observed_status: status,
        expected_status: 'interrupted',
      });
      if (status !== 'interrupted') {
        addBlocker(
          report,
          'live-turn-completion-status',
          'turn/completed was observed without the expected interrupted status',
          { observed_status: status },
        );
      }
    } catch (error) {
      check(report, 'live:turn-completed', 'blocked', {
        reason: error.message,
      });
      addBlocker(
        report,
        'live-turn-completed-missing',
        'turn/interrupt response was observed but turn/completed was not observed before the deadline',
      );
    }
  } catch (error) {
    check(report, 'live:protocol-session', 'fail', {
      reason: error.message,
      stderr_tail: stderr.slice(-2_000),
    });
    addBlocker(report, 'live-probe-error', 'live protocol probe stopped at its first failure', {
      reason: error.message,
    });
  } finally {
    try {
      if (child?.stdin && !child.stdin.destroyed) child.stdin.end();
    } catch {
      // The child may already have closed stdin.
    }
    if (child) {
      const exited = await Promise.race([
        childExitPromise(child),
        new Promise((resolvePromise) =>
          setTimeout(() => resolvePromise(null), options.timeoutMs),
        ),
      ]);
      if (!exited) {
        const cleanup = await terminateChild(child);
        forcedCleanup = cleanup.forced;
      }
      const finalExit = await Promise.race([
        childExitPromise(child),
        new Promise((resolvePromise) => setTimeout(() => resolvePromise(null), 1_000)),
      ]);
      check(report, 'live:process-close', finalExit ? 'observed' : 'blocked', {
        exit: finalExit,
        forced_cleanup: forcedCleanup,
        stderr_tail: stderr.slice(-2_000),
        note: 'the Host must close the protocol and reap descendants; no custom dispose RPC was sent',
      });
      if (!finalExit) {
        addBlocker(
          report,
          'live-process-not-closed',
          'app-server did not exit within the bounded close window',
        );
      }
      readline?.close();
    }
    if (temporaryHome) {
      rmSync(temporaryHome, { recursive: true, force: true });
    }
  }

  return report;
}

export async function runSpike(options = {}) {
  const staticReport = inspectUpstream(options);
  const report = {
    ...staticReport,
    mode: options.runLive ? 'static+live' : 'static',
    live: null,
  };

  if (options.runLive) {
    const live = await runLiveProbe(options);
    report.live = live;
    report.checks.push(...live.checks);
    report.blockers.push(...live.blockers);
    report.trace.push(...live.trace);
  } else {
    check(report, 'live:not-run', 'blocked', {
      reason: 'live app-server smoke was not requested',
      hint: '--run-live --binary <exact binary>',
    });
    addBlocker(
      report,
      'live-not-run',
      'static source inspection is not a substitute for live protocol evidence',
    );
  }

  report.status = finalStatus(report);
  if (options.report) {
    const output = resolve(options.report);
    mkdirSync(dirname(output), { recursive: true });
    writeFileSync(output, `${JSON.stringify(redactValue(report), null, 2)}\n`);
  }
  return report;
}

export function assertSelfTest() {
  const schema = {
    oneOf: [
      { properties: { method: { enum: ['initialize'] } } },
      { properties: { method: { enum: ['thread/start'] } } },
    ],
  };
  const methods = extractTopLevelMethodValues(schema);
  if (JSON.stringify(methods) !== JSON.stringify(['initialize', 'thread/start'])) {
    throw new Error(`method extraction failed: ${JSON.stringify(methods)}`);
  }

  const parsed = parseJsonLine('{"id":1,"method":"initialize","params":{}}');
  if (parsed.id !== 1 || parsed.method !== 'initialize') {
    throw new Error('JSONL parser failed');
  }
  if (parseJsonLine('') !== null) throw new Error('blank frame should be ignored');

  let malformed = false;
  try {
    parseJsonLine('not-json');
  } catch {
    malformed = true;
  }
  if (!malformed) throw new Error('malformed JSON must be rejected');

  const redacted = redactValue({
    apiKey: 'secret',
    nested: { authorization: 'bearer secret' },
    cwd: 'C:\\private\\workspace',
    method: 'thread/start',
  });
  if (
    redacted.apiKey !== '<redacted>' ||
    redacted.nested.authorization !== '<redacted>' ||
    redacted.cwd !== '<path>' ||
    redacted.method !== 'thread/start'
  ) {
    throw new Error(`redaction failed: ${JSON.stringify(redacted)}`);
  }

  return {
    status: 'self-test-pass',
    checks: [
      'top-level method extraction',
      'JSONL object parsing and malformed-frame rejection',
      'credential/path redaction',
    ],
  };
}

export function helpText() {
  return `SL-S2-10 Codex app-server upstream spike

Static:
  --upstream-dir <dir>       official Codex checkout (default: ../codex)
  --pinned-commit <sha>      exact commit to inspect
  --report <file>             write redacted JSON evidence

Live:
  --run-live                  run initialize and ephemeral thread smoke
  --binary <file>             codex executable that accepts app-server
  --skip-thread-start         only run initialize/initialized
  --run-turn-cancel           also start a real model turn and interrupt it
  --allow-live-model          required acknowledgement for --run-turn-cancel
  --codex-home <dir>          isolated Codex home with user-managed auth
  --timeout-ms <n>            per-step deadline (default: 10000)

Utility:
  --self-test                 test this harness only; not upstream evidence
  --help

The command never reads credential contents, fetches the network, or reports
an upstream PASS.`;
}

const isMain =
  process.argv[1] &&
  resolve(process.argv[1]) === fileURLToPath(import.meta.url);

if (isMain) {
  try {
    const options = parseArgs(process.argv.slice(2));
    if (options.help) {
      console.log(helpText());
      process.exit(0);
    }
    if (options.selfTest) {
      console.log(JSON.stringify(assertSelfTest(), null, 2));
      process.exit(0);
    }
    const report = await runSpike(options);
    console.log(JSON.stringify(report, null, 2));
    process.exit(report.status === 'fail' ? 1 : report.status === 'blocked' ? 3 : 0);
  } catch (error) {
    console.error(`codex app-server spike error: ${error.message}`);
    process.exit(2);
  }
}
