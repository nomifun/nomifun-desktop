/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { describe, expect, test } from 'bun:test';

import { cloneTemplateRunAggregate } from '../domain';
import { IDS, createTemplateRunFixture } from '../domain/testFixtures';
import {
  buildTemplateTaskPlan,
  createImageTaskInput,
  createPlannerTaskInput,
  parsePlannerPromptDrafts,
} from './plan';

describe('template task planning', () => {
  test('builds one exact image task with provider-neutral parameters and references', () => {
    const run = createTemplateRunFixture();
    run.request.referenceAssetIds = [IDS.asset];
    const plan = buildTemplateTaskPlan(run.templateSnapshot, [IDS.task], () => IDS.task2);
    expect(plan).toHaveLength(1);
    const entry = plan[0];
    if (entry.kind !== 'image') throw new Error('expected image entry');
    expect(createImageTaskInput(run, entry)).toEqual({
      idempotencyKey: IDS.task,
      owner: {
        kind: 'template_step',
        templateId: IDS.template,
        templateRunId: IDS.request,
        templateStepId: IDS.generateStep,
      },
      providerId: IDS.provider,
      model: 'nomifun-image-test',
      task: 'image_edit',
      capability: 'i2i',
      parameters: {
        prompt: 'Create a poster for NomiFun',
        interface_mode: 'images',
        quality: 'auto',
        aspect: '1:1',
        count: 1,
        width: 1024,
        height: 1024,
      },
      inputs: [{ assetId: IDS.asset, kind: 'image', role: 'reference' }],
    });
  });

  test('preallocates planner and per-prompt image tasks in stable topological order', () => {
    const run = createTemplateRunFixture(true);
    const plan = buildTemplateTaskPlan(
      run.templateSnapshot,
      [IDS.task, IDS.task2, IDS.task3],
      () => IDS.result2
    );
    expect(plan.map((entry) => [entry.kind, entry.taskId])).toEqual([
      ['planner', IDS.task],
      ['image', IDS.task2],
      ['image', IDS.task3],
    ]);
    const planner = plan[0];
    if (planner.kind !== 'planner') throw new Error('expected planner entry');
    const input = createPlannerTaskInput(run, planner);
    expect(input).toMatchObject({
      idempotencyKey: IDS.task,
      task: 'chat',
      capability: 'text',
      providerId: IDS.provider,
      model: 'nomifun-chat-test',
      parameters: { max_tokens: 4096 },
      inputs: [],
    });
    expect(String(input.parameters.system).includes('Return only one JSON object')).toBe(true);
  });

  test('accepts only exact JSON planner output and assigns canonical review drafts', () => {
    const run = createTemplateRunFixture(true);
    const ids = [IDS.draft1, IDS.draft2];
    const drafts = parsePlannerPromptDrafts(
      JSON.stringify({
        prompts: [
          { title: 'Front', prompt: 'Front view' },
          { title: 'Detail', prompt: 'Detail view' },
        ],
      }),
      run,
      () => ids.shift() ?? IDS.result2,
      () => 2_100
    );
    expect(drafts).toEqual([
      {
        id: IDS.draft1,
        templateId: IDS.template,
        runRequestId: IDS.request,
        seriesIndex: 0,
        title: 'Front',
        prompt: 'Front view',
        status: 'pending-review',
        createdAt: 2_100,
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
        createdAt: 2_100,
        reviewedAt: null,
        reviewNote: null,
      },
    ]);

    for (const invalid of [
      '```json\n{"prompts":[]}\n```',
      JSON.stringify({ prompts: [{ title: 'Only', prompt: 'One' }] }),
      JSON.stringify({ prompts: [
        { title: 'Front', prompt: 'Front', extra: true },
        { title: 'Detail', prompt: 'Detail' },
      ] }),
    ]) {
      let rejected = false;
      try {
        parsePlannerPromptDrafts(invalid, cloneTemplateRunAggregate(run), () => IDS.draft1, () => 2_100);
      } catch (error) {
        rejected = true;
        expect(error).toMatchObject({ code: 'planner-output' });
      }
      expect(rejected).toBe(true);
    }
  });
});
