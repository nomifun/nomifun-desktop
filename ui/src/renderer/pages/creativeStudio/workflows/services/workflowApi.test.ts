/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { describe, expect, test } from 'bun:test';
import { createWorkflowFixture } from '../domain/testFixtures';
import {
  CreativeWorkflowContractError,
  createCreativeWorkflowApi,
  type CreativeWorkflowHttpRequest,
} from './workflowApi';

describe('Creative Workflow API client', () => {
  test('uses the canonical CRUD paths and exact wire bodies', async () => {
    const original = createWorkflowFixture();
    const replacement = {
      ...createWorkflowFixture(),
      revision: 2,
      metadata: { ...original.metadata, name: 'Updated workflow' },
    };
    const calls: Array<{ method: string; path: string; body?: unknown }> = [];
    const request: CreativeWorkflowHttpRequest = async (method, path, body) => {
      calls.push({ method, path, ...(body === undefined ? {} : { body }) });
      if (method === 'DELETE') return undefined;
      if (method === 'GET' && path.endsWith('/workflows')) {
        return { workflows: [original] };
      }
      return { workflow: method === 'PUT' ? replacement : original };
    };
    const api = createCreativeWorkflowApi(request);

    expect(await api.listWorkflows()).toEqual([original]);
    expect(await api.createWorkflow(original)).toEqual(original);
    expect(await api.getWorkflow(original.id)).toEqual(original);
    expect(
      await api.saveWorkflow(original.id, {
        expectedRevision: '1',
        workflow: replacement,
      })
    ).toEqual(replacement);
    await api.deleteWorkflow(original.id);

    expect(calls).toEqual([
      { method: 'GET', path: '/api/creative-studio/workflows' },
      {
        method: 'POST',
        path: '/api/creative-studio/workflows',
        body: { workflow: original },
      },
      { method: 'GET', path: `/api/creative-studio/workflows/${original.id}` },
      {
        method: 'PUT',
        path: `/api/creative-studio/workflows/${original.id}`,
        body: { expectedRevision: '1', workflow: replacement },
      },
      { method: 'DELETE', path: `/api/creative-studio/workflows/${original.id}` },
    ]);
  });

  test('fails closed on unknown response fields and route identity drift', async () => {
    const workflow = createWorkflowFixture();
    const unknown = createCreativeWorkflowApi(async () => ({
      workflows: [{ ...workflow, legacyPrompt: 'unsafe' }],
    }));
    try {
      await unknown.listWorkflows();
      throw new Error('expected unknown-field rejection');
    } catch (error) {
      expect(error instanceof CreativeWorkflowContractError).toBe(true);
      expect(error).toMatchObject({ code: 'unknown-field' });
    }

    const drifted = createCreativeWorkflowApi(async () => ({
      workflow: {
        ...workflow,
        id: '018f0000-0000-7000-8000-000000000099',
      },
    }));
    try {
      await drifted.getWorkflow(workflow.id);
      throw new Error('expected route identity rejection');
    } catch (error) {
      expect(error).toMatchObject({ code: 'identity-mismatch', path: '$.workflow.id' });
    }
  });

  test('validates writes before issuing a request', async () => {
    const workflow = createWorkflowFixture();
    let calls = 0;
    const api = createCreativeWorkflowApi(async () => {
      calls += 1;
      return { workflow };
    });
    try {
      await api.saveWorkflow(workflow.id, {
        expectedRevision: '01',
        workflow: { ...workflow, revision: 2 },
      });
      throw new Error('expected invalid revision rejection');
    } catch (error) {
      expect(error).toMatchObject({ code: 'invalid-value', path: '$.expectedRevision' });
    }
    expect(calls).toBe(0);
  });
});
