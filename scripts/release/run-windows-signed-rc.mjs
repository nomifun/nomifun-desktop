#!/usr/bin/env bun

/**
 * Build and admit one immutable Windows x64 signed RC cohort.
 *
 * This orchestrator intentionally stays serial: signed build -> Authenticode
 * admission -> atomic staging/release lock -> live StepFun smoke -> combined
 * Signed RC Gate. It never creates a certificate or overwrites an RC root.
 */

import { spawnSync } from 'node:child_process';
import { randomUUID } from 'node:crypto';
import {
  copyFileSync,
  existsSync,
  lstatSync,
  mkdirSync,
  readdirSync,
  renameSync,
  rmSync,
} from 'node:fs';
import { basename, join, relative, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

import {
  createReleaseLock,
  readAndVerifyReleaseLock,
  writeReleaseLock,
} from './release-lock.mjs';
import { inspectAuthenticode } from '../validation/run-windows-signed-rc-product.mjs';
import {
  REPO_ROOT,
  isPathWithin,
} from '../validation/run-windows-desktop-candidate-smoke.mjs';

const TARGET_TRIPLE = 'x86_64-pc-windows-msvc';
const SOURCE_PATTERN = /^[0-9a-f]{40}$/;
const GLOBAL_TIMEOUT_MS = 60 * 60 * 1000;

class SignedRcBuildFailure extends Error {
  constructor(code, message, details = {}) {
    super(message);
    this.name = 'SignedRcBuildFailure';
    this.code = code;
    this.details = details;
  }
}

function failure(code, message, details = {}) {
  throw new SignedRcBuildFailure(code, message, details);
}

function runCommand(command, args, timeoutMs = GLOBAL_TIMEOUT_MS) {
  const result = spawnSync(command, args, {
    cwd: REPO_ROOT,
    encoding: 'utf8',
    shell: false,
    windowsHide: true,
    stdio: 'inherit',
    timeout: timeoutMs,
  });
  if (result.status !== 0 || result.error) {
    failure('signed_rc_command_failed', `${command} ${args.join(' ')} failed`, {
      exit_code: result.status,
      error_code: result.error?.code ?? null,
    });
  }
}

function currentCleanSource() {
  const status = spawnSync('git', ['status', '--porcelain', '--untracked-files=no'], {
    cwd: REPO_ROOT,
    encoding: 'utf8',
    shell: false,
    windowsHide: true,
    stdio: 'pipe',
    timeout: 10_000,
  });
  if (status.status !== 0 || String(status.stdout || '').trim()) {
    failure('signed_rc_source_dirty', 'Signed RC build requires a clean tracked worktree');
  }
  const head = spawnSync('git', ['rev-parse', 'HEAD'], {
    cwd: REPO_ROOT,
    encoding: 'utf8',
    shell: false,
    windowsHide: true,
    stdio: 'pipe',
    timeout: 10_000,
  });
  const sourceCommit = String(head.stdout || '').trim().toLowerCase();
  if (head.status !== 0 || !SOURCE_PATTERN.test(sourceCommit)) {
    failure('signed_rc_source_invalid', 'Cannot resolve a canonical source commit');
  }
  return sourceCommit;
}

export function signedRcPaths(sourceCommit, repoRoot = REPO_ROOT) {
  if (!SOURCE_PATTERN.test(sourceCommit)) {
    throw new Error('sourceCommit must be a 40-character lowercase Git SHA');
  }
  const parent = join(repoRoot, 'build.noindex', 'windows-signed-rc');
  const root = join(parent, sourceCommit.slice(0, 9));
  return {
    parent,
    root,
    artifactRoot: join(root, 'artifacts'),
    lock: join(root, 'artifacts', 'NomiFun.release-lock.json'),
  };
}

function oneBuiltInstaller() {
  const directory = join(REPO_ROOT, 'dist', 'desktop');
  const matches = existsSync(directory)
    ? readdirSync(directory, { withFileTypes: true })
        .filter((entry) => entry.isFile() && /_x64-setup\.exe$/i.test(entry.name))
        .map((entry) => join(directory, entry.name))
    : [];
  if (matches.length !== 1) {
    failure('signed_rc_installer_ambiguous', 'Expected exactly one built x64 setup.exe', {
      observed_count: matches.length,
    });
  }
  return matches[0];
}

function requireRegularFile(path, label) {
  if (!existsSync(path)) failure('signed_rc_artifact_missing', `${label} is missing`);
  const metadata = lstatSync(path);
  if (metadata.isSymbolicLink() || !metadata.isFile() || metadata.size === 0) {
    failure('signed_rc_artifact_invalid', `${label} must be a non-empty regular file`);
  }
  return path;
}

function stageArtifacts(sourceCommit, hostInput, packageInput) {
  const paths = signedRcPaths(sourceCommit);
  if (!isPathWithin(join(REPO_ROOT, 'build.noindex'), paths.root)) {
    failure('signed_rc_output_boundary', 'Signed RC output escaped build.noindex');
  }
  if (existsSync(paths.root)) {
    failure('signed_rc_already_exists', 'Signed RC root already exists and will not be overwritten', {
      root: relative(REPO_ROOT, paths.root).replaceAll('\\', '/'),
    });
  }
  mkdirSync(paths.parent, { recursive: true });
  const staging = join(paths.parent, `.${sourceCommit.slice(0, 9)}.staging-${randomUUID()}`);
  const artifactRoot = join(staging, 'artifacts');
  mkdirSync(artifactRoot, { recursive: false });
  try {
    const host = join(artifactRoot, 'nomifun-desktop.exe');
    const packagePath = join(artifactRoot, basename(packageInput));
    const license = join(artifactRoot, 'LICENSE');
    const notice = join(artifactRoot, 'NOTICE');
    copyFileSync(hostInput, host);
    copyFileSync(packageInput, packagePath);
    copyFileSync(join(REPO_ROOT, 'LICENSE'), license);
    copyFileSync(join(REPO_ROOT, 'NOTICE'), notice);
    const lock = createReleaseLock({
      root: staging,
      sourceCommit,
      platform: TARGET_TRIPLE,
      host,
      sidecars: {},
      packagePath,
      legal: [license, notice],
    });
    writeReleaseLock(join(artifactRoot, 'NomiFun.release-lock.json'), lock);
    renameSync(staging, paths.root);
  } catch (error) {
    rmSync(staging, { recursive: true, force: true });
    throw error;
  }
  const verified = readAndVerifyReleaseLock(paths.lock, { root: paths.root });
  if (verified.status !== 'pass') {
    failure('signed_rc_staged_lock_invalid', 'Published signed RC release lock did not verify');
  }
  return paths;
}

function run() {
  if (process.platform !== 'win32' || process.arch !== 'x64') {
    failure('signed_rc_native_host_required', 'Windows signed RC build requires Windows x64');
  }
  if (!process.env.WINDOWS_CERTIFICATE_THUMBPRINT?.trim()) {
    failure('signed_rc_certificate_missing', 'WINDOWS_CERTIFICATE_THUMBPRINT is required');
  }
  const sourceCommit = currentCleanSource();
  const planned = signedRcPaths(sourceCommit);
  if (existsSync(planned.root)) {
    failure('signed_rc_already_exists', 'Signed RC root already exists and will not be overwritten');
  }

  runCommand('bun', ['run', 'build:win', 'x64', '--signed']);
  const hostInput = requireRegularFile(
    join(REPO_ROOT, 'target', TARGET_TRIPLE, 'release', 'nomifun-desktop.exe'),
    'signed Host',
  );
  const packageInput = requireRegularFile(oneBuiltInstaller(), 'signed NSIS package');
  inspectAuthenticode(hostInput);
  inspectAuthenticode(packageInput);
  const paths = stageArtifacts(sourceCommit, hostInput, packageInput);

  runCommand(
    'powershell.exe',
    [
      '-NoLogo',
      '-NoProfile',
      '-ExecutionPolicy',
      'Bypass',
      '-File',
      'scripts/validation/run-nomi-core-live-provider-from-windows-credential-manager.ps1',
    ],
    30 * 60 * 1000,
  );
  runCommand('bun', [
    'run',
    'gate:plugin-n1',
    '--',
    '--stage',
    'windows_signed_rc',
    '--scope',
    'combined',
    '--cohort',
    `rc-win-${sourceCommit.slice(0, 9)}`,
  ]);

  return {
    schema_version: '1.0.0',
    status: 'pass',
    source_commit: sourceCommit,
    target: 'windows_desktop_x64',
    signed_rc_root: relative(REPO_ROOT, paths.root).replaceAll('\\', '/'),
    release_lock: relative(REPO_ROOT, paths.lock).replaceAll('\\', '/'),
    gate_cohort: `rc-win-${sourceCommit.slice(0, 9)}`,
  };
}

export function assertSelfTest() {
  const sourceCommit = 'a'.repeat(40);
  const paths = signedRcPaths(sourceCommit, 'C:\\repo');
  if (
    !paths.root.endsWith(join('windows-signed-rc', 'aaaaaaaaa')) ||
    !paths.lock.endsWith(join('artifacts', 'NomiFun.release-lock.json'))
  ) {
    throw new Error('signed RC path plan self-test failed');
  }
  return {
    schema_version: '1.0.0',
    status: 'pass',
    suite: {
      name: 'windows-signed-rc-orchestrator-self-test',
      checks: ['immutable-root-plan', 'release-lock-path'],
    },
  };
}

const isMain = process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url);
if (isMain) {
  try {
    const args = process.argv.slice(2);
    if (args.length > 1 || (args.length === 1 && args[0] !== '--self-test')) {
      throw new Error('usage: --self-test | <no arguments>');
    }
    const result = args[0] === '--self-test' ? assertSelfTest() : run();
    console.log(JSON.stringify(result, null, 2));
    process.exitCode = result.status === 'pass' ? 0 : 1;
  } catch (error) {
    console.log(JSON.stringify({
      schema_version: '1.0.0',
      status: 'fail',
      code: error instanceof SignedRcBuildFailure ? error.code : 'runner_error',
      reason: error instanceof Error ? error.message : String(error),
      ...(error instanceof SignedRcBuildFailure ? error.details : {}),
    }, null, 2));
    process.exitCode = 1;
  }
}
