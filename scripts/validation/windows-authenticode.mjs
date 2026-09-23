#!/usr/bin/env bun

import { spawnSync } from 'node:child_process';

import { REPO_ROOT } from './run-windows-desktop-candidate-smoke.mjs';

const THUMBPRINT_PATTERN = /^[0-9a-f]{40,128}$/i;

export class AuthenticodeError extends Error {
  constructor(code, message, details = {}) {
    super(message);
    this.name = 'AuthenticodeError';
    this.code = code;
    this.details = details;
  }
}

export function evaluateAuthenticode(observation) {
  const status = typeof observation?.status === 'string' ? observation.status : null;
  const signerThumbprint = typeof observation?.signer_thumbprint === 'string'
    ? observation.signer_thumbprint.toLowerCase()
    : null;
  const timestampThumbprint = typeof observation?.timestamp_thumbprint === 'string'
    ? observation.timestamp_thumbprint.toLowerCase()
    : null;
  const valid = status === 'Valid' &&
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
    '$signature = Get-AuthenticodeSignature -LiteralPath $target',
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
    throw new AuthenticodeError(
      'authenticode_probe_failed',
      'PowerShell could not inspect Authenticode',
      { exit_code: result.status, error_code: result.error?.code ?? null },
    );
  }
  let observation;
  try {
    observation = JSON.parse(String(result.stdout || ''));
  } catch {
    throw new AuthenticodeError(
      'authenticode_probe_invalid',
      'Authenticode probe returned invalid JSON',
    );
  }
  const evaluated = evaluateAuthenticode(observation);
  if (evaluated.status !== 'pass') {
    throw new AuthenticodeError(
      'authenticode_invalid',
      'Signed artifact lacks a valid timestamped Authenticode signature',
      {
        authenticode_status: evaluated.authenticode_status,
        signer_present: Boolean(evaluated.signer_thumbprint),
        timestamp_present: Boolean(evaluated.timestamp_thumbprint),
      },
    );
  }
  return evaluated;
}
