/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import type {
  BuildPluginRuntimeRequest,
  DeletePluginRuntimeRequest,
  PluginRuntimePublishMode,
  PluginRuntimeServiceLifecycle,
  PluginRuntimeSurfaceLaunchDescriptor,
  PluginRuntimeSummary,
  PluginRuntimeWorkshop,
  PublishPluginRuntimeRequest,
  RestorePluginRuntimeRequest,
  RetryPluginRuntimeDeleteRequest,
  RollbackPluginRuntimeRequest,
  SetPluginRuntimeEnabledRequest,
  SetPluginRuntimePublishModeRequest,
  SetPluginRuntimeServiceRunningRequest,
  TestPluginRuntimeReleaseRequest,
  TrashPluginRuntimeRequest,
  RetryPluginRuntimeServiceRequest,
  SharePluginRuntimeRequest,
} from '@/common/types/pluginRuntimePlatform';

export type PluginRuntimeReleaseStage = 'draft' | 'ready' | 'active';
export type PluginRuntimeWorkflowStepState =
  | 'done'
  | 'active'
  | 'blocked'
  | 'pending';

export interface PluginRuntimeWorkflowState {
  source: PluginRuntimeWorkflowStepState;
  build: PluginRuntimeWorkflowStepState;
  ready: PluginRuntimeWorkflowStepState;
  publish: PluginRuntimeWorkflowStepState;
  surface: PluginRuntimeWorkflowStepState;
}

export const EMPTY_MINIAPP_TEST_INPUT_DIGEST =
  'e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855';

export function pluginRuntimeReleaseStage(
  plugin: PluginRuntimeSummary
): PluginRuntimeReleaseStage {
  if (plugin.releases.active) return 'active';
  if (plugin.releases.ready) return 'ready';
  return 'draft';
}

export function pluginRuntimeWorkflowState(
  workshop: PluginRuntimeWorkshop
): PluginRuntimeWorkflowState {
  const buildRunning =
    workshop.active_operation?.kind === 'build' &&
    workshop.active_operation.state === 'running';
  const sourceReady = workshop.source_state !== 'empty';
  const hasReady = Boolean(workshop.ready);
  const hasActive = Boolean(workshop.plugin.releases.active);
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
      workshop.plugin.lifecycle === 'enabled' &&
      workshop.plugin.surface_available
        ? 'done'
        : hasActive
          ? 'blocked'
          : 'pending',
  };
}

export function pluginRuntimeBuildRequest(
  workshop: PluginRuntimeWorkshop,
  serviceLifecycle: PluginRuntimeServiceLifecycle = 'on_demand'
): BuildPluginRuntimeRequest | null {
  if (
    workshop.plugin.kind !== 'ui_only' &&
    workshop.plugin.kind !== 'service' ||
    workshop.source_state !== 'editable' ||
    workshop.active_operation?.state === 'running' ||
    !workshop.source_snapshot_digest ||
    !workshop.dependency_lock_digest ||
    workshop.build_generation < 1
  ) {
    return null;
  }
  return {
    plugin_id: workshop.plugin.plugin_id,
    expected_product_revision: workshop.plugin.product_revision,
    project_id: workshop.project_id,
    expected_project_revision: workshop.project_revision,
    expected_build_generation: workshop.build_generation,
    expected_source_snapshot_digest: workshop.source_snapshot_digest,
    expected_dependency_lock_digest: workshop.dependency_lock_digest,
    ...(workshop.plugin.kind === 'service'
      ? { service_lifecycle: serviceLifecycle }
      : {}),
  };
}

export function pluginRuntimeTestRequest(
  workshop: PluginRuntimeWorkshop
): TestPluginRuntimeReleaseRequest | null {
  const { plugin, ready } = workshop;
  if (
    plugin.kind !== 'service' ||
    (plugin.lifecycle !== 'enabled' && plugin.lifecycle !== 'disabled') ||
    !ready?.service ||
    workshop.active_operation?.state === 'running'
  ) {
    return null;
  }
  return {
    plugin_id: plugin.plugin_id,
    expected_product_revision: plugin.product_revision,
    expected_pointer_revision: plugin.releases.pointer_revision,
    project_id: workshop.project_id,
    expected_project_revision: workshop.project_revision,
    expected_build_generation: workshop.build_generation,
    release_id: ready.release.release_id,
    expected_release_digest: ready.release.release_digest,
    expected_config_revision: workshop.config.config_revision,
    expected_credential_bindings_revision:
      workshop.credential_bindings_revision,
    resolved_test_input_digest: EMPTY_MINIAPP_TEST_INPUT_DIGEST,
  };
}

export function pluginRuntimeShareRequest(
  workshop: PluginRuntimeWorkshop,
  content: SharePluginRuntimeRequest['content'],
  destinationPath: string,
  includeSource: boolean
): SharePluginRuntimeRequest | null {
  const { plugin } = workshop;
  const release =
    content === 'ready_release'
      ? workshop.ready?.release
      : plugin.releases.active;
  if (
    (plugin.lifecycle !== 'enabled' && plugin.lifecycle !== 'disabled') ||
    workshop.active_operation?.state === 'running' ||
    !release ||
    !destinationPath.trim() ||
    (includeSource && workshop.source_state !== 'editable')
  ) {
    return null;
  }
  if (
    content === 'ready_release' &&
    !releaseRefsMatch(plugin.releases.ready, release)
  ) {
    return null;
  }
  return {
    plugin_id: plugin.plugin_id,
    expected_product_revision: plugin.product_revision,
    expected_pointer_revision: plugin.releases.pointer_revision,
    content,
    release_id: release.release_id,
    expected_release_digest: release.release_digest,
    destination_path: destinationPath.trim(),
    include_source: includeSource,
  };
}

function releaseRefsMatch(
  left: PluginRuntimeSummary['releases']['ready'],
  right: PluginRuntimeSummary['releases']['ready']
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

function pluginRuntimeAllowsReleaseMutation(workshop: PluginRuntimeWorkshop): boolean {
  return (
    (workshop.plugin.kind === 'ui_only' ||
      workshop.plugin.kind === 'service') &&
    (workshop.plugin.lifecycle === 'enabled' ||
      workshop.plugin.lifecycle === 'disabled') &&
    workshop.active_operation?.state !== 'running'
  );
}

export function pluginRuntimePublishRequest(
  workshop: PluginRuntimeWorkshop
): PublishPluginRuntimeRequest | null {
  const { plugin, ready } = workshop;
  const readyPointer = plugin.releases.ready;
  if (
    !pluginRuntimeAllowsReleaseMutation(workshop) ||
    !ready ||
    !readyPointer ||
    !ready.can_publish ||
    (ready.kind === 'ui_only' && ready.test.status !== 'not_required') ||
    ready.test.release_id !== ready.release.release_id ||
    ready.test.expected_release_digest !== ready.release.release_digest ||
    !releaseRefsMatch(readyPointer, ready.release)
  ) {
    return null;
  }

  return {
    plugin_id: plugin.plugin_id,
    expected_product_revision: plugin.product_revision,
    expected_pointer_revision: plugin.releases.pointer_revision,
    expected_active_release_epoch: plugin.releases.active_release_epoch,
    ready_release_id: ready.release.release_id,
    expected_ready_release_digest: ready.release.release_digest,
    ...(plugin.releases.active
      ? {
          expected_active_release_digest:
            plugin.releases.active.release_digest,
        }
      : {}),
    ...(ready.kind === 'service' &&
    ready.test.status !== 'stale' &&
    ready.test.receipt_id
      ? { expected_service_test_receipt_id: ready.test.receipt_id }
      : {}),
    acknowledge_test_warning:
      ready.kind === 'service' && ready.test.status !== 'passed',
  };
}

export function pluginRuntimeRollbackRequest(
  workshop: PluginRuntimeWorkshop
): RollbackPluginRuntimeRequest | null {
  const { plugin } = workshop;
  const active = plugin.releases.active;
  const previous = plugin.releases.previous;
  if (
    !pluginRuntimeAllowsReleaseMutation(workshop) ||
    !active ||
    !previous ||
    plugin.releases.active_release_epoch < 1
  ) {
    return null;
  }

  return {
    plugin_id: plugin.plugin_id,
    expected_product_revision: plugin.product_revision,
    expected_pointer_revision: plugin.releases.pointer_revision,
    expected_active_release_epoch: plugin.releases.active_release_epoch,
    expected_current_release_digest: active.release_digest,
    previous_release_id: previous.release_id,
    expected_previous_release_digest: previous.release_digest,
  };
}

export function pluginRuntimeSetEnabledRequest(
  workshop: PluginRuntimeWorkshop,
  enabled: boolean
): SetPluginRuntimeEnabledRequest | null {
  const { plugin } = workshop;
  if (
    plugin.kind !== 'ui_only' && plugin.kind !== 'service' ||
    workshop.active_operation?.state === 'running' ||
    (plugin.lifecycle !== 'enabled' && plugin.lifecycle !== 'disabled') ||
    (enabled && !plugin.releases.active) ||
    (enabled && plugin.lifecycle === 'enabled') ||
    (!enabled && plugin.lifecycle === 'disabled')
  ) {
    return null;
  }

  return {
    plugin_id: plugin.plugin_id,
    expected_product_revision: plugin.product_revision,
    expected_pointer_revision: plugin.releases.pointer_revision,
    ...(plugin.releases.active
      ? {
          expected_active_release_digest:
            plugin.releases.active.release_digest,
        }
      : {}),
    enabled,
  };
}

export function pluginRuntimeSetPublishModeRequest(
  workshop: PluginRuntimeWorkshop,
  mode: PluginRuntimePublishMode
): SetPluginRuntimePublishModeRequest | null {
  const { plugin } = workshop;
  const buildRunning =
    workshop.active_operation?.kind === 'build' &&
    workshop.active_operation.state === 'running';
  const revokingDuringBuild =
    buildRunning &&
    workshop.publish_mode === 'auto_ui_only' &&
    mode === 'manual';
  if (
    plugin.kind !== 'ui_only' ||
    (buildRunning && !revokingDuringBuild) ||
    (plugin.lifecycle !== 'enabled' && plugin.lifecycle !== 'disabled') ||
    workshop.publish_mode === mode ||
    (mode === 'auto_ui_only' && !plugin.releases.active)
  ) {
    return null;
  }

  return {
    plugin_id: plugin.plugin_id,
    expected_product_revision: plugin.product_revision,
    expected_pointer_revision: plugin.releases.pointer_revision,
    mode,
  };
}

export function pluginRuntimeSetServiceRunningRequest(
  workshop: PluginRuntimeWorkshop,
  running: boolean
): SetPluginRuntimeServiceRunningRequest | null {
  const { plugin } = workshop;
  const active = plugin.releases.active;
  if (
    plugin.kind !== 'service' ||
    plugin.lifecycle !== 'enabled' ||
    !active ||
    plugin.releases.active_release_epoch < 1
  ) {
    return null;
  }
  return {
    plugin_id: plugin.plugin_id,
    expected_product_revision: plugin.product_revision,
    expected_pointer_revision: plugin.releases.pointer_revision,
    expected_active_release_epoch: plugin.releases.active_release_epoch,
    expected_active_release_digest: active.release_digest,
    running,
  };
}

export function pluginRuntimeRetryServiceRequest(
  workshop: PluginRuntimeWorkshop
): RetryPluginRuntimeServiceRequest | null {
  const request = pluginRuntimeSetServiceRunningRequest(workshop, true);
  if (!request) return null;
  const { running: _running, ...retry } = request;
  return retry;
}

export function pluginRuntimeTrashRequest(
  workshop: PluginRuntimeWorkshop
): TrashPluginRuntimeRequest | null {
  const { plugin } = workshop;
  if (
    (plugin.lifecycle !== 'enabled' && plugin.lifecycle !== 'disabled') ||
    workshop.active_operation?.state === 'running'
  ) {
    return null;
  }
  return {
    plugin_id: plugin.plugin_id,
    expected_product_revision: plugin.product_revision,
    expected_pointer_revision: plugin.releases.pointer_revision,
    ...(plugin.releases.active
      ? {
          expected_active_release_digest:
            plugin.releases.active.release_digest,
        }
      : {}),
  };
}

export function pluginRuntimeRestoreRequest(
  workshop: PluginRuntimeWorkshop
): RestorePluginRuntimeRequest | null {
  const { plugin } = workshop;
  if (
    plugin.lifecycle !== 'trashed' ||
    workshop.active_operation?.state === 'running'
  ) {
    return null;
  }
  return {
    plugin_id: plugin.plugin_id,
    expected_product_revision: plugin.product_revision,
    expected_lifecycle: 'trashed',
    expected_pointer_revision: plugin.releases.pointer_revision,
  };
}

export function pluginRuntimeDeleteRequest(
  workshop: PluginRuntimeWorkshop
): DeletePluginRuntimeRequest | null {
  const restore = pluginRuntimeRestoreRequest(workshop);
  if (!restore) return null;
  return {
    ...restore,
    ...(workshop.plugin.releases.active
      ? {
          expected_active_release_digest:
            workshop.plugin.releases.active.release_digest,
        }
      : {}),
  };
}

export function pluginRuntimeRetryDeleteRequest(
  workshop: PluginRuntimeWorkshop
): RetryPluginRuntimeDeleteRequest | null {
  const operation = workshop.active_operation;
  if (
    workshop.plugin.lifecycle !== 'deleting' ||
    operation?.kind !== 'plugin_permanent_delete' ||
    operation.state !== 'failed'
  ) {
    return null;
  }
  return {
    plugin_id: workshop.plugin.plugin_id,
    failed_operation_id: operation.operation_id,
    expected_operation_revision: operation.operation_revision,
  };
}

export function pluginRuntimeCanOpenSurface(workshop: PluginRuntimeWorkshop): boolean {
  return Boolean(
    (workshop.plugin.kind === 'ui_only' ||
      workshop.plugin.kind === 'service') &&
      workshop.plugin.lifecycle === 'enabled' &&
      workshop.plugin.surface_available &&
      workshop.plugin.releases.active &&
      workshop.plugin.releases.active_release_epoch > 0
  );
}

export function pluginRuntimeSurfaceMatchesWorkshop(
  descriptor: PluginRuntimeSurfaceLaunchDescriptor,
  workshop: PluginRuntimeWorkshop
): boolean {
  const active = workshop.plugin.releases.active;
  return Boolean(
    pluginRuntimeCanOpenSurface(workshop) &&
      active &&
      descriptor.plugin_id === workshop.plugin.plugin_id &&
      descriptor.release_id === active.release_id &&
      descriptor.expected_release_digest === active.release_digest &&
      descriptor.active_release_epoch ===
        workshop.plugin.releases.active_release_epoch &&
      descriptor.surface_session_id.trim().length > 0 &&
      Number.isSafeInteger(descriptor.surface_generation) &&
      descriptor.surface_generation > 0 &&
      descriptor.surface_capability.trim().length > 0 &&
      descriptor.kind === workshop.plugin.kind
  );
}

export function pluginRuntimeSurfaceAssetPath(
  descriptor: PluginRuntimeSurfaceLaunchDescriptor
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
  return `/api/plugins/runtimes/${encodeURIComponent(
    descriptor.plugin_id
  )}/surface/assets/${encodeURIComponent(
    descriptor.surface_capability
  )}/${descriptor.active_release_epoch}/${encodeURIComponent(
    descriptor.expected_release_digest
  )}/${encodedEntrypoint}`;
}

export function shortPluginRuntimeIdentity(
  value: string | undefined,
  edgeLength = 8
): string {
  if (!value) return '—';
  if (value.length <= edgeLength * 2 + 1) return value;
  return `${value.slice(0, edgeLength)}…${value.slice(-edgeLength)}`;
}

export function formatPluginRuntimeTimestamp(
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
