/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { describe, expect, test } from 'bun:test';
import { renderToStaticMarkup } from 'react-dom/server';

import { cloneWorkflowRunAggregate } from '../domain';
import { IDS, createWorkflowRunFixture } from '../domain/testFixtures';
import WorkflowRunCenter from './WorkflowRunCenter';

describe('Workflow Run Center', () => {
  test('renders durable status, progress, recovery actions, and real result URLs', () => {
    const succeeded = createWorkflowRunFixture();
    succeeded.revision = 4;
    succeeded.record.status = 'succeeded';
    succeeded.record.taskIds = [IDS.task];
    succeeded.record.resultAssetIds = [IDS.asset];
    succeeded.record.queuedAt = 2_100;
    succeeded.record.startedAt = 2_200;
    succeeded.record.completedAt = 2_300;
    const paused = cloneWorkflowRunAggregate(succeeded);
    paused.request.id = IDS.idempotency;
    paused.request.idempotencyKey = IDS.idempotency;
    paused.record.requestId = IDS.idempotency;
    paused.revision = 3;
    paused.record.status = 'running';
    paused.record.resultAssetIds = [];
    paused.record.completedAt = null;

    const html = renderToStaticMarkup(
      <WorkflowRunCenter
        port={{
          snapshot: {
            loading: false,
            loadError: null,
            runs: [paused, succeeded],
            activities: {
              [IDS.idempotency]: {
                state: 'paused',
                taskStatuses: { [IDS.task]: 'running' },
                error: 'network offline',
              },
            },
          },
          assetUrl: (assetId) => `/api/creative-studio/files/${assetId}`,
          resume: async () => undefined,
          cancel: async () => undefined,
          review: async () => undefined,
          retry: async () => undefined,
        }}
      />
    );

    expect(html.includes('data-workflow-run-center="true"')).toBe(true);
    expect(html.includes('模板任务')).toBe(true);
    expect(html.includes('等待恢复')).toBe(true);
    expect(html.includes('继续')).toBe(true);
    expect(html.includes('已完成')).toBe(true);
    expect(html.includes(`/api/creative-studio/files/${IDS.asset}`)).toBe(true);
    expect(html.includes('刷新或重启后会继续恢复')).toBe(true);
  });
});
