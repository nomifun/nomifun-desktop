/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import type { CreativeStudioProjectSummary } from './types';

export const mergeProjects = (
  current: readonly CreativeStudioProjectSummary[],
  incoming: readonly CreativeStudioProjectSummary[]
): CreativeStudioProjectSummary[] => {
  const merged = new Map(current.map((project) => [project.id, project]));
  for (const project of incoming) merged.set(project.id, project);
  return [...merged.values()];
};

export const pruneProjectSelection = (selectedIds: ReadonlySet<string>, projects: readonly CreativeStudioProjectSummary[]) => {
  const present = new Set(projects.map((project) => project.id));
  return new Set([...selectedIds].filter((id) => present.has(id)));
};

export const formatProjectTimestamp = (value: number, language: string | undefined): string => {
  const date = new Date(value);
  if (Number.isNaN(date.valueOf())) return '—';
  return new Intl.DateTimeFormat(language?.toLowerCase().startsWith('zh') ? 'zh-CN' : 'en-US', {
    month: '2-digit',
    day: '2-digit',
    hour: '2-digit',
    minute: '2-digit',
  }).format(date);
};

export const projectErrorMessage = (error: unknown): string =>
  error instanceof Error && error.message.trim() ? error.message : String(error || 'Unknown error');
