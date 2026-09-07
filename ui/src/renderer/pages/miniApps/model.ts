/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import type {
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

  return {
    source: sourceReady ? 'done' : 'pending',
    build: buildRunning ? 'active' : hasReady ? 'done' : 'blocked',
    ready: hasReady ? 'done' : 'pending',
    publish: hasActive
      ? 'done'
      : workshop.ready?.can_publish
        ? 'active'
        : hasReady
          ? 'blocked'
          : 'pending',
    surface: workshop.miniapp.surface_available
      ? 'done'
      : hasActive
        ? 'blocked'
        : 'pending',
  };
}

export function miniAppPublishBlockingReasons(
  workshop: MiniAppWorkshop
): string[] {
  const reasons = workshop.ready?.blocking_reasons ?? [];
  return [...new Set(reasons.map((reason) => reason.trim()).filter(Boolean))];
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
