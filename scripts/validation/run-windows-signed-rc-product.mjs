#!/usr/bin/env bun

/**
 * Fail-closed Windows signed-RC product admission.
 *
 * The runner verifies one current-source RC root before delegating to the
 * existing installed Plugin or MiniApp product journey. It never signs,
 * downloads, copies, or manufactures release artifacts.
 *
 * Expected layout:
 *   build.noindex/windows-signed-rc/<HEAD-short>/
 *     artifacts/NomiFun_*_x64-setup.exe
 *     artifacts/nomifun-desktop.exe
 *     artifacts/NomiFun.release-lock.json
 *
 * Override the root with NOMIFUN_WINDOWS_SIGNED_RC_ROOT. The override must
 * remain below this repository's build.noindex directory.
 */

import { spawnSync } from 'node:child_process';
import {
  existsSync,
  lstatSync,
  readdirSync,
  realpathSync,
} from 'node:fs';
import { join, relative, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

import {
  readAndVerifyReleaseLock,
  resolveReleaseArtifactPath,
  sha256File,
} from '../release/release-lock.mjs';
import {
  REPO_ROOT,
  isPathWithin,
} from './run-windows-desktop-candidate-smoke.mjs';

const SOURCE_COMMIT_PATTERN = /^[0-9a-f]{40}$/;
const THUMBPRINT_PATTERN = /^[0-9a-f]{40,128}$/i;
const TARGET_TRIPLE = 'x86_64-pc-windows-msvc';
const PRODUCT_TIMEOUT_MS = 15 * 60 * 1000;
const SCOPES = Object.freeze({
  plugin_n1: 'scripts/validation/run-windows-plugin-product-candidate.mjs',
  miniapp_m1: 'scripts/validation/run-windows-miniapp-product-candidate.mjs',
});

class SignedRcFailure extends Error {
  constructor(code, message, details = {}) {
    super(message);
    this.name = 'SignedRcFailure';
    this.code = code;
    this.details = details;
  }
}

function failure(code, message, details = {}) {
  throw new SignedRcFailure(code, message, details);
}

function parseArgs(argv) {
  if (argv.length === 1 && argv[0] === '--self-test') {
    return { selfTest: true, scope: null };
  }
  if (argv.length === 2 && argv[0] === '--scope' && Object.hasOwn(SCOPES, argv[1])) {
    return { selfTest: false, scope: argv[1] };
  }
  throw new Error('usage: --self-test | --scope <plugin_n1|miniapp_m1>');
}

function cleanHead() {
  const status = spawnSync(
    'git',
    ['status', '--porcelain', '--untracked-files=no'],
    {
      cwd: REPO_ROOT,
      encoding: 'utf8',
      shell: false,
      windowsHide: true,
      stdio: 'pipe',
      timeout: 10_000,
    },
  );
  if (status.status !== 0 || status.error) {
    failure('source_checkpoint_unavailable', 'Cannot inspect the signed-RC source checkpoint');
  }
  if (String(status.stdout || '').trim()) {
    failure('source_checkpoint_dirty', 'Signed-RC admission requires a clean tracked worktree');
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
  if (head.status !== 0 || !SOURCE_COMMIT_PATTERN.test(sourceCommit)) {
    failure('source_checkpoint_invalid', 'Signed-RC source commit is unavailable or non-canonical');
  }
  return sourceCommit;
}

function validateRcRoot(sourceCommit) {
  const configured = process.env.NOMIFUN_WINDOWS_SIGNED_RC_ROOT;
  const root = configured
    ? resolve(configured)
    : join(
        REPO_ROOT,
        'build.noindex',
        'windows-signed-rc',
        sourceCommit.slice(0, 9),
      );
  const allowed = join(REPO_ROOT, 'build.noindex');
  if (!isPathWithin(allowed, root) || resolve(root) === resolve(allowed)) {
    failure('signed_rc_root_outside_build', 'Signed-RC root must be a child of build.noindex');
  }
  if (!existsSync(root) || !lstatSync(root).isDirectory() || lstatSync(root).isSymbolicLink()) {
    failure('signed_rc_root_missing', 'Current-source Windows signed-RC root is unavailable', {
      expected_root: relative(REPO_ROOT, root).replaceAll('\\', '/'),
    });
  }
  return root;
}

function oneRegularFile(directory, predicate, label) {
  if (!existsSync(directory) || !lstatSync(directory).isDirectory()) {
    failure('signed_rc_artifacts_missing', 'Signed-RC artifacts directory is unavailable');
  }
  const matches = readdirSync(directory, { withFileTypes: true })
    .filter((entry) => entry.isFile() && predicate(entry.name))
    .map((entry) => join(directory, entry.name));
  if (matches.length !== 1) {
    failure('signed_rc_artifact_ambiguous', `Expected exactly one ${label}`, {
      observed_count: matches.length,
    });
  }
  const path = matches[0];
  const metadata = lstatSync(path);
  if (metadata.isSymbolicLink() || !metadata.isFile() || metadata.size === 0) {
    failure('signed_rc_artifact_invalid', `${label} must be a non-empty regular file`);
  }
  return path;
}

export function evaluateAuthenticode(observation) {
  const status = typeof observation?.status === 'string' ? observation.status : null;
  const signerThumbprint =
    typeof observation?.signer_thumbprint === 'string'
      ? observation.signer_thumbprint.toLowerCase()
      : null;
  const timestampThumbprint =
    typeof observation?.timestamp_thumbprint === 'string'
      ? observation.timestamp_thumbprint.toLowerCase()
      : null;
  const valid =
    status === 'Valid' &&
    THUMBPRINT_PATTERN.test(signerThumbprint || '') &&
    THUMBPRINT_PATTERN.test(timestampThumbprint || '');
  return {
    status: valid ? 'pass' : 'fail',
    authenticode_status: status,
    signer_thumbprint: signerThumbprint,
    timestamp_thumbprint: timestampThumbprint,
  };
}

export function inspectAuthenticode(path) {
  const script = [
    "$target = [Environment]::GetEnvironmentVariable('NOMIFUN_SIGNATURE_TARGET')",
    "$signature = Get-AuthenticodeSignature -LiteralPath $target",
    '[PSCustomObject]@{',
    '  status = [string]$signature.Status',
    '  signer_thumbprint = if ($null -eq $signature.SignerCertificate) { $null } else { [string]$signature.SignerCertificate.Thumbprint }',
    '  timestamp_thumbprint = if ($null -eq $signature.TimeStamperCertificate) { $null } else { [string]$signature.TimeStamperCertificate.Thumbprint }',
    '} | ConvertTo-Json -Compress',
  ].join('; ');
  const result = spawnSync(
    'powershell.exe',
    ['-NoLogo', '-NoProfile', '-NonInteractive', '-Command', script],
    {
      cwd: REPO_ROOT,
      env: { ...process.env, NOMIFUN_SIGNATURE_TARGET: path },
      encoding: 'utf8',
      shell: false,
      windowsHide: true,
      stdio: 'pipe',
      timeout: 30_000,
    },
  );
  if (result.status !== 0 || result.error) {
    failure('authenticode_probe_failed', 'PowerShell could not inspect Authenticode', {
      exit_code: result.status,
      error_code: result.error?.code ?? null,
    });
  }
  let observation;
  try {
    observation = JSON.parse(String(result.stdout || ''));
  } catch {
    failure('authenticode_probe_invalid', 'Authenticode probe returned invalid JSON');
  }
  const evaluated = evaluateAuthenticode(observation);
  if (evaluated.status !== 'pass') {
    failure('authenticode_invalid', 'Signed-RC artifact lacks a valid timestamped Authenticode signature', {
      authenticode_status: evaluated.authenticode_status,
      signer_present: Boolean(evaluated.signer_thumbprint),
      timestamp_present: Boolean(evaluated.timestamp_thumbprint),
    });
  }
  return evaluated;
}

function sameFile(left, right) {
  return realpathSync.native(left).toLowerCase() === realpathSync.native(right).toLowerCase();
}

function verifyReleaseProvenance(root, sourceCommit, host, installer, lockPath) {
  const release = readAndVerifyReleaseLock(lockPath, { root });
  if (release.status !== 'pass') {
    failure('release_lock_invalid', 'Windows signed-RC release lock did not verify', {
      status: release.status,
      reason: release.reason ?? null,
    });
  }
  if (release.lock.source_commit !== sourceCommit || release.lock.platform !== TARGET_TRIPLE) {
    failure('release_lock_identity_mismatch', 'Release lock does not bind the exact source/platform cohort');
  }
  const lockedHost = resolveReleaseArtifactPath(root, release.lock.host.path);
  const lockedPackage = resolveReleaseArtifactPath(root, release.lock.package.path);
  if (!sameFile(lockedHost, host) || !sameFile(lockedPackage, installer)) {
    failure('release_lock_artifact_mismatch', 'Release lock does not identify the admitted Host and installer');
  }
  return {
    path: relative(REPO_ROOT, lockPath).replaceAll('\\', '/'),
    sha256: release.lock_sha256,
    source_commit: release.lock.source_commit,
    platform: release.lock.platform,
  };
}

function runProduct(scope, candidateRoot, sourceCommit) {
  const result = spawnSync(
    'bun',
    [SCOPES[scope], '--current-candidate'],
    {
      cwd: REPO_ROOT,
      env: {
        ...process.env,
        NOMIFUN_WINDOWS_CANDIDATE_ROOT: candidateRoot,
      },
      encoding: 'utf8',
      shell: false,
      windowsHide: true,
      stdio: 'pipe',
      timeout: PRODUCT_TIMEOUT_MS,
      maxBuffer: 16 * 1024 * 1024,
    },
  );
  let parsed = null;
  try {
    parsed = JSON.parse(String(result.stdout || ''));
  } catch {
    // The structured failure below deliberately avoids echoing child output.
  }
  if (
    result.status !== 0 ||
    result.error ||
    parsed?.status !== 'pass' ||
    parsed?.source_commit !== sourceCommit
  ) {
    failure('signed_rc_product_failed', `Signed-RC ${scope} installed product journey failed`, {
      exit_code: result.status,
      error_code: result.error?.code ?? null,
      product_status: parsed?.status ?? null,
      failed_check: parsed?.checks?.find((check) => check.status === 'fail')?.id ?? null,
    });
  }
  return {
    status: parsed.status,
    suite: parsed.suite?.name ?? null,
    check_count: Array.isArray(parsed.checks) ? parsed.checks.length : 0,
    result_root: parsed.artifacts?.result_root ?? null,
  };
}

function run(scope) {
  if (process.platform !== 'win32' || process.arch !== 'x64') {
    failure('native_host_required', 'Windows signed-RC admission requires a native Windows x64 host');
  }
  const sourceCommit = cleanHead();
  const root = validateRcRoot(sourceCommit);
  const artifactRoot = join(root, 'artifacts');
  const installer = oneRegularFile(
    artifactRoot,
    (name) => /setup\.exe$/i.test(name),
    'signed x64 setup.exe',
  );
  const host = oneRegularFile(
    artifactRoot,
    (name) => /^nomifun-desktop\.exe$/i.test(name),
    'signed nomifun-desktop.exe',
  );
  const lockPath = oneRegularFile(
    artifactRoot,
    (name) => /release-lock\.json$/i.test(name),
    'release-lock.json',
  );
  const signatures = {
    host: inspectAuthenticode(host),
    package: inspectAuthenticode(installer),
  };
  const releaseLock = verifyReleaseProvenance(
    root,
    sourceCommit,
    host,
    installer,
    lockPath,
  );
  const product = runProduct(scope, root, sourceCommit);
  return {
    schema_version: '1.0.0',
    status: 'pass',
    source_commit: sourceCommit,
    target: 'windows_desktop_x64',
    scope,
    artifacts: {
      host: {
        path: relative(REPO_ROOT, host).replaceAll('\\', '/'),
        sha256: sha256File(host),
      },
      package: {
        path: relative(REPO_ROOT, installer).replaceAll('\\', '/'),
        sha256: sha256File(installer),
      },
      release_lock: releaseLock,
    },
    signatures,
    product,
  };
}

export function assertSelfTest() {
  const valid = evaluateAuthenticode({
    status: 'Valid',
    signer_thumbprint: 'a'.repeat(40),
    timestamp_thumbprint: 'b'.repeat(40),
  });
  const unsigned = evaluateAuthenticode({
    status: 'NotSigned',
    signer_thumbprint: null,
    timestamp_thumbprint: null,
  });
  if (valid.status !== 'pass' || unsigned.status !== 'fail') {
    throw new Error('Authenticode evaluator self-test failed');
  }
  if (parseArgs(['--scope', 'plugin_n1']).scope !== 'plugin_n1') {
    throw new Error('scope parser self-test failed');
  }
  return {
    schema_version: '1.0.0',
    status: 'pass',
    suite: {
      name: 'windows-signed-rc-product-self-test',
      checks: ['scope-parser', 'timestamped-authenticode'],
    },
  };
}

const isMain = process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url);
if (isMain) {
  try {
    const options = parseArgs(process.argv.slice(2));
    const result = options.selfTest ? assertSelfTest() : run(options.scope);
    console.log(JSON.stringify(result, null, 2));
    process.exitCode = result.status === 'pass' ? 0 : 1;
  } catch (error) {
    console.log(JSON.stringify({
      schema_version: '1.0.0',
      status: 'fail',
      code: error instanceof SignedRcFailure ? error.code : 'runner_error',
      reason: error instanceof Error ? error.message : String(error),
      ...(error instanceof SignedRcFailure ? error.details : {}),
    }, null, 2));
    process.exitCode = 1;
  }
}
