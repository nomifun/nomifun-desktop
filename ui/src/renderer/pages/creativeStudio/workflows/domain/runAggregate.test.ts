/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { describe, expect, test } from 'bun:test';

import {
  cloneWorkflowRunAggregate,
  expectedWorkflowRunResultCount,
  expectedWorkflowRunTaskCount,
  validateWorkflowRunAggregate,
  validateWorkflowRunTransition,
} from './runAggregate';
import { IDS, createWorkflowRunFixture } from './testFixtures';

describe('workflow run aggregate v1', () => {
  test('validates and clones a server-authoritative requested run', () => {
    const run = createWorkflowRunFixture();
    expect(validateWorkflowRunAggregate(run)).toEqual({ ok: true });
    expect(expectedWorkflowRunTaskCount(run.workflowSnapshot)).toBe(1);
    expect(expectedWorkflowRunResultCount(run.workflowSnapshot)).toBe(1);

    const clone = cloneWorkflowRunAggregate(run);
    clone.request.inputs[0] = {
      variableId: IDS.variable,
      type: 'text',
      value: 'Changed',
    };
    expect(run.request.inputs[0]).toMatchObject({ value: 'NomiFun' });
  });

  test('fails closed on unknown fields, identity drift, and missing model bindings', () => {
    const run = createWorkflowRunFixture();
    const unknown = {
      ...run,
      legacyState: 'unsafe',
    };
    expect(validateWorkflowRunAggregate(unknown)).toMatchObject({
      ok: false,
      error: { code: 'unknown-field', path: '$.legacyState' },
    });

    const drifted = cloneWorkflowRunAggregate(run);
    drifted.request.idempotencyKey = IDS.idempotency;
    expect(validateWorkflowRunAggregate(drifted)).toMatchObject({
      ok: false,
      error: { path: '$.request.idempotencyKey' },
    });

    const unbound = cloneWorkflowRunAggregate(run);
    const generate = unbound.workflowSnapshot.steps.find(
      (step) => step.kind === 'generate-images'
    );
    if (!generate || generate.kind !== 'generate-images') throw new Error('missing image step');
    generate.generation.model = null;
    expect(validateWorkflowRunAggregate(unbound)).toMatchObject({
      ok: false,
      error: { path: '$.workflowSnapshot.steps[0].generation.model' },
    });
  });

  test('enforces exact task/result counts and monotonic terminal transitions', () => {
    const requested = createWorkflowRunFixture();
    const queued = cloneWorkflowRunAggregate(requested);
    queued.revision = 2;
    queued.record.status = 'queued';
    queued.record.taskIds = [IDS.task];
    queued.record.queuedAt = 2_100;
    expect(validateWorkflowRunTransition(requested, queued).ok).toBe(true);

    const running = cloneWorkflowRunAggregate(queued);
    running.revision = 3;
    running.record.status = 'running';
    running.record.startedAt = 2_200;
    expect(validateWorkflowRunTransition(queued, running).ok).toBe(true);

    const succeeded = cloneWorkflowRunAggregate(running);
    succeeded.revision = 4;
    succeeded.record.status = 'succeeded';
    succeeded.record.resultAssetIds = [IDS.asset];
    succeeded.record.completedAt = 2_300;
    expect(validateWorkflowRunTransition(running, succeeded).ok).toBe(true);
    expect(validateWorkflowRunTransition(succeeded, succeeded)).toMatchObject({
      ok: false,
      error: { code: 'invalid-transition' },
    });

    const incomplete = cloneWorkflowRunAggregate(running);
    incomplete.revision = 4;
    incomplete.record.status = 'succeeded';
    incomplete.record.completedAt = 2_300;
    expect(validateWorkflowRunAggregate(incomplete).ok).toBe(false);
  });

  test('persists planner output before review and permits edits only during review', () => {
    const requested = createWorkflowRunFixture(true);
    expect(expectedWorkflowRunTaskCount(requested.workflowSnapshot)).toBe(3);
    expect(expectedWorkflowRunResultCount(requested.workflowSnapshot)).toBe(2);

    const queued = cloneWorkflowRunAggregate(requested);
    queued.revision = 2;
    queued.record.status = 'queued';
    queued.record.taskIds = [IDS.task, IDS.task2, IDS.task3];
    queued.record.queuedAt = 2_100;

    const planning = cloneWorkflowRunAggregate(queued);
    planning.revision = 3;
    planning.record.status = 'running';
    planning.record.startedAt = 2_200;
    expect(validateWorkflowRunTransition(queued, planning).ok).toBe(true);

    const review = cloneWorkflowRunAggregate(planning);
    review.revision = 4;
    review.record.status = 'awaiting-review';
    review.promptDrafts = [
      {
        id: IDS.draft1,
        workflowId: IDS.workflow,
        runRequestId: IDS.request,
        seriesIndex: 0,
        title: 'Front',
        prompt: 'Front view',
        status: 'pending-review',
        createdAt: 2_300,
        reviewedAt: null,
        reviewNote: null,
      },
      {
        id: IDS.draft2,
        workflowId: IDS.workflow,
        runRequestId: IDS.request,
        seriesIndex: 1,
        title: 'Detail',
        prompt: 'Detail view',
        status: 'pending-review',
        createdAt: 2_300,
        reviewedAt: null,
        reviewNote: null,
      },
    ];
    review.record.promptDraftIds = [IDS.draft1, IDS.draft2];
    expect(validateWorkflowRunTransition(planning, review).ok).toBe(true);

    const approved = cloneWorkflowRunAggregate(review);
    approved.revision = 5;
    approved.promptDrafts = approved.promptDrafts.map((draft) => ({
      ...draft,
      status: 'approved',
      reviewedAt: 2_400,
    }));
    expect(validateWorkflowRunTransition(review, approved).ok).toBe(true);

    const imagePhase = cloneWorkflowRunAggregate(approved);
    imagePhase.revision = 6;
    imagePhase.record.status = 'running';
    expect(validateWorkflowRunTransition(approved, imagePhase).ok).toBe(true);

    const illegalEdit = cloneWorkflowRunAggregate(imagePhase);
    illegalEdit.revision = 7;
    illegalEdit.promptDrafts[0].prompt = 'Silent rewrite';
    expect(validateWorkflowRunTransition(imagePhase, illegalEdit)).toMatchObject({
      ok: false,
      error: { code: 'invalid-transition' },
    });
  });
});
