/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { BackendHttpError } from '@/common/adapter/httpBridge';
import { describe, expect, test } from 'bun:test';
import { createWorkflowFixture } from '../domain/testFixtures';
import type { CreativeWorkflowApi } from './workflowApi';
import {
  CreativeWorkflowRepositoryError,
  createCreativeWorkflowRepository,
} from './workflowRepository';

const workflow = createWorkflowFixture();

const apiStub = (overrides: Partial<CreativeWorkflowApi> = {}): CreativeWorkflowApi => ({
  listWorkflows: async () => [workflow],
  createWorkflow: async (definition) => definition,
  getWorkflow: async () => workflow,
  saveWorkflow: async (_workflowId, request) => request.workflow,
  deleteWorkflow: async () => undefined,
  ...overrides,
});

describe('Creative Workflow repository', () => {
  test('exposes a revision-safe product persistence port', async () => {
    const calls: unknown[] = [];
    const repository = createCreativeWorkflowRepository(
      apiStub({
        saveWorkflow: async (workflowId, request) => {
          calls.push({ workflowId, request });
          return request.workflow;
        },
      })
    );
    const replacement = {
      ...workflow,
      revision: 2,
      metadata: { ...workflow.metadata, name: 'Updated' },
    };

    expect(await repository.list()).toEqual([workflow]);
    expect(await repository.load(workflow.id)).toEqual(workflow);
    expect(await repository.save(workflow.id, 1, replacement)).toEqual(replacement);
    expect(calls).toEqual([
      {
        workflowId: workflow.id,
        request: { expectedRevision: '1', workflow: replacement },
      },
    ]);
  });

  test('rejects revision drift before crossing the API boundary', async () => {
    let calls = 0;
    const repository = createCreativeWorkflowRepository(
      apiStub({
        saveWorkflow: async () => {
          calls += 1;
          return workflow;
        },
      })
    );
    try {
      await repository.save(workflow.id, 1, workflow);
      throw new Error('expected revision rejection');
    } catch (error) {
      expect(error).toMatchObject({ kind: 'invalid-request' });
    }
    expect(calls).toBe(0);
  });

  test('maps stale and missing backend states to stable error kinds', async () => {
    const stale = createCreativeWorkflowRepository(
      apiStub({
        saveWorkflow: async () =>
          Promise.reject(
            new BackendHttpError({
              method: 'PUT',
              path: `/api/creative-studio/workflows/${workflow.id}`,
              status: 409,
              body: { code: 'CONFLICT', error: 'workflow revision conflict' },
            })
          ),
      })
    );
    try {
      await stale.save(workflow.id, 1, { ...workflow, revision: 2 });
      throw new Error('expected conflict');
    } catch (error) {
      expect(error instanceof CreativeWorkflowRepositoryError).toBe(true);
      expect(error).toMatchObject({
        kind: 'revision-conflict',
        status: 409,
        backendCode: 'CONFLICT',
      });
    }

    const missing = createCreativeWorkflowRepository(
      apiStub({
        getWorkflow: async () =>
          Promise.reject(
            new BackendHttpError({
              method: 'GET',
              path: `/api/creative-studio/workflows/${workflow.id}`,
              status: 404,
              body: { code: 'NOT_FOUND', error: 'workflow not found' },
            })
          ),
      })
    );
    try {
      await missing.load(workflow.id);
      throw new Error('expected missing workflow');
    } catch (error) {
      expect(error).toMatchObject({ kind: 'not-found', status: 404 });
    }
  });
});
