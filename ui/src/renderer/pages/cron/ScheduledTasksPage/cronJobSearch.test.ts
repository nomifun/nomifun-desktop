/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { describe, expect, test } from 'bun:test';
import { parseConversationId, parseCronJobId } from '@/common/types/ids';
import type { ICronJob } from '@/common/adapter/ipcBridge';
import { filterCronJobsByQuery } from './cronJobSearch';

function job(overrides: Partial<ICronJob>): ICronJob {
  return {
    id: parseCronJobId('cron_019b0000-0000-7000-8000-000000000001'),
    name: 'Daily standup',
    description: 'Summarize project work',
    enabled: true,
    schedule: { kind: 'cron', expr: '0 0 9 * * ?', description: 'Every day at 09:00' },
    message: 'Collect yesterday progress',
    execution_mode: 'new_conversation',
    metadata: {
      conversation_id: parseConversationId('conv_0190f5fe-7c00-7a00-8000-000000000101'),
      conversation_title: 'Engineering Room',
      agent_type: 'claude',
      created_by: 'user',
      created_at: 1,
      updated_at: 1,
      agent_config: { backend: 'claude', name: 'Claude Code' },
    },
    state: {
      run_count: 0,
      retry_count: 0,
      max_retries: 0,
    },
    ...overrides,
  };
}

describe('filterCronJobsByQuery', () => {
  const jobs = [
    job({ id: parseCronJobId('cron_019b0000-0000-7000-8000-000000000001'), name: 'Daily standup' }),
    job({
      id: parseCronJobId('cron_019b0000-0000-7000-8000-000000000002'),
      name: 'Release notes',
      description: 'Prepare customer changelog',
      schedule: { kind: 'cron', expr: '0 30 17 * * ?', description: 'Every day at 17:30' },
      message: 'Draft the changelog from merged PRs',
      execution_mode: 'existing',
      metadata: {
        conversation_id: parseConversationId('conv_0190f5fe-7c00-7a00-8000-000000000102'),
        conversation_title: 'Launch Plan',
        agent_type: 'nomi',
        created_by: 'user',
        created_at: 2,
        updated_at: 2,
        agent_config: { backend: 'nomi-provider', name: 'Nomi' },
      },
    }),
  ];

  test('returns every job for a blank query', () => {
    expect(filterCronJobsByQuery(jobs, '   ')).toEqual(jobs);
  });

  test('matches job metadata, message, schedule, and execution fields case-insensitively', () => {
    expect(filterCronJobsByQuery(jobs, 'launch').map((item) => item.id)).toEqual([jobs[1].id]);
    expect(filterCronJobsByQuery(jobs, 'MERGED prs').map((item) => item.id)).toEqual([jobs[1].id]);
    expect(filterCronJobsByQuery(jobs, '09:00').map((item) => item.id)).toEqual([jobs[0].id]);
  });
});
