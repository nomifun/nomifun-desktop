/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { describe, expect, test } from 'bun:test';

import type { WorkflowCommand } from './commands';
import { createWorkflowWorkspaceDocumentV1 } from './model';
import { workflowReducer } from './reducer';
import { IDS, createWorkflowFixture } from './testFixtures';
import { validateWorkflowWorkspaceDocument } from './validation';

function reduce(commands: WorkflowCommand[]) {
  return commands.reduce(workflowReducer, createWorkflowWorkspaceDocumentV1());
}

const input = [{ variableId: IDS.variable, type: 'text' as const, value: 'NomiFun' }];

describe('workflow reducer', () => {
  test('creates, reads, updates, and deletes typed workflow definitions', () => {
    const workflow = createWorkflowFixture();
    let state = reduce([{ type: 'workflow/create', workflow }]);
    expect(state.workflows).toHaveLength(1);
    state = workflowReducer(state, {
      type: 'workflow/update-metadata',
      workflowId: IDS.workflow,
      patch: { name: 'Updated poster', tags: ['brand', 'poster'] },
      updatedAt: 2_000,
    });
    expect(state.workflows[0].revision).toBe(2);
    expect(state.workflows[0].metadata.name).toBe('Updated poster');

    const referencedDelete = workflowReducer(state, {
      type: 'variable/delete',
      workflowId: IDS.workflow,
      variableId: IDS.variable,
      updatedAt: 3_000,
    });
    expect(referencedDelete).toBe(state);

    state = workflowReducer(state, { type: 'workflow/delete', workflowId: IDS.workflow });
    expect(state.workflows).toEqual([]);
  });

  test('rejects invalid edits instead of partially corrupting the graph', () => {
    const state = reduce([{ type: 'workflow/create', workflow: createWorkflowFixture() }]);
    const cyclicStep = {
      ...state.workflows[0].steps[0],
      dependsOn: [IDS.historyStep],
    };
    const next = workflowReducer(state, {
      type: 'step/upsert',
      workflowId: IDS.workflow,
      step: cyclicStep,
      updatedAt: 2_000,
    });
    expect(next).toBe(state);
  });

  test('runs a reviewed multi-image series through monotonic status transitions', () => {
    let state = reduce([
      { type: 'workflow/create', workflow: createWorkflowFixture(true) },
      {
        type: 'run/request',
        id: IDS.request,
        idempotencyKey: IDS.idempotency,
        workflowId: IDS.workflow,
        requestedAt: 2_000,
        inputs: input,
      },
    ]);
    expect(state.runs[0].status).toBe('awaiting-review');

    state = workflowReducer(state, {
      type: 'prompt-draft/add',
      id: IDS.draft1,
      runRequestId: IDS.request,
      seriesIndex: 0,
      title: 'Front',
      prompt: 'Front view',
      createdAt: 2_100,
    });
    state = workflowReducer(state, {
      type: 'prompt-draft/add',
      id: IDS.draft2,
      runRequestId: IDS.request,
      seriesIndex: 1,
      title: 'Detail',
      prompt: 'Detail view',
      createdAt: 2_101,
    });
    const rejected = workflowReducer(state, {
      type: 'prompt-draft/reject',
      draftId: IDS.draft1,
      reviewedAt: 2_200,
      note: 'Needs brand color',
    });
    expect(rejected.promptDrafts[0].status).toBe('rejected');
    expect(
      workflowReducer(rejected, { type: 'run/queue', requestId: IDS.request, queuedAt: 2_300 })
    ).toBe(rejected);

    state = workflowReducer(rejected, {
      type: 'prompt-draft/edit',
      draftId: IDS.draft1,
      title: 'Front',
      prompt: 'Front view in brand colors',
    });
    state = workflowReducer(state, {
      type: 'prompt-draft/approve',
      draftId: IDS.draft1,
      reviewedAt: 2_400,
      note: null,
    });
    state = workflowReducer(state, {
      type: 'prompt-draft/approve',
      draftId: IDS.draft2,
      reviewedAt: 2_401,
      note: 'Approved',
    });
    state = workflowReducer(state, {
      type: 'run/queue',
      requestId: IDS.request,
      queuedAt: 2_500,
    });
    state = workflowReducer(state, {
      type: 'run/start',
      requestId: IDS.request,
      taskIds: [IDS.task],
      startedAt: 2_600,
    });
    state = workflowReducer(state, {
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
    expect(validateWorkflowWorkspaceDocument(state).ok).toBe(true);

    const unchanged = workflowReducer(state, {
      type: 'run/fail',
      requestId: IDS.request,
      code: 'late_failure',
      message: 'Late callback',
      completedAt: 3_100,
    });
    expect(unchanged).toBe(state);
  });

  test('keeps terminal external history references when a workflow is deleted', () => {
    let state = reduce([
      { type: 'workflow/create', workflow: createWorkflowFixture() },
      {
        type: 'run/request',
        id: IDS.request,
        idempotencyKey: IDS.idempotency,
        workflowId: IDS.workflow,
        requestedAt: 2_000,
        inputs: input,
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
    state = workflowReducer(state, { type: 'workflow/delete', workflowId: IDS.workflow });
    expect(state.workflows).toEqual([]);
    expect(state.runs[0].historyReferenceIds).toEqual([IDS.history]);
    expect(validateWorkflowWorkspaceDocument(state).ok).toBe(true);
  });

  test('does not delete a workflow while one of its runs is active', () => {
    const state = reduce([
      { type: 'workflow/create', workflow: createWorkflowFixture() },
      {
        type: 'run/request',
        id: IDS.request,
        idempotencyKey: IDS.idempotency,
        workflowId: IDS.workflow,
        requestedAt: 2_000,
        inputs: input,
      },
    ]);
    expect(workflowReducer(state, { type: 'workflow/delete', workflowId: IDS.workflow })).toBe(
      state
    );
  });
});
