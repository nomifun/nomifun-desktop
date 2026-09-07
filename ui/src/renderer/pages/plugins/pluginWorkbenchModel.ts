/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import {
  isBackendHttpError,
  isBackendRequestError,
} from '@/common/adapter/httpBridge';
import type {
  ApplyPluginCandidateRequest,
  BuildPluginProjectRequest,
  DeletePluginDataRequest,
  DeletePluginProjectRequest,
  PluginDetail,
  PluginProjectDetail,
  PluginSummary,
  RestorePluginPreviousRequest,
  RetryPluginRequest,
  SetPluginEnabledRequest,
  TestPluginCandidateRequest,
  UninstallPluginRequest,
} from '@/common/types/pluginPlatform';

export type PluginLoadFailureKind = 'unavailable' | 'error';

export interface PluginLoadFailure {
  kind: PluginLoadFailureKind;
  message: string;
}

export interface PluginMountActions {
  canToggleEnabled: boolean;
  canRetry: boolean;
  canRestore: boolean;
  canUninstall: boolean;
  canDeleteData: boolean;
}

/** SHA-256 of an explicitly empty candidate-test input snapshot. */
export const EMPTY_TEST_INPUT_DIGEST =
  'e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855';

export type PluginApplyTargetSelection = 'initial_install' | 'existing_mount';

export function isPluginDigest(value: string): boolean {
  return /^[0-9a-f]{64}$/i.test(value.trim());
}

const currentTarget = (detail: PluginDetail) => {
  if (!detail.summary.current) {
    throw new Error('Plugin Mount has no current target');
  }
  return detail.summary.current;
};

/**
 * Build the exact Apply CAS request from the project detail currently shown in
 * the Workshop. The UI deliberately accepts only a linked Mount summary for
 * an existing-mount apply; it never guesses a revision or target digest.
 */
export function applyPluginCandidateRequest(
  detail: PluginProjectDetail,
  libraryRevision: number,
  targetSelection: PluginApplyTargetSelection,
  allowBreaking: boolean,
  acknowledgeTestWarning: boolean,
  linkedMount?: PluginSummary
): ApplyPluginCandidateRequest {
  const ready = detail.ready;
  if (!ready) {
    throw new Error('Plugin Project has no Ready Candidate');
  }

  const target =
    targetSelection === 'initial_install'
      ? {
          target: 'initial_install' as const,
          expected_library_revision: libraryRevision,
        }
      : (() => {
          if (!linkedMount?.current) {
            throw new Error('Linked Plugin Mount has no current target');
          }
          return {
            target: 'existing_mount' as const,
            mount_id: linkedMount.mount_id,
            expected_mount_revision: linkedMount.mount_revision,
            expected_current_target_digest: linkedMount.current.artifact_digest,
          };
        })();

  return {
    project_id: detail.summary.project_id,
    expected_project_revision: detail.summary.project_revision,
    expected_build_generation: detail.summary.build_generation,
    candidate_id: ready.candidate.candidate_id,
    expected_candidate_digest: ready.candidate.candidate_digest,
    target,
    allow_breaking: allowBreaking,
    acknowledge_test_warning: acknowledgeTestWarning,
  };
}

export function deletePluginProjectRequest(
  detail: PluginProjectDetail
): DeletePluginProjectRequest {
  const ready = detail.ready?.candidate ?? detail.summary.ready_candidate;
  return {
    project_id: detail.summary.project_id,
    expected_project_revision: detail.summary.project_revision,
    expected_build_generation: detail.summary.build_generation,
    ...(ready
      ? {
          expected_ready_candidate_id: ready.candidate_id,
          expected_ready_candidate_digest: ready.candidate_digest,
        }
      : {}),
  };
}

export function buildPluginProjectRequest(
  detail: PluginProjectDetail
): BuildPluginProjectRequest {
  if (
    detail.summary.source_state !== 'editable' ||
    !detail.source_snapshot_digest ||
    !detail.dependency_lock_digest
  ) {
    throw new Error('Plugin Project does not have a complete editable source snapshot');
  }
  return {
    project_id: detail.summary.project_id,
    expected_project_revision: detail.summary.project_revision,
    expected_build_generation: detail.summary.build_generation,
    expected_source_snapshot_digest: detail.source_snapshot_digest,
    expected_dependency_lock_digest: detail.dependency_lock_digest,
  };
}

export function testPluginCandidateRequest(
  detail: PluginProjectDetail,
  configRevision: number,
  credentialBindingsRevision: number,
  resolvedTestInputDigest: string
): TestPluginCandidateRequest {
  const ready = detail.ready;
  if (!ready) {
    throw new Error('Plugin Project has no Ready Candidate');
  }
  if (!isPluginDigest(resolvedTestInputDigest)) {
    throw new Error('Candidate test input digest is invalid');
  }
  return {
    project_id: detail.summary.project_id,
    expected_project_revision: detail.summary.project_revision,
    expected_build_generation: detail.summary.build_generation,
    candidate_id: ready.candidate.candidate_id,
    expected_candidate_digest: ready.candidate.candidate_digest,
    expected_config_revision: configRevision,
    expected_credential_bindings_revision: credentialBindingsRevision,
    resolved_test_input_digest: resolvedTestInputDigest.toLowerCase(),
  };
}

export function pluginLoadFailure(
  error: unknown,
  scope: 'platform' | 'resource'
): PluginLoadFailure {
  const unavailable =
    isBackendRequestError(error) ||
    (isBackendHttpError(error) &&
      (error.status === 503 ||
        error.status === 501 ||
        (scope === 'platform' && error.status === 404)));

  const message = isBackendHttpError(error)
    ? error.backendMessage || error.message
    : error instanceof Error
      ? error.message
      : String(error);

  return {
    kind: unavailable ? 'unavailable' : 'error',
    message,
  };
}

export function pluginMountActions(detail: PluginDetail): PluginMountActions {
  const { lifecycle, current, previous } = detail.summary;
  const stableCurrent = current != null && lifecycle !== 'delete_pending';
  return {
    canToggleEnabled:
      stableCurrent && (lifecycle === 'enabled' || lifecycle === 'disabled'),
    canRetry: stableCurrent && lifecycle === 'error',
    canRestore:
      stableCurrent &&
      previous != null &&
      lifecycle !== 'uninstalled_data_retained',
    canUninstall:
      stableCurrent &&
      lifecycle !== 'uninstalled_data_retained',
    canDeleteData:
      lifecycle === 'uninstalled_data_retained' &&
      detail.retained_data &&
      current == null,
  };
}

export function setPluginEnabledRequest(
  detail: PluginDetail,
  enabled: boolean
): SetPluginEnabledRequest {
  const current = currentTarget(detail);
  return {
    mount_id: detail.summary.mount_id,
    expected_mount_revision: detail.summary.mount_revision,
    expected_current_target_digest: current.artifact_digest,
    enabled,
  };
}

export function retryPluginRequest(detail: PluginDetail): RetryPluginRequest {
  const current = currentTarget(detail);
  return {
    mount_id: detail.summary.mount_id,
    expected_mount_revision: detail.summary.mount_revision,
    expected_current_target_digest: current.artifact_digest,
  };
}

export function restorePluginRequest(
  detail: PluginDetail
): RestorePluginPreviousRequest {
  const current = currentTarget(detail);
  const previous = detail.summary.previous;
  if (!previous) {
    throw new Error('Plugin Mount has no previous target');
  }
  return {
    mount_id: detail.summary.mount_id,
    expected_mount_revision: detail.summary.mount_revision,
    expected_current_target_digest: current.artifact_digest,
    expected_previous_target_digest: previous.artifact_digest,
  };
}

export function uninstallPluginRequest(
  detail: PluginDetail
): UninstallPluginRequest {
  const current = currentTarget(detail);
  return {
    mount_id: detail.summary.mount_id,
    expected_mount_revision: detail.summary.mount_revision,
    expected_current_target_digest: current.artifact_digest,
  };
}

export function deletePluginDataRequest(
  detail: PluginDetail
): DeletePluginDataRequest {
  if (!pluginMountActions(detail).canDeleteData) {
    throw new Error('Plugin Mount is not retained and uninstalled');
  }
  return {
    mount_id: detail.summary.mount_id,
    expected_mount_revision: detail.summary.mount_revision,
    expected_lifecycle: 'uninstalled_data_retained',
    expected_data_revision: detail.summary.mount_revision,
  };
}

export function shortPluginIdentity(value: string | undefined): string {
  if (!value) return '-';
  if (value.length <= 18) return value;
  return `${value.slice(0, 8)}...${value.slice(-6)}`;
}

export function formatPluginTimestamp(value: number, locale: string): string {
  const date = new Date(value);
  if (Number.isNaN(date.getTime())) return '-';
  return new Intl.DateTimeFormat(locale, {
    dateStyle: 'medium',
    timeStyle: 'short',
  }).format(date);
}

export function projectDeleteAvailable(detail: PluginProjectDetail): boolean {
  return detail.active_operation?.state !== 'running';
}
