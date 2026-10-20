/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import type { ICronJob } from '@/common/adapter/ipcBridge';
import { shortSessionId } from '@renderer/utils/ui/shortId';
import { scheduledWorkspace } from './scheduledAgentPresentation';

export type CronJobStatusFilter = 'all' | 'active' | 'paused';

function normalizeSearchText(value: unknown): string {
  return String(value ?? '')
    .trim()
    .toLowerCase();
}

function pushSearchValue(parts: string[], value: unknown) {
  const normalized = normalizeSearchText(value);
  if (normalized) parts.push(normalized);
}

function buildCronJobSearchText(job: ICronJob): string {
  const parts: string[] = [];
  pushSearchValue(parts, job.cron_job_id);
  pushSearchValue(parts, shortSessionId(job.cron_job_id));
  pushSearchValue(parts, job.name);
  pushSearchValue(parts, job.description);
  pushSearchValue(parts, job.schedule.description);
  if (job.schedule.kind === 'cron') pushSearchValue(parts, job.schedule.expr);
  pushSearchValue(parts, job.message);
  pushSearchValue(parts, job.execution_mode);
  if (job.metadata.conversation_id) {
    pushSearchValue(parts, job.metadata.conversation_id);
    pushSearchValue(parts, shortSessionId(job.metadata.conversation_id));
  }
  pushSearchValue(parts, job.metadata.conversation_title);
  pushSearchValue(parts, job.metadata.agent_label);
  const selection = job.metadata.agent_selection;
  pushSearchValue(parts, selection?.kind === 'template' ? selection.template_key : selection?.preset_id);
  pushSearchValue(parts, job.metadata.model?.provider_id);
  pushSearchValue(parts, job.metadata.model?.model);
  pushSearchValue(parts, scheduledWorkspace(job));

  return parts.join(' ');
}

export function filterCronJobsByQuery(jobs: ICronJob[], query: string): ICronJob[] {
  const normalizedQuery = normalizeSearchText(query);
  if (!normalizedQuery) return jobs;

  return jobs.filter((job) => buildCronJobSearchText(job).includes(normalizedQuery));
}

export function filterCronJobsByStatus(
  jobs: ICronJob[],
  statusFilter: CronJobStatusFilter
): ICronJob[] {
  if (statusFilter === 'all') return jobs;

  return jobs.filter((job) => job.enabled === (statusFilter === 'active'));
}
