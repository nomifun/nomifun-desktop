/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { describe, expect, test } from 'bun:test';

import type { CreativeTemplateCommand } from './commands';
import { createTemplateWorkspaceDocumentV1 } from './model';
import { templateReducer } from './reducer';
import { IDS, createTemplateFixture } from './testFixtures';
import { validateTemplateWorkspaceDocument } from './validation';

function reduce(commands: CreativeTemplateCommand[]) {
  return commands.reduce(templateReducer, createTemplateWorkspaceDocumentV1());
}

const input = [{ variableId: IDS.variable, type: 'text' as const, value: 'NomiFun' }];

describe('template reducer', () => {
  test('creates, reads, updates, and deletes typed template definitions', () => {
    const template = createTemplateFixture();
    let state = reduce([{ type: 'template/create', template }]);
    expect(state.templates).toHaveLength(1);
    state = templateReducer(state, {
      type: 'template/update-metadata',
      templateId: IDS.template,
      patch: { name: 'Updated poster', tags: ['brand', 'poster'] },
      updatedAt: 2_000,
    });
    expect(state.templates[0].revision).toBe(2);
    expect(state.templates[0].metadata.name).toBe('Updated poster');

    const referencedDelete = templateReducer(state, {
      type: 'variable/delete',
      templateId: IDS.template,
      variableId: IDS.variable,
      updatedAt: 3_000,
    });
    expect(referencedDelete).toBe(state);

    state = templateReducer(state, { type: 'template/delete', templateId: IDS.template });
    expect(state.templates).toEqual([]);
  });

  test('rejects invalid edits instead of partially corrupting the graph', () => {
    const state = reduce([{ type: 'template/create', template: createTemplateFixture() }]);
    const cyclicStep = {
      ...state.templates[0].steps[0],
      dependsOn: [IDS.historyStep],
    };
    const next = templateReducer(state, {
      type: 'step/upsert',
      templateId: IDS.template,
      step: cyclicStep,
      updatedAt: 2_000,
    });
    expect(next).toBe(state);
  });

  test('runs a reviewed multi-image series through monotonic status transitions', () => {
    let state = reduce([
      { type: 'template/create', template: createTemplateFixture(true) },
      {
        type: 'run/request',
        id: IDS.request,
        idempotencyKey: IDS.request,
        templateId: IDS.template,
        requestedAt: 2_000,
        inputs: input,
        referenceAssetIds: [],
      },
    ]);
    expect(state.runs[0].status).toBe('awaiting-review');

    state = templateReducer(state, {
      type: 'prompt-draft/add',
      id: IDS.draft1,
      runRequestId: IDS.request,
      seriesIndex: 0,
      title: 'Front',
      prompt: 'Front view',
      createdAt: 2_100,
    });
    state = templateReducer(state, {
      type: 'prompt-draft/add',
      id: IDS.draft2,
      runRequestId: IDS.request,
      seriesIndex: 1,
      title: 'Detail',
      prompt: 'Detail view',
      createdAt: 2_101,
    });
    const rejected = templateReducer(state, {
      type: 'prompt-draft/reject',
      draftId: IDS.draft1,
      reviewedAt: 2_200,
      note: 'Needs brand color',
    });
    expect(rejected.promptDrafts[0].status).toBe('rejected');
    expect(
      templateReducer(rejected, { type: 'run/queue', requestId: IDS.request, queuedAt: 2_300 })
    ).toBe(rejected);

    state = templateReducer(rejected, {
      type: 'prompt-draft/edit',
      draftId: IDS.draft1,
      title: 'Front',
      prompt: 'Front view in brand colors',
    });
    state = templateReducer(state, {
      type: 'prompt-draft/approve',
      draftId: IDS.draft1,
      reviewedAt: 2_400,
      note: null,
    });
    state = templateReducer(state, {
      type: 'prompt-draft/approve',
      draftId: IDS.draft2,
      reviewedAt: 2_401,
      note: 'Approved',
    });
    state = templateReducer(state, {
      type: 'run/queue',
      requestId: IDS.request,
      queuedAt: 2_500,
    });
    state = templateReducer(state, {
      type: 'run/start',
      requestId: IDS.request,
      taskIds: [IDS.task],
      startedAt: 2_600,
    });
    state = templateReducer(state, {
      type: 'run/succeed',
      requestId: IDS.request,
      resultAssetIds: [IDS.asset],
      historyReferenceIds: [IDS.history],
      completedAt: 3_000,
    });
    expect(state.runs[0]).toMatchObject({
      status: 'succeeded',
      taskIds: [IDS.task],
      resultAssetIds: [IDS.asset],
      historyReferenceIds: [IDS.history],
    });
    expect(validateTemplateWorkspaceDocument(state).ok).toBe(true);

    const unchanged = templateReducer(state, {
      type: 'run/fail',
      requestId: IDS.request,
      code: 'late_failure',
      message: 'Late callback',
      completedAt: 3_100,
    });
    expect(unchanged).toBe(state);
  });

  test('keeps terminal external history references when a template is deleted', () => {
    let state = reduce([
      { type: 'template/create', template: createTemplateFixture() },
      {
        type: 'run/request',
        id: IDS.request,
        idempotencyKey: IDS.request,
        templateId: IDS.template,
        requestedAt: 2_000,
        inputs: input,
        referenceAssetIds: [],
      },
      { type: 'run/queue', requestId: IDS.request, queuedAt: 2_100 },
      { type: 'run/start', requestId: IDS.request, taskIds: [IDS.task], startedAt: 2_200 },
      {
        type: 'run/succeed',
        requestId: IDS.request,
        resultAssetIds: [IDS.asset],
        historyReferenceIds: [IDS.history],
        completedAt: 2_300,
      },
    ]);
    state = templateReducer(state, { type: 'template/delete', templateId: IDS.template });
    expect(state.templates).toEqual([]);
    expect(state.runs[0].historyReferenceIds).toEqual([IDS.history]);
    expect(validateTemplateWorkspaceDocument(state).ok).toBe(true);
  });

  test('does not delete a template while one of its runs is active', () => {
    const state = reduce([
      { type: 'template/create', template: createTemplateFixture() },
      {
        type: 'run/request',
        id: IDS.request,
        idempotencyKey: IDS.request,
        templateId: IDS.template,
        requestedAt: 2_000,
        inputs: input,
        referenceAssetIds: [],
      },
    ]);
    expect(templateReducer(state, { type: 'template/delete', templateId: IDS.template })).toBe(
      state
    );
  });
});
