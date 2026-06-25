/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { describe, expect, test } from 'bun:test';
import type { ICronJob } from '@/common/adapter/ipcBridge';
import { filterCronJobsByQuery } from './cronJobSearch';

function job(overrides: Partial<ICronJob>): ICronJob {
  return {
    id: 'cron_alpha',
    name: 'Daily standup',
    description: 'Summarize project work',
    enabled: true,
    schedule: { kind: 'cron', expr: '0 0 9 * * ?', description: 'Every day at 09:00' },
    target: {
      payload: { kind: 'message', text: 'Collect yesterday progress' },
      execution_mode: 'new_conversation',
      target_kind: 'agent',
    },
    metadata: {
      conversation_id: 1001,
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
    job({ id: 'cron_alpha', name: 'Daily standup' }),
    job({
      id: 'cron_beta',
      name: 'Release notes',
      description: 'Prepare customer changelog',
      schedule: { kind: 'cron', expr: '0 30 17 * * ?', description: 'Every day at 17:30' },
      target: {
        payload: { kind: 'message', text: 'Draft the changelog from merged PRs' },
        execution_mode: 'existing',
        target_kind: 'agent',
      },
      metadata: {
        conversation_id: 2002,
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

  test('matches job metadata, prompt, schedule, and target fields case-insensitively', () => {
    expect(filterCronJobsByQuery(jobs, 'launch').map((item) => item.id)).toEqual(['cron_beta']);
    expect(filterCronJobsByQuery(jobs, 'MERGED prs').map((item) => item.id)).toEqual(['cron_beta']);
    expect(filterCronJobsByQuery(jobs, '09:00').map((item) => item.id)).toEqual(['cron_alpha']);
  });
});
