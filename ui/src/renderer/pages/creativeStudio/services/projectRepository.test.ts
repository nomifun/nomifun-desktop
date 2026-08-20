/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { BackendHttpError } from '@/common/adapter/httpBridge';
import { describe, expect, test } from 'bun:test';
import { createEmptyCreativeProjectDocument } from '../domain';
import type { CreativeStudioProjectApi } from './projectApi';
import {
  createCreativeProjectRepository,
  CreativeProjectRepositoryError,
} from './projectRepository';
import {
  sortCreativeProjectSummaries,
  upsertCreativeProjectSummary,
} from './useCreativeProjects';

const PROJECT_ID = '0198f8bb-8424-7b3d-8f17-bc6a1676f112';
const project = {
  projectId: PROJECT_ID,
  title: 'Project',
  revision: '1',
  nodeCount: 0,
  connectionCount: 0,
  createdAt: 100,
  updatedAt: 200,
};

const apiStub = (overrides: Partial<CreativeStudioProjectApi> = {}): CreativeStudioProjectApi => ({
  listProjects: async () => [project],
  createProject: async () => project,
  getProject: async () => ({ project, document: createEmptyCreativeProjectDocument(PROJECT_ID) }),
  renameProject: async () => project,
  deleteProject: async () => undefined,
  saveProject: async () => project,
  ...overrides,
});

describe('Creative Project repository', () => {
  test('presents the narrow product persistence port', async () => {
    const calls: unknown[] = [];
    const repository = createCreativeProjectRepository(
      apiStub({
        saveProject: async (projectId, request) => {
          calls.push({ projectId, request });
          return { ...project, revision: '2' };
        },
      })
    );
    const document = createEmptyCreativeProjectDocument(PROJECT_ID);

    expect(await repository.list()).toEqual([project]);
    expect(await repository.load(PROJECT_ID)).toEqual({ project, document });
    expect(await repository.save(PROJECT_ID, '1', document)).toMatchObject({ revision: '2' });
    expect(calls).toEqual([{ projectId: PROJECT_ID, request: { expectedRevision: '1', document } }]);
  });

  test('maps stale saves and missing projects to distinct stable errors', async () => {
    const conflict = new BackendHttpError({
      method: 'PUT',
      path: `/api/creative-studio/projects/${PROJECT_ID}/document`,
      status: 409,
      body: { code: 'REVISION_CONFLICT', error: 'Project revision is stale' },
    });
    const repository = createCreativeProjectRepository(
      apiStub({ saveProject: async () => Promise.reject(conflict) })
    );

    try {
      await repository.save(PROJECT_ID, '1', createEmptyCreativeProjectDocument(PROJECT_ID));
      throw new Error('Expected repository failure');
    } catch (error) {
      expect(error instanceof CreativeProjectRepositoryError).toBe(true);
      expect((error as CreativeProjectRepositoryError).kind).toBe('revision-conflict');
      expect((error as CreativeProjectRepositoryError).status).toBe(409);
      expect((error as CreativeProjectRepositoryError).backendCode).toBe('REVISION_CONFLICT');
    }

    const missing = createCreativeProjectRepository(
      apiStub({
        getProject: async () =>
          Promise.reject(
            new BackendHttpError({
              method: 'GET',
              path: `/api/creative-studio/projects/${PROJECT_ID}`,
              status: 404,
              body: { code: 'NOT_FOUND', error: 'Project not found' },
            })
          ),
      })
    );
    try {
      await missing.load(PROJECT_ID);
      throw new Error('Expected missing project failure');
    } catch (error) {
      expect(error).toMatchObject({ kind: 'not-found', status: 404 });
    }
  });
});

describe('Creative Project hook cache helpers', () => {
  test('sorts newest first and upserts by authoritative project id', () => {
    const older = { ...project, projectId: '0198f8bb-8424-7b3d-8f17-bc6a1676f113', updatedAt: 100 };
    expect(sortCreativeProjectSummaries([older, project]).map((item) => item.projectId)).toEqual([
      PROJECT_ID,
      older.projectId,
    ]);
    expect(
      upsertCreativeProjectSummary([older, project], { ...project, title: 'Renamed', updatedAt: 300 })
    ).toEqual([{ ...project, title: 'Renamed', updatedAt: 300 }, older]);
  });
});
