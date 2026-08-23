/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { describe, expect, test } from 'bun:test';

import {
  cloneTemplateRunAggregate,
  expectedTemplateRunResultCount,
  expectedTemplateRunTaskCount,
  validateTemplateRunAggregate,
  validateTemplateRunTransition,
} from './runAggregate';
import { IDS, createTemplateRunFixture } from './testFixtures';

describe('template run aggregate v1', () => {
  test('validates and clones a server-authoritative requested run', () => {
    const run = createTemplateRunFixture();
    expect(validateTemplateRunAggregate(run)).toEqual({ ok: true });
    expect(expectedTemplateRunTaskCount(run.templateSnapshot)).toBe(1);
    expect(expectedTemplateRunResultCount(run.templateSnapshot)).toBe(1);

    const clone = cloneTemplateRunAggregate(run);
    clone.request.inputs[0] = {
      variableId: IDS.variable,
      type: 'text',
      value: 'Changed',
    };
    expect(run.request.inputs[0]).toMatchObject({ value: 'NomiFun' });
  });

  test('fails closed on unknown fields, identity drift, and missing model bindings', () => {
    const run = createTemplateRunFixture();
    const unknown = {
      ...run,
      legacyState: 'unsafe',
    };
    expect(validateTemplateRunAggregate(unknown)).toMatchObject({
      ok: false,
      error: { code: 'unknown-field', path: '$.legacyState' },
    });

    const drifted = cloneTemplateRunAggregate(run);
    drifted.request.idempotencyKey = IDS.idempotency;
    expect(validateTemplateRunAggregate(drifted)).toMatchObject({
      ok: false,
      error: { path: '$.request.idempotencyKey' },
    });

    const unbound = cloneTemplateRunAggregate(run);
    const generate = unbound.templateSnapshot.steps.find(
      (step) => step.kind === 'generate-images'
    );
    if (!generate || generate.kind !== 'generate-images') throw new Error('missing image step');
    generate.generation.model = null;
    expect(validateTemplateRunAggregate(unbound)).toMatchObject({
      ok: false,
      error: { path: '$.templateSnapshot.steps[0].generation.model' },
    });
  });

  test('enforces exact task/result counts and monotonic terminal transitions', () => {
    const requested = createTemplateRunFixture();
    const queued = cloneTemplateRunAggregate(requested);
    queued.revision = 2;
    queued.record.status = 'queued';
    queued.record.taskIds = [IDS.task];
    queued.record.queuedAt = 2_100;
    expect(validateTemplateRunTransition(requested, queued).ok).toBe(true);

    const running = cloneTemplateRunAggregate(queued);
    running.revision = 3;
    running.record.status = 'running';
    running.record.startedAt = 2_200;
    expect(validateTemplateRunTransition(queued, running).ok).toBe(true);

    const succeeded = cloneTemplateRunAggregate(running);
    succeeded.revision = 4;
    succeeded.record.status = 'succeeded';
    succeeded.record.resultAssetIds = [IDS.asset];
    succeeded.record.completedAt = 2_300;
    expect(validateTemplateRunTransition(running, succeeded).ok).toBe(true);
    expect(validateTemplateRunTransition(succeeded, succeeded)).toMatchObject({
      ok: false,
      error: { code: 'invalid-transition' },
    });

    const incomplete = cloneTemplateRunAggregate(running);
    incomplete.revision = 4;
    incomplete.record.status = 'succeeded';
    incomplete.record.completedAt = 2_300;
    expect(validateTemplateRunAggregate(incomplete).ok).toBe(false);
  });

  test('persists planner output before review and permits edits only during review', () => {
    const requested = createTemplateRunFixture(true);
    expect(expectedTemplateRunTaskCount(requested.templateSnapshot)).toBe(3);
    expect(expectedTemplateRunResultCount(requested.templateSnapshot)).toBe(2);

    const queued = cloneTemplateRunAggregate(requested);
    queued.revision = 2;
    queued.record.status = 'queued';
    queued.record.taskIds = [IDS.task, IDS.task2, IDS.task3];
    queued.record.queuedAt = 2_100;

    const planning = cloneTemplateRunAggregate(queued);
    planning.revision = 3;
    planning.record.status = 'running';
    planning.record.startedAt = 2_200;
    expect(validateTemplateRunTransition(queued, planning).ok).toBe(true);

    const review = cloneTemplateRunAggregate(planning);
    review.revision = 4;
    review.record.status = 'awaiting-review';
    review.promptDrafts = [
      {
        id: IDS.draft1,
        templateId: IDS.template,
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
        templateId: IDS.template,
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
    expect(validateTemplateRunTransition(planning, review).ok).toBe(true);

    const approved = cloneTemplateRunAggregate(review);
    approved.revision = 5;
    approved.promptDrafts = approved.promptDrafts.map((draft) => ({
      ...draft,
      status: 'approved',
      reviewedAt: 2_400,
    }));
    expect(validateTemplateRunTransition(review, approved).ok).toBe(true);

    const imagePhase = cloneTemplateRunAggregate(approved);
    imagePhase.revision = 6;
    imagePhase.record.status = 'running';
    expect(validateTemplateRunTransition(approved, imagePhase).ok).toBe(true);

    const illegalEdit = cloneTemplateRunAggregate(imagePhase);
    illegalEdit.revision = 7;
    illegalEdit.promptDrafts[0].prompt = 'Silent rewrite';
    expect(validateTemplateRunTransition(imagePhase, illegalEdit)).toMatchObject({
      ok: false,
      error: { code: 'invalid-transition' },
    });
  });
});
