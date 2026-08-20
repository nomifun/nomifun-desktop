/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { describe, expect, test } from 'bun:test';
import {
  createEmptyCreativeProjectDocument,
  CreativeStudioContractError,
} from '../domain';
import {
  createCreativeStudioProjectApi,
  type CreativeStudioHttpRequest,
} from './projectApi';

const PROJECT_ID = '0198f8bb-8424-7b3d-8f17-bc6a1676f112';
const summary = {
  projectId: PROJECT_ID,
  title: 'Untitled canvas',
  revision: '1',
  nodeCount: 0,
  createdAt: 1_770_000_000_000,
  updatedAt: 1_770_000_000_000,
};

describe('Creative Studio project API client', () => {
  test('uses the agreed endpoint and validates every response', async () => {
    const calls: Array<{ method: string; path: string; body?: unknown }> = [];
    const request: CreativeStudioHttpRequest = async (method, path, body) => {
      calls.push({ method, path, ...(body === undefined ? {} : { body }) });
      if (method === 'GET' && path.endsWith('/projects')) return { projects: [summary] };
      if (method === 'GET') {
        return { project: summary, document: createEmptyCreativeProjectDocument(PROJECT_ID) };
      }
      if (method === 'DELETE') return undefined;
      return { project: summary };
    };
    const api = createCreativeStudioProjectApi(request);

    await api.listProjects();
    await api.createProject({ title: 'Untitled canvas' });
    await api.getProject(PROJECT_ID);
    await api.renameProject(PROJECT_ID, { title: 'Renamed' });
    await api.saveProject(PROJECT_ID, {
      expectedRevision: '1',
      document: createEmptyCreativeProjectDocument(PROJECT_ID),
    });
    await api.deleteProject(PROJECT_ID);

    expect(calls).toEqual([
      { method: 'GET', path: '/api/creative-studio/projects' },
      {
        method: 'POST',
        path: '/api/creative-studio/projects',
        body: { title: 'Untitled canvas' },
      },
      { method: 'GET', path: `/api/creative-studio/projects/${PROJECT_ID}` },
      {
        method: 'PATCH',
        path: `/api/creative-studio/projects/${PROJECT_ID}`,
        body: { title: 'Renamed' },
      },
      {
        method: 'PUT',
        path: `/api/creative-studio/projects/${PROJECT_ID}/document`,
        body: {
          expectedRevision: '1',
          document: createEmptyCreativeProjectDocument(PROJECT_ID),
        },
      },
      { method: 'DELETE', path: `/api/creative-studio/projects/${PROJECT_ID}` },
    ]);
  });

  test('rejects server identity drift even when the payload is otherwise valid', async () => {
    const otherId = '0198f8bb-8424-7b3d-8f17-bc6a1676f113';
    const api = createCreativeStudioProjectApi(async () => ({
      project: { ...summary, projectId: otherId },
      document: createEmptyCreativeProjectDocument(otherId),
    }));

    try {
      await api.getProject(PROJECT_ID);
      throw new Error('Expected identity mismatch');
    } catch (error) {
      expect(error instanceof CreativeStudioContractError).toBe(true);
    }
  });

  test('rejects unknown response fields instead of normalizing them away', async () => {
    const api = createCreativeStudioProjectApi(async () => ({
      projects: [{ ...summary, legacyCanvasId: 'old' }],
    }));

    try {
      await api.listProjects();
      throw new Error('Expected response contract failure');
    } catch (error) {
      expect(error instanceof CreativeStudioContractError).toBe(true);
    }
  });
});
