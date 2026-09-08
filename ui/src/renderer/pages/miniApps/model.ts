/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import type {
  BuildMiniAppRequest,
  MiniAppPublishMode,
  MiniAppServiceLifecycle,
  MiniAppSurfaceLaunchDescriptor,
  MiniAppSummary,
  MiniAppWorkshop,
  PublishMiniAppRequest,
  RollbackMiniAppRequest,
  SetMiniAppEnabledRequest,
  SetMiniAppPublishModeRequest,
  SetMiniAppServiceRunningRequest,
  RetryMiniAppServiceRequest,
} from '@/common/types/miniAppPlatform';

export type MiniAppReleaseStage = 'draft' | 'ready' | 'active';
export type MiniAppWorkflowStepState =
  | 'done'
  | 'active'
  | 'blocked'
  | 'pending';

export interface MiniAppWorkflowState {
  source: MiniAppWorkflowStepState;
  build: MiniAppWorkflowStepState;
  ready: MiniAppWorkflowStepState;
  publish: MiniAppWorkflowStepState;
  surface: MiniAppWorkflowStepState;
}

export function miniAppReleaseStage(
  miniapp: MiniAppSummary
): MiniAppReleaseStage {
  if (miniapp.releases.active) return 'active';
  if (miniapp.releases.ready) return 'ready';
  return 'draft';
}

export function miniAppWorkflowState(
  workshop: MiniAppWorkshop
): MiniAppWorkflowState {
  const buildRunning =
    workshop.active_operation?.kind === 'build' &&
    workshop.active_operation.state === 'running';
  const sourceReady = workshop.source_state !== 'empty';
  const hasReady = Boolean(workshop.ready);
  const hasActive = Boolean(workshop.miniapp.releases.active);
  const hasBuiltRelease = hasReady || hasActive;

  return {
    source: sourceReady ? 'done' : 'pending',
    build: buildRunning
      ? 'active'
      : hasBuiltRelease
        ? 'done'
        : sourceReady
          ? 'active'
          : 'blocked',
    ready: hasBuiltRelease ? 'done' : 'pending',
    publish: hasReady
      ? workshop.ready?.can_publish
        ? 'active'
        : 'blocked'
      : hasActive
        ? 'done'
        : 'pending',
    surface:
      hasActive &&
      workshop.miniapp.lifecycle === 'enabled' &&
      workshop.miniapp.surface_available
        ? 'done'
        : hasActive
          ? 'blocked'
          : 'pending',
  };
}

export function miniAppBuildRequest(
  workshop: MiniAppWorkshop,
  serviceLifecycle: MiniAppServiceLifecycle = 'on_demand'
): BuildMiniAppRequest | null {
  if (
    workshop.miniapp.kind !== 'ui_only' &&
    workshop.miniapp.kind !== 'service' ||
    workshop.source_state !== 'editable' ||
    workshop.active_operation?.state === 'running' ||
    !workshop.source_snapshot_digest ||
    !workshop.dependency_lock_digest ||
    workshop.build_generation < 1
  ) {
    return null;
  }
  return {
    miniapp_id: workshop.miniapp.miniapp_id,
    expected_product_revision: workshop.miniapp.product_revision,
    project_id: workshop.project_id,
    expected_project_revision: workshop.project_revision,
    expected_build_generation: workshop.build_generation,
    expected_source_snapshot_digest: workshop.source_snapshot_digest,
    expected_dependency_lock_digest: workshop.dependency_lock_digest,
    ...(workshop.miniapp.kind === 'service'
      ? { service_lifecycle: serviceLifecycle }
      : {}),
  };
}

function releaseRefsMatch(
  left: MiniAppSummary['releases']['ready'],
  right: MiniAppSummary['releases']['ready']
): boolean {
  return Boolean(
    left &&
      right &&
      left.release_id === right.release_id &&
      left.artifact_id === right.artifact_id &&
      left.release_digest === right.release_digest &&
      left.manifest_digest === right.manifest_digest
  );
}

function miniAppAllowsReleaseMutation(workshop: MiniAppWorkshop): boolean {
  return (
    (workshop.miniapp.kind === 'ui_only' ||
      workshop.miniapp.kind === 'service') &&
    (workshop.miniapp.lifecycle === 'enabled' ||
      workshop.miniapp.lifecycle === 'disabled') &&
    workshop.active_operation?.state !== 'running'
  );
}

export function miniAppPublishRequest(
  workshop: MiniAppWorkshop
): PublishMiniAppRequest | null {
  const { miniapp, ready } = workshop;
  const readyPointer = miniapp.releases.ready;
  if (
    !miniAppAllowsReleaseMutation(workshop) ||
    !ready ||
    !readyPointer ||
    !ready.can_publish ||
    (ready.kind === 'ui_only' && ready.test.status !== 'not_required') ||
    (ready.kind === 'service' && ready.test.status !== 'needs_test_input') ||
    ready.test.release_id !== ready.release.release_id ||
    ready.test.expected_release_digest !== ready.release.release_digest ||
    !releaseRefsMatch(readyPointer, ready.release)
  ) {
    return null;
  }

  return {
    miniapp_id: miniapp.miniapp_id,
    expected_product_revision: miniapp.product_revision,
    expected_pointer_revision: miniapp.releases.pointer_revision,
    expected_active_release_epoch: miniapp.releases.active_release_epoch,
    ready_release_id: ready.release.release_id,
    expected_ready_release_digest: ready.release.release_digest,
    ...(miniapp.releases.active
      ? {
          expected_active_release_digest:
            miniapp.releases.active.release_digest,
        }
      : {}),
    ...(ready.test.receipt_id
      ? { expected_service_test_receipt_id: ready.test.receipt_id }
      : {}),
    acknowledge_test_warning: ready.kind === 'service',
  };
}

export function miniAppRollbackRequest(
  workshop: MiniAppWorkshop
): RollbackMiniAppRequest | null {
  const { miniapp } = workshop;
  const active = miniapp.releases.active;
  const previous = miniapp.releases.previous;
  if (
    !miniAppAllowsReleaseMutation(workshop) ||
    !active ||
    !previous ||
    miniapp.releases.active_release_epoch < 1
  ) {
    return null;
  }

  return {
    miniapp_id: miniapp.miniapp_id,
    expected_product_revision: miniapp.product_revision,
    expected_pointer_revision: miniapp.releases.pointer_revision,
    expected_active_release_epoch: miniapp.releases.active_release_epoch,
    expected_current_release_digest: active.release_digest,
    previous_release_id: previous.release_id,
    expected_previous_release_digest: previous.release_digest,
  };
}

export function miniAppSetEnabledRequest(
  workshop: MiniAppWorkshop,
  enabled: boolean
): SetMiniAppEnabledRequest | null {
  const { miniapp } = workshop;
  if (
    miniapp.kind !== 'ui_only' && miniapp.kind !== 'service' ||
    workshop.active_operation?.state === 'running' ||
    (miniapp.lifecycle !== 'enabled' && miniapp.lifecycle !== 'disabled') ||
    (enabled && !miniapp.releases.active) ||
    (enabled && miniapp.lifecycle === 'enabled') ||
    (!enabled && miniapp.lifecycle === 'disabled')
  ) {
    return null;
  }

  return {
    miniapp_id: miniapp.miniapp_id,
    expected_product_revision: miniapp.product_revision,
    expected_pointer_revision: miniapp.releases.pointer_revision,
    ...(miniapp.releases.active
      ? {
          expected_active_release_digest:
            miniapp.releases.active.release_digest,
        }
      : {}),
    enabled,
  };
}

export function miniAppSetPublishModeRequest(
  workshop: MiniAppWorkshop,
  mode: MiniAppPublishMode
): SetMiniAppPublishModeRequest | null {
  const { miniapp } = workshop;
  const buildRunning =
    workshop.active_operation?.kind === 'build' &&
    workshop.active_operation.state === 'running';
  const revokingDuringBuild =
    buildRunning &&
    workshop.publish_mode === 'auto_ui_only' &&
    mode === 'manual';
  if (
    miniapp.kind !== 'ui_only' ||
    (buildRunning && !revokingDuringBuild) ||
    (miniapp.lifecycle !== 'enabled' && miniapp.lifecycle !== 'disabled') ||
    workshop.publish_mode === mode ||
    (mode === 'auto_ui_only' && !miniapp.releases.active)
  ) {
    return null;
  }

  return {
    miniapp_id: miniapp.miniapp_id,
    expected_product_revision: miniapp.product_revision,
    expected_pointer_revision: miniapp.releases.pointer_revision,
    mode,
  };
}

export function miniAppSetServiceRunningRequest(
  workshop: MiniAppWorkshop,
  running: boolean
): SetMiniAppServiceRunningRequest | null {
  const { miniapp } = workshop;
  const active = miniapp.releases.active;
  if (
    miniapp.kind !== 'service' ||
    miniapp.lifecycle !== 'enabled' ||
    !active ||
    miniapp.releases.active_release_epoch < 1
  ) {
    return null;
  }
  return {
    miniapp_id: miniapp.miniapp_id,
    expected_product_revision: miniapp.product_revision,
    expected_pointer_revision: miniapp.releases.pointer_revision,
    expected_active_release_epoch: miniapp.releases.active_release_epoch,
    expected_active_release_digest: active.release_digest,
    running,
  };
}

export function miniAppRetryServiceRequest(
  workshop: MiniAppWorkshop
): RetryMiniAppServiceRequest | null {
  const request = miniAppSetServiceRunningRequest(workshop, true);
  if (!request) return null;
  const { running: _running, ...retry } = request;
  return retry;
}

export function miniAppCanOpenSurface(workshop: MiniAppWorkshop): boolean {
  return Boolean(
    (workshop.miniapp.kind === 'ui_only' ||
      workshop.miniapp.kind === 'service') &&
      workshop.miniapp.lifecycle === 'enabled' &&
      workshop.miniapp.surface_available &&
      workshop.miniapp.releases.active &&
      workshop.miniapp.releases.active_release_epoch > 0
  );
}

export function miniAppSurfaceMatchesWorkshop(
  descriptor: MiniAppSurfaceLaunchDescriptor,
  workshop: MiniAppWorkshop
): boolean {
  const active = workshop.miniapp.releases.active;
  return Boolean(
    miniAppCanOpenSurface(workshop) &&
      active &&
      descriptor.miniapp_id === workshop.miniapp.miniapp_id &&
      descriptor.release_id === active.release_id &&
      descriptor.expected_release_digest === active.release_digest &&
      descriptor.active_release_epoch ===
        workshop.miniapp.releases.active_release_epoch &&
      descriptor.surface_session_id.trim().length > 0 &&
      Number.isSafeInteger(descriptor.surface_generation) &&
      descriptor.surface_generation > 0 &&
      descriptor.surface_capability.trim().length > 0 &&
      descriptor.kind === workshop.miniapp.kind
  );
}

export function miniAppSurfaceAssetPath(
  descriptor: MiniAppSurfaceLaunchDescriptor
): string | null {
  const entrypoint = descriptor.ui_entrypoint;
  const segments = entrypoint.split('/');
  if (
    !Number.isSafeInteger(descriptor.active_release_epoch) ||
    descriptor.active_release_epoch < 1 ||
    !/^[a-f0-9]{64}$/.test(descriptor.expected_release_digest) ||
    !descriptor.surface_capability ||
    entrypoint.startsWith('/') ||
    entrypoint.endsWith('/') ||
    entrypoint.includes('\\') ||
    segments.some((segment) => !segment || segment === '.' || segment === '..')
  ) {
    return null;
  }

  const encodedEntrypoint = segments.map(encodeURIComponent).join('/');
  return `/api/miniapps/${encodeURIComponent(
    descriptor.miniapp_id
  )}/surface/assets/${encodeURIComponent(
    descriptor.surface_capability
  )}/${descriptor.active_release_epoch}/${encodeURIComponent(
    descriptor.expected_release_digest
  )}/${encodedEntrypoint}`;
}

export function shortMiniAppIdentity(
  value: string | undefined,
  edgeLength = 8
): string {
  if (!value) return '—';
  if (value.length <= edgeLength * 2 + 1) return value;
  return `${value.slice(0, edgeLength)}…${value.slice(-edgeLength)}`;
}

export function formatMiniAppTimestamp(
  value: number,
  locale: string
): string {
  if (!Number.isFinite(value)) return '—';
  return new Intl.DateTimeFormat(locale, {
    year: 'numeric',
    month: 'short',
    day: '2-digit',
    hour: '2-digit',
    minute: '2-digit',
  }).format(new Date(value));
}
