#!/usr/bin/env node

import { spawnSync } from 'node:child_process';
import { createHash } from 'node:crypto';
import {
  closeSync,
  lstatSync,
  mkdirSync,
  openSync,
  readSync,
  readFileSync,
  writeFileSync,
} from 'node:fs';
import { dirname, isAbsolute, relative, resolve, sep } from 'node:path';
import { fileURLToPath } from 'node:url';

export const RELEASE_LOCK_SCHEMA_VERSION = '1.0.0';

const SHA256_PATTERN = /^[0-9a-f]{64}$/;
const SOURCE_COMMIT_PATTERN = /^[0-9a-f]{40}(?:[0-9a-f]{24})?$/;
const TOP_LEVEL_KEYS = [
  'helpers',
  'host',
  'legal',
  'package',
  'platform',
  'schema_version',
  'sidecars',
  'source_commit',
];
const ARTIFACT_KEYS = ['path', 'sha256'];

export class ReleaseLockError extends Error {
  constructor(status, message, details = {}) {
    super(message);
    this.name = 'ReleaseLockError';
    this.status = status;
    this.details = details;
  }
}

export function sha256File(path) {
  const hash = createHash('sha256');
  const descriptor = openSync(path, 'r');
  const buffer = Buffer.allocUnsafe(1024 * 1024);
  try {
    let bytesRead;
    do {
      bytesRead = readSync(descriptor, buffer, 0, buffer.length, null);
      if (bytesRead > 0) hash.update(buffer.subarray(0, bytesRead));
    } while (bytesRead > 0);
    return hash.digest('hex');
  } finally {
    closeSync(descriptor);
  }
}

function sortedKeys(value) {
  return Object.keys(value).sort();
}

function sameKeys(value, expected) {
  return (
    value &&
    typeof value === 'object' &&
    !Array.isArray(value) &&
    JSON.stringify(sortedKeys(value)) === JSON.stringify(expected)
  );
}

function portablePath(path) {
  return path.split(sep).join('/');
}

function pathInsideRoot(root, path) {
  const value = relative(root, path);
  return value !== '' && value !== '..' && !value.startsWith(`..${sep}`) && !isAbsolute(value);
}

function artifactPathForLock(root, path) {
  const absolute = resolve(path);
  if (!pathInsideRoot(root, absolute)) {
    throw new ReleaseLockError(
      'fail',
      `release artifact must be inside the artifact root: ${absolute}`,
      { artifact_root: root, path: absolute },
    );
  }
  return portablePath(relative(root, absolute));
}

export function resolveReleaseArtifactPath(root, recordedPath) {
  if (
    typeof recordedPath !== 'string' ||
    recordedPath.length === 0 ||
    isAbsolute(recordedPath) ||
    recordedPath.split('/').includes('..') ||
    recordedPath.includes('\\')
  ) {
    throw new ReleaseLockError('fail', `invalid release-lock artifact path: ${recordedPath}`);
  }
  const absolute = resolve(root, ...recordedPath.split('/'));
  if (!pathInsideRoot(resolve(root), absolute)) {
    throw new ReleaseLockError('fail', `release-lock artifact escapes its root: ${recordedPath}`);
  }
  return absolute;
}

function inspectRealFile(path) {
  let metadata;
  try {
    metadata = lstatSync(path);
  } catch (error) {
    if (error?.code === 'ENOENT') {
      return { status: 'blocked', reason: 'artifact_missing', path };
    }
    return { status: 'fail', reason: 'artifact_stat_failed', path, error: error.message };
  }
  if (metadata.isSymbolicLink()) {
    return { status: 'fail', reason: 'artifact_symlink_not_allowed', path };
  }
  if (!metadata.isFile()) {
    return { status: 'fail', reason: 'artifact_not_regular_file', path };
  }
  try {
    return { status: 'pass', path, sha256: sha256File(path) };
  } catch (error) {
    return { status: 'fail', reason: 'artifact_hash_failed', path, error: error.message };
  }
}

function createArtifactEntry(root, path, label) {
  const absolute = resolve(path);
  const inspected = inspectRealFile(absolute);
  if (inspected.status !== 'pass') {
    throw new ReleaseLockError(
      inspected.status,
      `${label} is not an available real file: ${absolute}`,
      inspected,
    );
  }
  return {
    path: artifactPathForLock(root, absolute),
    sha256: inspected.sha256,
  };
}

function validateSourceCommit(sourceCommit) {
  if (typeof sourceCommit !== 'string' || !SOURCE_COMMIT_PATTERN.test(sourceCommit)) {
    throw new ReleaseLockError(
      'fail',
      'source_commit must be a canonical lowercase Git SHA (40 or 64 hex characters)',
    );
  }
}

function validatePlatform(platform) {
  if (typeof platform !== 'string' || platform.trim() !== platform || platform.length === 0) {
    throw new ReleaseLockError('fail', 'platform must be a non-empty canonical string');
  }
}

export function createReleaseLock({
  root,
  sourceCommit,
  platform,
  host,
  sidecars,
  helpers = [],
  packagePath,
  legal = [],
}) {
  const artifactRoot = resolve(root);
  validateSourceCommit(sourceCommit);
  validatePlatform(platform);
  if (!sidecars || typeof sidecars !== 'object' || Array.isArray(sidecars)) {
    throw new ReleaseLockError('fail', 'sidecars must be a target-id keyed object');
  }
  const sidecarEntries = Object.entries(sidecars).sort(([left], [right]) =>
    left.localeCompare(right),
  );
  if (sidecarEntries.length === 0) {
    throw new ReleaseLockError('blocked', 'at least one real sidecar artifact is required');
  }

  const lockedSidecars = {};
  for (const [targetId, path] of sidecarEntries) {
    if (typeof targetId !== 'string' || targetId.length === 0) {
      throw new ReleaseLockError('fail', 'sidecar target id must be a non-empty string');
    }
    lockedSidecars[targetId] = createArtifactEntry(
      artifactRoot,
      path,
      `sidecar ${targetId}`,
    );
  }

  const lock = {
    schema_version: RELEASE_LOCK_SCHEMA_VERSION,
    source_commit: sourceCommit,
    platform,
    host: createArtifactEntry(artifactRoot, host, 'host'),
    sidecars: lockedSidecars,
    helpers: helpers
      .map((path) => createArtifactEntry(artifactRoot, path, 'helper'))
      .sort((left, right) => left.path.localeCompare(right.path)),
    package: createArtifactEntry(artifactRoot, packagePath, 'package'),
    legal: legal
      .map((path) => createArtifactEntry(artifactRoot, path, 'legal artifact'))
      .sort((left, right) => left.path.localeCompare(right.path)),
  };
  validateReleaseLockShape(lock);
  return lock;
}

function validateArtifactShape(value, label) {
  if (!sameKeys(value, ARTIFACT_KEYS)) {
    throw new ReleaseLockError('fail', `${label} must contain only path and sha256`);
  }
  if (typeof value.path !== 'string' || value.path.length === 0) {
    throw new ReleaseLockError('fail', `${label}.path must be a non-empty string`);
  }
  if (typeof value.sha256 !== 'string' || !SHA256_PATTERN.test(value.sha256)) {
    throw new ReleaseLockError('fail', `${label}.sha256 must be canonical lowercase SHA-256`);
  }
}

export function validateReleaseLockShape(lock) {
  if (!sameKeys(lock, TOP_LEVEL_KEYS)) {
    throw new ReleaseLockError(
      'fail',
      `release lock must contain exactly: ${TOP_LEVEL_KEYS.join(', ')}`,
    );
  }
  if (lock.schema_version !== RELEASE_LOCK_SCHEMA_VERSION) {
    throw new ReleaseLockError(
      'fail',
      `unsupported release lock schema_version: ${lock.schema_version}`,
    );
  }
  validateSourceCommit(lock.source_commit);
  validatePlatform(lock.platform);
  validateArtifactShape(lock.host, 'host');
  validateArtifactShape(lock.package, 'package');
  if (
    !lock.sidecars ||
    typeof lock.sidecars !== 'object' ||
    Array.isArray(lock.sidecars) ||
    Object.keys(lock.sidecars).length === 0
  ) {
    throw new ReleaseLockError('fail', 'sidecars must contain at least one target');
  }
  for (const [targetId, artifact] of Object.entries(lock.sidecars)) {
    if (!targetId) throw new ReleaseLockError('fail', 'sidecar target id must not be empty');
    validateArtifactShape(artifact, `sidecars.${targetId}`);
  }
  for (const [field, entries] of [
    ['helpers', lock.helpers],
    ['legal', lock.legal],
  ]) {
    if (!Array.isArray(entries)) {
      throw new ReleaseLockError('fail', `${field} must be an array`);
    }
    entries.forEach((artifact, index) =>
      validateArtifactShape(artifact, `${field}[${index}]`),
    );
  }
  return lock;
}

function lockedArtifacts(lock) {
  return [
    ['host', lock.host],
    ...Object.entries(lock.sidecars).map(([targetId, artifact]) => [
      `sidecars.${targetId}`,
      artifact,
    ]),
    ...lock.helpers.map((artifact, index) => [`helpers[${index}]`, artifact]),
    ['package', lock.package],
    ...lock.legal.map((artifact, index) => [`legal[${index}]`, artifact]),
  ];
}

export function verifyReleaseLock(lock, { root }) {
  const artifactRoot = resolve(root);
  try {
    validateReleaseLockShape(lock);
  } catch (error) {
    return {
      status: error instanceof ReleaseLockError ? error.status : 'fail',
      reason: 'invalid_release_lock',
      error: error.message,
      checks: [],
    };
  }

  const checks = [];
  for (const [id, artifact] of lockedArtifacts(lock)) {
    let path;
    try {
      path = resolveReleaseArtifactPath(artifactRoot, artifact.path);
    } catch (error) {
      checks.push({
        id,
        status: 'fail',
        reason: 'invalid_artifact_path',
        path: artifact.path,
        error: error.message,
      });
      continue;
    }
    const inspected = inspectRealFile(path);
    if (inspected.status !== 'pass') {
      checks.push({ id, ...inspected, expected_sha256: artifact.sha256 });
      continue;
    }
    checks.push({
      id,
      status: inspected.sha256 === artifact.sha256 ? 'pass' : 'fail',
      reason: inspected.sha256 === artifact.sha256 ? undefined : 'digest_mismatch',
      path,
      expected_sha256: artifact.sha256,
      observed_sha256: inspected.sha256,
    });
  }
  const status = checks.some((entry) => entry.status === 'fail')
    ? 'fail'
    : checks.some((entry) => entry.status === 'blocked')
      ? 'blocked'
      : 'pass';
  return { status, checks };
}

export function readAndVerifyReleaseLock(lockPath, { root }) {
  const absoluteLockPath = resolve(lockPath);
  const inspected = inspectRealFile(absoluteLockPath);
  if (inspected.status !== 'pass') {
    return {
      status: inspected.status,
      reason: inspected.reason === 'artifact_missing' ? 'release_lock_missing' : inspected.reason,
      lock_path: absoluteLockPath,
      checks: [],
    };
  }
  let lock;
  try {
    lock = JSON.parse(readFileSync(absoluteLockPath, 'utf8'));
  } catch (error) {
    return {
      status: 'fail',
      reason: 'release_lock_invalid_json',
      error: error.message,
      lock_path: absoluteLockPath,
      checks: [],
    };
  }
  const verification = verifyReleaseLock(lock, { root });
  return {
    ...verification,
    lock,
    lock_path: absoluteLockPath,
    lock_sha256: inspected.sha256,
  };
}

export function writeReleaseLock(outputPath, lock) {
  validateReleaseLockShape(lock);
  const absolute = resolve(outputPath);
  mkdirSync(dirname(absolute), { recursive: true });
  writeFileSync(absolute, `${JSON.stringify(lock, null, 2)}\n`);
  return {
    path: absolute,
    sha256: sha256File(absolute),
  };
}

function gitSourceCommit(root) {
  const worktree = spawnSync(
    'git',
    ['-C', root, 'status', '--porcelain', '--untracked-files=no'],
    {
      encoding: 'utf8',
      shell: false,
      stdio: 'pipe',
    },
  );
  if (worktree.status !== 0) {
    throw new ReleaseLockError(
      'blocked',
      `cannot inspect source worktree: ${String(worktree.stderr || worktree.error?.message || '').trim()}`,
    );
  }
  if (String(worktree.stdout || '').trim().length > 0) {
    throw new ReleaseLockError(
      'blocked',
      'cannot attest source_commit from a dirty tracked worktree',
    );
  }
  const result = spawnSync('git', ['-C', root, 'rev-parse', 'HEAD'], {
    encoding: 'utf8',
    shell: false,
    stdio: 'pipe',
  });
  if (result.status !== 0) {
    throw new ReleaseLockError(
      'blocked',
      `cannot determine source commit: ${String(result.stderr || result.error?.message || '').trim()}`,
    );
  }
  const sourceCommit = String(result.stdout || '').trim();
  validateSourceCommit(sourceCommit);
  return sourceCommit;
}

function parseCli(argv) {
  const [command, ...tokens] = argv;
  if (!['create', 'verify'].includes(command)) {
    throw new ReleaseLockError(
      'fail',
      'usage: release-lock.mjs <create|verify> [options]',
    );
  }
  const single = {};
  const repeated = { sidecar: [], helper: [], legal: [] };
  for (let index = 0; index < tokens.length; index += 1) {
    const token = tokens[index];
    const match = token.match(/^--([^=]+)(?:=(.*))?$/);
    if (!match) throw new ReleaseLockError('fail', `unknown argument: ${token}`);
    const key = match[1];
    let value = match[2];
    if (value === undefined) {
      value = tokens[++index];
      if (!value || value.startsWith('--')) {
        throw new ReleaseLockError('fail', `${token} requires a value`);
      }
    }
    if (key in repeated) repeated[key].push(value);
    else if (['root', 'source-commit', 'platform', 'host', 'package', 'output', 'lock'].includes(key)) {
      if (single[key] !== undefined) {
        throw new ReleaseLockError('fail', `${token} may only be specified once`);
      }
      single[key] = value;
    } else {
      throw new ReleaseLockError('fail', `unknown argument: ${token}`);
    }
  }
  return { command, single, repeated };
}

function required(value, name) {
  if (!value) throw new ReleaseLockError('blocked', `missing required --${name}`);
  return value;
}

function parseSidecars(values) {
  const sidecars = {};
  for (const value of values) {
    const separator = value.indexOf('=');
    if (separator <= 0 || separator === value.length - 1) {
      throw new ReleaseLockError('fail', '--sidecar must use <target_id>=<path>');
    }
    const targetId = value.slice(0, separator);
    if (sidecars[targetId]) {
      throw new ReleaseLockError('fail', `duplicate sidecar target id: ${targetId}`);
    }
    sidecars[targetId] = value.slice(separator + 1);
  }
  return sidecars;
}

function runCli(argv) {
  const { command, single, repeated } = parseCli(argv);
  const root = resolve(single.root || process.cwd());
  if (command === 'create') {
    const lock = createReleaseLock({
      root,
      sourceCommit: single['source-commit'] || gitSourceCommit(root),
      platform: required(single.platform, 'platform'),
      host: required(single.host, 'host'),
      sidecars: parseSidecars(repeated.sidecar),
      helpers: repeated.helper,
      packagePath: required(single.package, 'package'),
      legal: repeated.legal,
    });
    const written = writeReleaseLock(required(single.output, 'output'), lock);
    return { status: 'pass', release_lock: written, lock };
  }
  return readAndVerifyReleaseLock(required(single.lock, 'lock'), { root });
}

const isMain = process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url);
if (isMain) {
  try {
    const result = runCli(process.argv.slice(2));
    console.log(JSON.stringify(result, null, 2));
    process.exit(result.status === 'pass' ? 0 : result.status === 'blocked' ? 3 : 1);
  } catch (error) {
    const status = error instanceof ReleaseLockError ? error.status : 'fail';
    console.error(JSON.stringify({
      status,
      reason: 'release_lock_command_failed',
      error: error.message,
      details: error instanceof ReleaseLockError ? error.details : {},
    }, null, 2));
    process.exit(status === 'blocked' ? 3 : 1);
  }
}
