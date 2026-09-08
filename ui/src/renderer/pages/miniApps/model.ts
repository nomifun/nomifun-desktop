/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import type {
  BuildMiniAppRequest,
  MiniAppSummary,
  MiniAppWorkshop,
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

  return {
    source: sourceReady ? 'done' : 'pending',
    build: buildRunning
      ? 'active'
      : hasReady
        ? 'done'
        : sourceReady
          ? 'active'
          : 'blocked',
    ready: hasReady ? 'done' : 'pending',
  };
}

export function miniAppBuildRequest(
  workshop: MiniAppWorkshop
): BuildMiniAppRequest | null {
  if (
    workshop.miniapp.kind !== 'ui_only' ||
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
  };
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
