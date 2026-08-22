/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { describe, expect, test } from 'bun:test';

import { cloneWorkflowRunAggregate } from '../domain';
import { IDS, createWorkflowRunFixture } from '../domain/testFixtures';
import type { CreativeWorkflowHttpRequest } from './workflowApi';
import { createCreativeWorkflowRunApi } from './workflowRunApi';

describe('Creative Workflow run API client', () => {
  test('uses exact list, create, detail, and CAS save contracts', async () => {
    const requested = createWorkflowRunFixture();
    const queued = cloneWorkflowRunAggregate(requested);
    queued.revision = 2;
    queued.record.status = 'queued';
    queued.record.taskIds = [IDS.task];
    queued.record.queuedAt = 2_100;
    const calls: Array<{ method: string; path: string; body?: unknown }> = [];
    const request: CreativeWorkflowHttpRequest = async (method, path, body) => {
      calls.push({ method, path, ...(body === undefined ? {} : { body }) });
      if (method === 'GET' && path.includes('?workflowId=')) return { runs: [requested] };
      return { run: method === 'PUT' ? queued : requested };
    };
    const api = createCreativeWorkflowRunApi(request);

    expect(await api.listRuns(IDS.workflow)).toEqual([requested]);
    expect(await api.createRun({
      runId: IDS.request,
      workflowId: IDS.workflow,
      workflowRevision: 1,
      inputs: requested.request.inputs,
      referenceAssetIds: [],
    })).toEqual(requested);
    expect(await api.getRun(IDS.request)).toEqual(requested);
    expect(await api.saveRun(IDS.request, { expectedRevision: '1', run: queued })).toEqual(queued);

    expect(calls).toEqual([
      {
        method: 'GET',
        path: `/api/creative-studio/workflow-runs?workflowId=${IDS.workflow}`,
      },
      {
        method: 'POST',
        path: '/api/creative-studio/workflow-runs',
        body: {
          request: {
            runId: IDS.request,
            workflowId: IDS.workflow,
            workflowRevision: 1,
            inputs: requested.request.inputs,
            referenceAssetIds: [],
          },
        },
      },
      { method: 'GET', path: `/api/creative-studio/workflow-runs/${IDS.request}` },
      {
        method: 'PUT',
        path: `/api/creative-studio/workflow-runs/${IDS.request}`,
        body: { expectedRevision: '1', run: queued },
      },
    ]);
  });

  test('fails closed on unknown fields and response identity drift', async () => {
    const run = createWorkflowRunFixture();
    const unknown = createCreativeWorkflowRunApi(async () => ({
      run: { ...run, legacyState: 'unsafe' },
    }));
    try {
      await unknown.getRun(IDS.request);
      throw new Error('expected unknown-field rejection');
    } catch (error) {
      expect(error).toMatchObject({ code: 'unknown-field', path: '$.run.legacyState' });
    }

    const drifted = cloneWorkflowRunAggregate(run);
    drifted.request.id = IDS.idempotency;
    drifted.request.idempotencyKey = IDS.idempotency;
    drifted.record.requestId = IDS.idempotency;
    const identity = createCreativeWorkflowRunApi(async () => ({ run: drifted }));
    try {
      await identity.getRun(IDS.request);
      throw new Error('expected identity rejection');
    } catch (error) {
      expect(error).toMatchObject({
        code: 'identity-mismatch',
        path: '$.run.request.id',
      });
    }
  });

  test('validates writes before issuing an HTTP request', async () => {
    const run = createWorkflowRunFixture();
    let calls = 0;
    const api = createCreativeWorkflowRunApi(async () => {
      calls += 1;
      return { run };
    });
    try {
      await api.createRun({
        runId: IDS.request,
        workflowId: IDS.workflow,
        workflowRevision: 1,
        inputs: run.request.inputs,
        referenceAssetIds: [IDS.asset, IDS.asset],
      });
      throw new Error('expected duplicate-id rejection');
    } catch (error) {
      expect(error).toMatchObject({
        code: 'duplicate-id',
        path: '$.request.referenceAssetIds',
      });
    }
    expect(calls).toBe(0);
  });
});
