#!/usr/bin/env bun

/**
 * Installed Windows MiniApp M1 product acceptance for the current commit.
 * Reuses the hardened NSIS candidate harness and the common product/CDP
 * helpers exercised by the Plugin N1 candidate.
 */

import { spawnSync } from 'node:child_process';
import {
  existsSync,
  mkdirSync,
  readFileSync,
} from 'node:fs';
import { join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

import {
  REPO_ROOT,
  SmokeFailure,
  runCandidateSmoke,
} from './run-windows-desktop-candidate-smoke.mjs';
import {
  auditInteractiveNames,
  canonicalJson,
  capturePage,
  connectToProductPage,
  productApi,
  resolveCurrentCandidate,
  sha256,
  waitForPageSelector,
} from './run-windows-plugin-product-candidate.mjs';

const UI_NAME = 'Installed UI MiniApp';
const SERVICE_NAME = 'Installed Service MiniApp';
const EMPTY_INPUT_DIGEST = sha256('');
const PRODUCT_TIMEOUT_MS = 6 * 60 * 1000;
const POLL_INTERVAL_MS = 250;

function failure(code, message, details = {}) {
  throw new SmokeFailure(code, message, details);
}

function sleep(milliseconds) {
  return new Promise((resolvePromise) => setTimeout(resolvePromise, milliseconds));
}

function requireString(value, code, message) {
  if (typeof value !== 'string' || value.length === 0) failure(code, message);
  return value;
}

function requireInteger(value, code, message) {
  if (!Number.isSafeInteger(value) || value < 0) failure(code, message);
  return value;
}

function readJson(path) {
  if (!existsSync(path)) failure('candidate_artifact_missing', `missing Candidate artifact ${path}`);
  return JSON.parse(readFileSync(path, 'utf8'));
}

async function library(context, phase) {
  const value = await productApi(context, '/api/miniapps', { phase });
  requireInteger(value?.library_revision, 'miniapp_library_revision_missing', 'MiniApp Library revision is missing');
  if (!Array.isArray(value?.miniapps)) failure('miniapp_library_invalid', 'MiniApp Library items are invalid');
  return value;
}

async function workshop(context, miniappId, phase) {
  return productApi(
    context,
    `/api/miniapps/${encodeURIComponent(miniappId)}/workshop`,
    { phase },
  );
}

async function editUiSource(context, current) {
  const path = 'ui/index.html';
  const source = await productApi(
    context,
    `/api/miniapps/${encodeURIComponent(current.miniapp.miniapp_id)}/source/files/${encodeURIComponent(path)}`,
    { phase: 'miniapp.ui.source.read' },
  );
  const previousDigest = requireString(
    source?.source_snapshot_digest,
    'miniapp_source_file_digest_missing',
    'MiniApp Source file response omitted its exact snapshot digest',
  );
  const previousGeneration = requireInteger(
    source?.build_generation,
    'miniapp_source_file_generation_missing',
    'MiniApp Source file response omitted its build generation',
  );
  const content = requireString(
    source?.content,
    'miniapp_source_file_content_missing',
    'MiniApp Source file response omitted its text content',
  );
  const edited = await productApi(
    context,
    `/api/miniapps/${encodeURIComponent(current.miniapp.miniapp_id)}/source/edit`,
    {
      method: 'POST',
      phase: 'miniapp.ui.source.edit',
      body: {
        miniapp_id: current.miniapp.miniapp_id,
        expected_product_revision: current.miniapp.product_revision,
        project_id: current.project_id,
        expected_project_revision: current.project_revision,
        expected_build_generation: previousGeneration,
        expected_source_snapshot_digest: previousDigest,
        path,
        content: `${content}\n<!-- installed-candidate-v2 -->\n`,
      },
    },
  );
  if (
    edited.build_generation !== previousGeneration + 1 ||
    edited.source_snapshot_digest === previousDigest
  ) {
    failure('miniapp_source_edit_not_committed', 'Source edit did not advance the exact Project generation and digest');
  }
  return edited;
}

async function createMiniApp(context, kind, displayName) {
  let created = null;
  for (let attempt = 1; attempt <= 2; attempt += 1) {
    const current = await library(
      context,
      `miniapp.${kind}.library_before_create.${attempt}`,
    );
    try {
      created = await productApi(context, '/api/miniapps/projects', {
        method: 'POST',
        phase: `miniapp.${kind}.create`,
        body: {
          expected_library_revision: current.library_revision,
          display_name: displayName,
          description: `Installed ${kind} MiniApp Candidate fixture.`,
          kind,
        },
      });
      break;
    } catch (error) {
      const retryableCas =
        attempt === 1 &&
        error instanceof SmokeFailure &&
        error.code === 'product_api_status' &&
        error.details?.status_code === 409;
      if (!retryableCas) throw error;
      await sleep(100);
    }
  }
  if (!created) {
    failure('miniapp_create_retry_exhausted', `Could not create ${kind} MiniApp after an exact Library refresh`);
  }
  if (
    created?.miniapp?.kind !== kind ||
    created?.source_state !== 'editable' ||
    created?.build_generation !== 1
  ) {
    failure('miniapp_create_projection_invalid', `Created ${kind} MiniApp projection is invalid`);
  }
  return created;
}

async function buildMiniApp(context, current, serviceLifecycle = null) {
  const built = await productApi(
    context,
    `/api/miniapps/${encodeURIComponent(current.miniapp.miniapp_id)}/build`,
    {
      method: 'POST',
      timeoutMs: 180_000,
      phase: `miniapp.${current.miniapp.kind}.build`,
      body: {
        miniapp_id: current.miniapp.miniapp_id,
        expected_product_revision: current.miniapp.product_revision,
        project_id: current.project_id,
        expected_project_revision: current.project_revision,
        expected_build_generation: current.build_generation,
        expected_source_snapshot_digest: requireString(
          current.source_snapshot_digest,
          'miniapp_source_digest_missing',
          'MiniApp Source digest is missing before Build',
        ),
        expected_dependency_lock_digest: requireString(
          current.dependency_lock_digest,
          'miniapp_lock_digest_missing',
          'MiniApp dependency lock digest is missing before Build',
        ),
        ...(serviceLifecycle ? { service_lifecycle: serviceLifecycle } : {}),
      },
    },
  );
  if (!built.ready?.release || built.miniapp.releases.ready?.release_id !== built.ready.release.release_id) {
    failure('miniapp_ready_release_missing', 'MiniApp Build did not publish the exact Ready Release');
  }
  return built;
}

async function testServiceReady(context, current) {
  const ready = current.ready;
  const tested = await productApi(
    context,
    `/api/miniapps/${encodeURIComponent(current.miniapp.miniapp_id)}/test`,
    {
      method: 'POST',
      timeoutMs: 180_000,
      phase: 'miniapp.service.test',
      body: {
        miniapp_id: current.miniapp.miniapp_id,
        expected_product_revision: current.miniapp.product_revision,
        expected_pointer_revision: current.miniapp.releases.pointer_revision,
        project_id: current.project_id,
        expected_project_revision: current.project_revision,
        expected_build_generation: current.build_generation,
        release_id: ready.release.release_id,
        expected_release_digest: ready.release.release_digest,
        expected_config_revision: current.config.config_revision,
        expected_credential_bindings_revision: current.credential_bindings_revision,
        resolved_test_input_digest: EMPTY_INPUT_DIGEST,
      },
    },
  );
  if (tested.ready?.test?.status !== 'passed' || !tested.ready.test.receipt_id) {
    failure('miniapp_service_test_failed', 'Default Service Ready Test did not pass', {
      status: tested.ready?.test?.status ?? null,
      error_code: tested.ready?.test?.error_code ?? null,
    });
  }
  return tested;
}

async function publishReady(context, current) {
  const ready = current.ready;
  const active = current.miniapp.releases.active;
  const published = await productApi(
    context,
    `/api/miniapps/${encodeURIComponent(current.miniapp.miniapp_id)}/publish`,
    {
      method: 'POST',
      timeoutMs: 180_000,
      phase: `miniapp.${current.miniapp.kind}.publish`,
      body: {
        miniapp_id: current.miniapp.miniapp_id,
        expected_product_revision: current.miniapp.product_revision,
        expected_pointer_revision: current.miniapp.releases.pointer_revision,
        expected_active_release_epoch: current.miniapp.releases.active_release_epoch,
        ready_release_id: ready.release.release_id,
        expected_ready_release_digest: ready.release.release_digest,
        ...(active ? { expected_active_release_digest: active.release_digest } : {}),
        ...(current.miniapp.kind === 'service' && ready.test.receipt_id
          ? { expected_service_test_receipt_id: ready.test.receipt_id }
          : {}),
        acknowledge_test_warning:
          current.miniapp.kind === 'service' && ready.test.status !== 'passed',
      },
    },
  );
  if (
    published.miniapp.releases.active?.release_id !== ready.release.release_id ||
    published.miniapp.releases.ready !== undefined
  ) {
    failure('miniapp_publish_rotation_failed', 'MiniApp Publish did not move Ready to Active exactly');
  }
  return published;
}

async function openSurface(context, current, phase) {
  const descriptor = await productApi(
    context,
    `/api/miniapps/${encodeURIComponent(current.miniapp.miniapp_id)}/surface/open`,
    {
      method: 'POST',
      phase,
      body: { miniapp_id: current.miniapp.miniapp_id },
    },
  );
  if (
    descriptor.miniapp_id !== current.miniapp.miniapp_id ||
    descriptor.release_id !== current.miniapp.releases.active?.release_id ||
    descriptor.active_release_epoch !== current.miniapp.releases.active_release_epoch
  ) {
    failure('miniapp_surface_descriptor_mismatch', 'Surface descriptor does not bind the exact Active Release');
  }
  return descriptor;
}

function surfaceAssetPath(descriptor) {
  const entrypoint = String(descriptor.ui_entrypoint || '');
  const segments = entrypoint.split('/');
  if (
    entrypoint.startsWith('/') ||
    entrypoint.includes('\\') ||
    segments.some((segment) => !segment || segment === '.' || segment === '..')
  ) {
    failure('miniapp_surface_entrypoint_invalid', 'Surface entrypoint is not a canonical relative path');
  }
  return `/api/miniapps/${encodeURIComponent(descriptor.miniapp_id)}/surface/assets/${encodeURIComponent(
    descriptor.surface_capability,
  )}/${descriptor.active_release_epoch}/${encodeURIComponent(
    descriptor.expected_release_digest,
  )}/${segments.map(encodeURIComponent).join('/')}`;
}

async function surfaceAsset(context, descriptor, expectedStatus = 200) {
  const path = surfaceAssetPath(descriptor);
  const response = await fetch(`${context.getBaseUrl()}${path}`, {
    method: 'GET',
    redirect: 'error',
  });
  const body = await response.text();
  if (response.status !== expectedStatus) {
    failure('miniapp_surface_asset_status', 'Surface asset returned an unexpected status', {
      status_code: response.status,
      body_sha256: sha256(body),
    });
  }
  return { path, body, body_sha256: sha256(body) };
}

async function surfaceBridge(context, descriptor, callId, target, phase, expected = [200]) {
  return productApi(
    context,
    `/api/miniapps/${encodeURIComponent(descriptor.miniapp_id)}/surface/bridge`,
    {
      method: 'POST',
      phase,
      expected,
      raw: expected.some((status) => status !== 200),
      body: {
        surface_capability: descriptor.surface_capability,
        active_release_epoch: descriptor.active_release_epoch,
        expected_release_digest: descriptor.expected_release_digest,
        request: { call_id: callId, target },
      },
    },
  );
}

async function closeSurface(context, descriptor, phase) {
  const closed = await productApi(
    context,
    `/api/miniapps/${encodeURIComponent(descriptor.miniapp_id)}/surface/close`,
    {
      method: 'POST',
      phase,
      body: {
        miniapp_id: descriptor.miniapp_id,
        surface_session_id: descriptor.surface_session_id,
        surface_capability: descriptor.surface_capability,
      },
    },
  );
  if (closed !== true) failure('miniapp_surface_close_failed', 'Surface session did not close exactly');
}

async function setEnabled(context, current, enabled) {
  return productApi(
    context,
    `/api/miniapps/${encodeURIComponent(current.miniapp.miniapp_id)}/enabled`,
    {
      method: 'POST',
      phase: `miniapp.lifecycle.${enabled ? 'enable' : 'disable'}`,
      body: {
        miniapp_id: current.miniapp.miniapp_id,
        expected_product_revision: current.miniapp.product_revision,
        expected_pointer_revision: current.miniapp.releases.pointer_revision,
        ...(current.miniapp.releases.active
          ? { expected_active_release_digest: current.miniapp.releases.active.release_digest }
          : {}),
        enabled,
      },
    },
  );
}

async function trash(context, current) {
  return productApi(
    context,
    `/api/miniapps/${encodeURIComponent(current.miniapp.miniapp_id)}/trash`,
    {
      method: 'POST',
      phase: 'miniapp.lifecycle.trash',
      body: {
        miniapp_id: current.miniapp.miniapp_id,
        expected_product_revision: current.miniapp.product_revision,
        expected_pointer_revision: current.miniapp.releases.pointer_revision,
        ...(current.miniapp.releases.active
          ? { expected_active_release_digest: current.miniapp.releases.active.release_digest }
          : {}),
      },
    },
  );
}

async function restore(context, current) {
  return productApi(
    context,
    `/api/miniapps/${encodeURIComponent(current.miniapp.miniapp_id)}/restore`,
    {
      method: 'POST',
      phase: 'miniapp.lifecycle.restore',
      body: {
        miniapp_id: current.miniapp.miniapp_id,
        expected_product_revision: current.miniapp.product_revision,
        expected_lifecycle: 'trashed',
        expected_pointer_revision: current.miniapp.releases.pointer_revision,
      },
    },
  );
}

async function checkUiLifecycleTransfer(context, state) {
  let current = await createMiniApp(context, 'ui_only', UI_NAME);
  current = await buildMiniApp(context, current);
  const first = structuredClone(current.ready.release);
  current = await publishReady(context, current);
  if (current.miniapp.lifecycle === 'disabled') {
    current = await setEnabled(context, current, true);
  }

  const descriptor = await openSurface(context, current, 'miniapp.ui.surface.open');
  const asset = await surfaceAsset(context, descriptor);
  if (!asset.body.includes(UI_NAME)) {
    failure('miniapp_surface_content_missing', 'UI Surface entrypoint did not contain the Project display name');
  }
  await surfaceBridge(
    context,
    descriptor,
    'candidate-ui-kv-set',
    { target: 'host_kv', request: { operation: 'set', key: 'candidate-key', value: { marker: 'ui-kv-ok' } } },
    'miniapp.ui.bridge.kv_set',
  );
  const kv = await surfaceBridge(
    context,
    descriptor,
    'candidate-ui-kv-get',
    { target: 'host_kv', request: { operation: 'get', key: 'candidate-key' } },
    'miniapp.ui.bridge.kv_get',
  );
  if (!JSON.stringify(kv).includes('ui-kv-ok')) {
    failure('miniapp_surface_kv_failed', 'UI Surface Host KV did not round-trip the exact value');
  }
  await closeSurface(context, descriptor, 'miniapp.ui.surface.close');
  await surfaceAsset(context, descriptor, 404);

  current = await workshop(context, current.miniapp.miniapp_id, 'miniapp.ui.before_second_build');
  current = await editUiSource(context, current);
  current = await buildMiniApp(context, current);
  const second = structuredClone(current.ready.release);
  if (second.release_id === first.release_id) {
    failure('miniapp_second_release_identity_reused', 'Second Build reused the first Release identity');
  }
  current = await publishReady(context, current);
  if (current.miniapp.releases.previous?.release_id !== first.release_id) {
    failure('miniapp_previous_release_missing', 'Second Publish did not retain the first Release as Previous');
  }

  const transferRoot = join(context.dataRoot, 'candidate-miniapp-transfer');
  mkdirSync(transferRoot, { recursive: true });
  const shareRoot = join(transferRoot, 'ui-share');
  const shareOperation = await productApi(
    context,
    `/api/miniapps/${encodeURIComponent(current.miniapp.miniapp_id)}/share`,
    {
      method: 'POST',
      phase: 'miniapp.ui.share_export',
      body: {
        miniapp_id: current.miniapp.miniapp_id,
        expected_product_revision: current.miniapp.product_revision,
        expected_pointer_revision: current.miniapp.releases.pointer_revision,
        content: 'active_release',
        release_id: current.miniapp.releases.active.release_id,
        expected_release_digest: current.miniapp.releases.active.release_digest,
        destination_path: shareRoot,
        include_source: true,
      },
    },
  );
  if (shareOperation.state !== 'succeeded') failure('miniapp_share_export_failed', 'Share Export did not succeed');
  const bundle = readJson(join(shareRoot, 'bundle.json'));
  const beforeShareImport = await library(context, 'miniapp.ui.library_before_share_import');
  const importedShare = await productApi(context, '/api/miniapps/import/share', {
    method: 'POST',
    phase: 'miniapp.ui.share_import',
    body: {
      expected_library_revision: beforeShareImport.library_revision,
      source_path: shareRoot,
      expected_bundle_digest: bundle.bundle_digest,
      expected_release_digest: bundle.release.artifact_digest,
      display_name: 'Imported UI MiniApp Share',
    },
  });
  if (
    importedShare.miniapp.miniapp_id === current.miniapp.miniapp_id ||
    importedShare.miniapp.lifecycle !== 'disabled'
  ) {
    failure('miniapp_share_import_identity_invalid', 'Share Import did not create a new disabled identity');
  }

  current = await productApi(
    context,
    `/api/miniapps/${encodeURIComponent(current.miniapp.miniapp_id)}/rollback`,
    {
      method: 'POST',
      timeoutMs: 180_000,
      phase: 'miniapp.ui.rollback',
      body: {
        miniapp_id: current.miniapp.miniapp_id,
        expected_product_revision: current.miniapp.product_revision,
        expected_pointer_revision: current.miniapp.releases.pointer_revision,
        expected_active_release_epoch: current.miniapp.releases.active_release_epoch,
        expected_current_release_digest: current.miniapp.releases.active.release_digest,
        previous_release_id: current.miniapp.releases.previous.release_id,
        expected_previous_release_digest: current.miniapp.releases.previous.release_digest,
      },
    },
  );
  if (current.miniapp.releases.active?.release_id !== first.release_id) {
    failure('miniapp_rollback_failed', 'Rollback did not restore the first Release identity');
  }

  current = await setEnabled(context, current, false);
  const backupRoot = join(transferRoot, 'ui-backup');
  const backupOperation = await productApi(
    context,
    `/api/miniapps/${encodeURIComponent(current.miniapp.miniapp_id)}/backup`,
    {
      method: 'POST',
      timeoutMs: 180_000,
      phase: 'miniapp.ui.backup_export',
      body: {
        miniapp_id: current.miniapp.miniapp_id,
        expected_product_revision: current.miniapp.product_revision,
        expected_lifecycle: 'disabled',
        expected_pointer_revision: current.miniapp.releases.pointer_revision,
        expected_config_revision: current.config.config_revision,
        expected_credential_bindings_revision: current.credential_bindings_revision,
        destination_path: backupRoot,
      },
    },
  );
  if (backupOperation.state !== 'succeeded') failure('miniapp_backup_export_failed', 'Backup Export did not succeed');
  const metadata = readJson(join(backupRoot, 'metadata.json'));
  const metadataDigest = sha256(canonicalJson(metadata));
  const beforeBackupImport = await library(context, 'miniapp.ui.library_before_backup_import');
  const importedBackup = await productApi(context, '/api/miniapps/import/backup', {
    method: 'POST',
    timeoutMs: 180_000,
    phase: 'miniapp.ui.backup_import',
    body: {
      expected_library_revision: beforeBackupImport.library_revision,
      source_path: backupRoot,
      expected_backup_metadata_digest: metadataDigest,
      display_name: 'Imported UI MiniApp Backup',
    },
  });
  if (
    importedBackup.miniapp.miniapp_id === current.miniapp.miniapp_id ||
    importedBackup.miniapp.lifecycle !== 'disabled'
  ) {
    failure('miniapp_backup_import_identity_invalid', 'Backup Import did not create a new disabled identity');
  }

  current = await trash(context, current);
  if (current.miniapp.lifecycle !== 'trashed') failure('miniapp_trash_failed', 'MiniApp did not enter Trash');
  current = await restore(context, current);
  if (current.miniapp.lifecycle !== 'disabled') failure('miniapp_restore_failed', 'Restored MiniApp did not return disabled');
  current = await setEnabled(context, current, true);

  let disposable = await trash(context, importedShare);
  const afterDelete = await productApi(
    context,
    `/api/miniapps/${encodeURIComponent(disposable.miniapp.miniapp_id)}/delete`,
    {
      method: 'POST',
      timeoutMs: 180_000,
      phase: 'miniapp.ui.permanent_delete',
      body: {
        miniapp_id: disposable.miniapp.miniapp_id,
        expected_product_revision: disposable.miniapp.product_revision,
        expected_lifecycle: 'trashed',
        expected_pointer_revision: disposable.miniapp.releases.pointer_revision,
        ...(disposable.miniapp.releases.active
          ? { expected_active_release_digest: disposable.miniapp.releases.active.release_digest }
          : {}),
      },
    },
  );
  if (afterDelete.miniapps.some((item) => item.miniapp_id === disposable.miniapp.miniapp_id)) {
    failure('miniapp_permanent_delete_failed', 'Permanently deleted MiniApp remains in the Library');
  }

  state.uiMiniAppId = current.miniapp.miniapp_id;
  state.uiName = UI_NAME;
  state.backupMiniAppId = importedBackup.miniapp.miniapp_id;
  return {
    miniapp_id_sha256: sha256(current.miniapp.miniapp_id),
    first_release_id_sha256: sha256(first.release_id),
    second_release_id_sha256: sha256(second.release_id),
    active_release_digest: current.miniapp.releases.active.release_digest,
    surface_asset_sha256: asset.body_sha256,
    host_kv_round_trip: true,
    share_import_as_new: true,
    backup_import_as_new: true,
    trash_restore: true,
    permanent_delete: true,
  };
}

async function descendantProcesses(rootPid) {
  const script = [
    `$rootPid = ${rootPid}`,
    '$rootProcess = Get-CimInstance Win32_Process -Filter "ProcessId = $rootPid"',
    'if ($null -eq $rootProcess) { ConvertTo-Json -Compress -InputObject @(); exit 0 }',
    '$rootCreated = $rootProcess.CreationDate',
    '$all = @(Get-CimInstance Win32_Process | Where-Object { $_.CreationDate -ge $rootCreated })',
    '$frontier = @($rootPid)',
    '$result = @()',
    'while ($frontier.Count -gt 0) {',
    '  $next = @()',
    '  foreach ($parentPid in $frontier) {',
    '    $children = @($all | Where-Object { $_.ParentProcessId -eq $parentPid })',
    '    foreach ($childProcess in $children) {',
    '      $result += [PSCustomObject]@{ process_id = [int]$childProcess.ProcessId; name = [string]$childProcess.Name; executable_path = [string]$childProcess.ExecutablePath; command_line = [string]$childProcess.CommandLine }',
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
      { encoding: 'utf8', shell: false, windowsHide: true, stdio: 'pipe', timeout: 15_000 },
    );
    if (result.status === 0 && !result.error) {
      try {
        const parsed = JSON.parse(String(result.stdout || '[]'));
        return Array.isArray(parsed) ? parsed : [parsed];
      } catch (error) {
        attempts.push({
          attempt,
          status: result.status,
          parse_error: error instanceof Error ? error.name : 'parse_error',
          stdout_sha256: sha256(String(result.stdout || '')),
        });
      }
    } else {
      attempts.push({
        attempt,
        status: result.status,
        error_code: result.error?.code ?? null,
        stderr_sha256: sha256(String(result.stderr || '')),
      });
    }
    await sleep(200 * attempt);
  }
  failure(
    'miniapp_service_process_snapshot_failed',
    'Could not snapshot Service process descendants after bounded retries',
    { attempts },
  );
}

async function serviceNodeProcess(context) {
  const candidates = (await descendantProcesses(context.getApplicationPid())).filter((process) =>
    /node\.exe$/i.test(process.name || '') &&
    String(process.command_line || '').includes('--input-type=module'),
  );
  if (candidates.length !== 1) {
    failure('miniapp_service_process_identity_ambiguous', 'Expected exactly one resident MiniApp Service Node process', {
      observed_count: candidates.length,
    });
  }
  return candidates[0];
}

function terminateProcess(pid) {
  const result = spawnSync('taskkill.exe', ['/PID', String(pid), '/F'], {
    shell: false,
    windowsHide: true,
    stdio: 'ignore',
    timeout: 10_000,
  });
  if (result.status !== 0 || result.error) {
    failure('miniapp_service_fault_injection_failed', 'Could not terminate the exact Service Node process');
  }
}

async function waitForServiceHealth(context, miniappId, state, timeoutMs) {
  const deadline = Date.now() + timeoutMs;
  let observed = null;
  while (Date.now() < deadline) {
    observed = await workshop(context, miniappId, `miniapp.service.health.${state}`);
    if (observed.miniapp.service_health?.state === state) return observed;
    await sleep(POLL_INTERVAL_MS);
  }
  failure('miniapp_service_health_timeout', `Service did not reach ${state}`, {
    observed_state: observed?.miniapp?.service_health?.state ?? null,
  });
}

async function setServiceRunning(context, current, running) {
  const active = current.miniapp.releases.active;
  return productApi(
    context,
    `/api/miniapps/${encodeURIComponent(current.miniapp.miniapp_id)}/service/running`,
    {
      method: 'POST',
      timeoutMs: 180_000,
      phase: `miniapp.service.${running ? 'start' : 'stop'}`,
      body: {
        miniapp_id: current.miniapp.miniapp_id,
        expected_product_revision: current.miniapp.product_revision,
        expected_pointer_revision: current.miniapp.releases.pointer_revision,
        expected_active_release_epoch: current.miniapp.releases.active_release_epoch,
        expected_active_release_digest: active.release_digest,
        running,
      },
    },
  );
}

async function checkServiceLifecycleFault(context, state) {
  let current = await createMiniApp(context, 'service', SERVICE_NAME);
  current = await buildMiniApp(context, current, 'on_demand');
  current = await testServiceReady(context, current);
  current = await publishReady(context, current);
  if (current.miniapp.lifecycle === 'disabled') {
    current = await setEnabled(context, current, true);
  }
  current = await setServiceRunning(context, current, true);
  if (current.miniapp.service_health?.state !== 'ready') {
    failure('miniapp_service_start_failed', 'Published Service did not become ready');
  }

  const descriptor = await openSurface(context, current, 'miniapp.service.surface.open');
  const echo = await surfaceBridge(
    context,
    descriptor,
    'candidate-service-echo',
    { target: 'service', method: 'echo', payload: { marker: 'service-bridge-ok' } },
    'miniapp.service.bridge.echo',
  );
  if (echo?.marker !== 'service-bridge-ok') {
    failure('miniapp_service_bridge_failed', 'Service Bridge did not return the exact payload');
  }

  const crashed = await serviceNodeProcess(context);
  terminateProcess(crashed.process_id);
  current = await waitForServiceHealth(
    context,
    current.miniapp.miniapp_id,
    'failed',
    30_000,
  );
  current = await productApi(
    context,
    `/api/miniapps/${encodeURIComponent(current.miniapp.miniapp_id)}/service/retry`,
    {
      method: 'POST',
      timeoutMs: 180_000,
      phase: 'miniapp.service.retry',
      body: {
        miniapp_id: current.miniapp.miniapp_id,
        expected_product_revision: current.miniapp.product_revision,
        expected_pointer_revision: current.miniapp.releases.pointer_revision,
        expected_active_release_epoch: current.miniapp.releases.active_release_epoch,
        expected_active_release_digest: current.miniapp.releases.active.release_digest,
      },
    },
  );
  if (current.miniapp.service_health?.state !== 'ready') {
    failure('miniapp_service_retry_failed', 'Service Retry did not start a fresh Host');
  }
  const restarted = await serviceNodeProcess(context);
  if (restarted.process_id === crashed.process_id) {
    failure('miniapp_service_generation_reused', 'Service Retry reused the crashed process identity');
  }

  current = await setServiceRunning(context, current, false);
  if (current.miniapp.service_health?.state !== 'stopped') {
    failure('miniapp_service_stop_failed', 'Service Stop did not reach stopped');
  }
  await closeSurface(context, descriptor, 'miniapp.service.surface.close');
  const staleBridge = await surfaceBridge(
    context,
    descriptor,
    'candidate-service-stale',
    { target: 'service', method: 'echo', payload: { marker: 'must-not-run' } },
    'miniapp.service.bridge.stale',
    [404],
  );
  if (staleBridge.status !== 404) failure('miniapp_stale_bridge_not_rejected', 'Closed Surface Bridge was not rejected');

  const beforeRestartRelease = current.miniapp.releases.active;
  const restart = await context.restart();
  current = await workshop(context, current.miniapp.miniapp_id, 'miniapp.service.after_restart');
  if (
    current.miniapp.releases.active?.release_id !== beforeRestartRelease.release_id ||
    current.miniapp.service_health?.state !== 'stopped'
  ) {
    failure('miniapp_service_restart_state_invalid', 'Service Active Release or stopped state did not survive restart');
  }
  state.serviceMiniAppId = current.miniapp.miniapp_id;
  state.serviceName = SERVICE_NAME;
  return {
    miniapp_id_sha256: sha256(current.miniapp.miniapp_id),
    release_digest: current.miniapp.releases.active.release_digest,
    test_status: 'passed',
    service_bridge_round_trip: true,
    crashed_process_id_sha256: sha256(String(crashed.process_id)),
    retry_process_changed: true,
    stale_surface_rejected: true,
    desktop_restart_pid_changed: restart.cleanup.root_pid !== restart.pid,
    restart_preserved_stopped_active_release: true,
  };
}

async function clickSurfaceButton(client, displayName, action) {
  const clicked = await client.evaluate(`(() => {
    const displayName = ${JSON.stringify(displayName)};
    const action = ${JSON.stringify(action)};
    const button = [...document.querySelectorAll('button')].find((candidate) => {
      const label = candidate.getAttribute('aria-label') ?? '';
      if (!label.includes(displayName) || !label.includes('Surface')) return false;
      return action === 'open'
        ? !label.includes('Close') && !label.includes('关闭') && !label.includes('Reload') && !label.includes('重新加载')
        : label.includes('Close') || label.includes('关闭');
    });
    if (!button) return false;
    button.click();
    return true;
  })()`);
  if (!clicked) failure('miniapp_surface_ui_action_missing', `Could not ${action} Surface through the Desktop UI`);
}

async function auditPage(client, rootSelector, phase) {
  const audit = await auditInteractiveNames(client, rootSelector);
  if (audit.interactive_count === 0 || audit.missing.length > 0) {
    failure('miniapp_desktop_a11y_failed', `${phase} has unnamed interactive controls`, {
      phase,
      interactive_count: audit.interactive_count,
      missing: audit.missing,
    });
  }
  const layout = await client.evaluate(`(() => {
    const scrolling = document.scrollingElement ?? document.documentElement;
    const content = document.querySelector('.layout-content');
    const page = content?.firstElementChild;
    return {
      viewport_width: document.documentElement.clientWidth,
      document_scroll_width: scrolling.scrollWidth,
      document_scroll_left: scrolling.scrollLeft,
      content_client_width: content?.clientWidth ?? 0,
      content_scroll_width: content?.scrollWidth ?? 0,
      content_scroll_left: content?.scrollLeft ?? 0,
      page_client_width: page?.clientWidth ?? 0,
      page_scroll_width: page?.scrollWidth ?? 0,
      page_scroll_left: page?.scrollLeft ?? 0,
    };
  })()`);
  if (
    layout.document_scroll_width > layout.viewport_width + 1 ||
    layout.document_scroll_left !== 0 ||
    layout.content_scroll_width > layout.content_client_width + 1 ||
    layout.content_scroll_left !== 0 ||
    layout.page_scroll_width > layout.page_client_width + 1 ||
    layout.page_scroll_left !== 0
  ) {
    failure('miniapp_desktop_horizontal_overflow', `${phase} drifted outside the Desktop content viewport`, {
      phase,
      layout,
    });
  }
  return { ...audit, layout };
}

async function settleDesktopPaint(client) {
  await client.evaluate(`new Promise((resolve) => {
    requestAnimationFrame(() => requestAnimationFrame(() => resolve(true)));
  })`);
  await sleep(300);
}

async function captureSettledPage(client, outputPath) {
  await settleDesktopPaint(client);
  await client.command('Page.captureScreenshot', {
    format: 'png',
    captureBeyondViewport: false,
  });
  await sleep(200);
  return capturePage(client, outputPath);
}

async function checkDesktopA11y(context, state) {
  if (!state.uiMiniAppId || !state.serviceMiniAppId) {
    failure('miniapp_a11y_fixture_missing', 'MiniApp product fixtures are missing before Desktop audit');
  }
  const client = await connectToProductPage(context);
  try {
    const evidenceRoot = join(context.runRoot, 'evidence');
    mkdirSync(evidenceRoot, { recursive: true });

    await waitForPageSelector(
      client,
      '/mini-apps',
      'section[aria-labelledby="miniapp-library-title"]',
      30_000,
      state.uiName,
    );
    await settleDesktopPaint(client);
    const libraryAudit = await auditPage(client, 'body', 'MiniApp Library');
    const libraryScreenshot = await captureSettledPage(
      client,
      join(evidenceRoot, 'miniapp-library.png'),
    );

    await waitForPageSelector(
      client,
      `/mini-apps/${state.uiMiniAppId}`,
      'ol[aria-label]',
      30_000,
      state.uiName,
    );
    await clickSurfaceButton(client, state.uiName, 'open');
    await waitForPageSelector(
      client,
      `/mini-apps/${state.uiMiniAppId}`,
      'section[aria-labelledby="miniapp-surface-title"]:not([aria-busy]) iframe[title*="Surface"]',
      30_000,
      state.uiName,
    );
    await settleDesktopPaint(client);
    const surfaceAudit = await auditPage(client, 'body', 'MiniApp Workshop/Surface');
    const surfaceScreenshot = await captureSettledPage(
      client,
      join(evidenceRoot, 'miniapp-workshop-surface.png'),
    );
    await clickSurfaceButton(client, state.uiName, 'close');

    await waitForPageSelector(
      client,
      `/mini-apps/${state.serviceMiniAppId}`,
      'ol[aria-label]',
      30_000,
      state.serviceName,
    );
    await settleDesktopPaint(client);
    const serviceAudit = await auditPage(client, 'body', 'MiniApp Service Workshop');
    const serviceScreenshot = await captureSettledPage(
      client,
      join(evidenceRoot, 'miniapp-service-workshop.png'),
    );
    return {
      library: { ...libraryAudit, screenshot: libraryScreenshot },
      workshop_surface: { ...surfaceAudit, screenshot: surfaceScreenshot },
      service_workshop: { ...serviceAudit, screenshot: serviceScreenshot },
    };
  } finally {
    client.close();
  }
}

export function assertSelfTest() {
  const descriptor = {
    miniapp_id: 'miniapp-id',
    surface_capability: 'surface-capability',
    active_release_epoch: 3,
    expected_release_digest: 'a'.repeat(64),
    ui_entrypoint: 'ui/index.html',
  };
  const path = surfaceAssetPath(descriptor);
  if (!path.endsWith('/ui/index.html') || !path.includes('/3/')) {
    throw new Error('Surface path self-test failed');
  }
  const metadata = { z: 1, a: { y: 2, x: 1 } };
  if (canonicalJson(metadata) !== '{"a":{"x":1,"y":2},"z":1}') {
    throw new Error('Backup metadata canonicalization self-test failed');
  }
  if (EMPTY_INPUT_DIGEST !== 'e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855') {
    throw new Error('Service Test input digest self-test failed');
  }
  return {
    schema_version: '1.0.0',
    status: 'pass',
    suite: {
      name: 'windows-miniapp-product-candidate-self-test',
      checks: ['surface-path', 'backup-canonical-json', 'empty-input-digest'],
    },
  };
}

async function runCurrentCandidate() {
  const candidate = resolveCurrentCandidate();
  const state = {};
  return runCandidateSmoke({
    installer: candidate.installer,
    sourceCommit: candidate.sourceCommit,
    workRoot: candidate.workRoot.replace('product-runs', 'miniapp-product-runs'),
    productChecks: [
      {
        id: 'miniapp-ui-release-surface-transfer',
        timeoutMs: PRODUCT_TIMEOUT_MS,
        run: (context) => checkUiLifecycleTransfer(context, state),
      },
      {
        id: 'miniapp-service-test-bridge-fault',
        timeoutMs: PRODUCT_TIMEOUT_MS,
        run: (context) => checkServiceLifecycleFault(context, state),
      },
      {
        id: 'miniapp-desktop-a11y',
        timeoutMs: 180_000,
        run: (context) => checkDesktopA11y(context, state),
      },
    ],
  });
}

const isMain = process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url);
if (isMain) {
  const args = process.argv.slice(2);
  try {
    let result;
    if (args.length === 1 && args[0] === '--self-test') result = assertSelfTest();
    else if (args.length === 1 && args[0] === '--current-candidate') result = await runCurrentCandidate();
    else throw new Error('usage: --self-test | --current-candidate');
    console.log(JSON.stringify(result, null, 2));
    process.exitCode = result.status === 'pass' ? 0 : 1;
  } catch (error) {
    console.log(JSON.stringify({
      schema_version: '1.0.0',
      status: 'fail',
      code: error instanceof SmokeFailure ? error.code : 'runner_error',
      reason: error instanceof Error ? error.message : 'runner failed',
    }, null, 2));
    process.exitCode = 2;
  }
}
