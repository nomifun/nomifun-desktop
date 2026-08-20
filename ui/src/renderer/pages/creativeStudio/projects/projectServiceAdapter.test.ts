/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { describe, expect, test } from 'bun:test';

import {
  createEmptyCreativeProjectDocument,
  type CreativeProjectDetail,
  type CreativeProjectSummary,
} from '../domain';
import type { CreativeProjectRepository } from '../services/projectRepository';
import {
  createCreativeStudioProjectsService,
  CreativeStudioProjectArchiveUnavailableError,
  mapCreativeProjectSummary,
  type CreativeStudioProjectArchivePort,
} from './projectServiceAdapter';

const PROJECT: CreativeProjectSummary = {
  projectId: 'project-1',
  title: '无限画布 1',
  revision: '7',
  nodeCount: 4,
  connectionCount: 3,
  createdAt: 1_776_600_000_000,
  updatedAt: 1_776_600_030_000,
};

interface RepositoryFixture {
  repository: CreativeProjectRepository;
  calls: {
    creates: Array<{ title?: string }>;
    loads: string[];
    renames: Array<{ id: string; title: string }>;
    removes: string[];
  };
}

const createRepositoryFixture = (): RepositoryFixture => {
  const calls: RepositoryFixture['calls'] = {
    creates: [],
    loads: [],
    renames: [],
    removes: [],
  };

  return {
    calls,
    repository: {
      list: async () => [PROJECT],
      create: async (request = {}) => {
        calls.creates.push(request);
        return { ...PROJECT, title: request.title ?? PROJECT.title };
      },
      load: async (projectId) => {
        calls.loads.push(projectId);
        return {
          project: PROJECT,
          document: createEmptyCreativeProjectDocument(projectId),
        };
      },
      save: async () => PROJECT,
      rename: async (id, title) => {
        calls.renames.push({ id, title });
        return { ...PROJECT, projectId: id, title };
      },
      remove: async (id) => {
        calls.removes.push(id);
      },
    },
  };
};

const captureError = async (operation: () => Promise<unknown>): Promise<unknown> => {
  try {
    await operation();
    return null;
  } catch (error) {
    return error;
  }
};

describe('Creative Studio project service adapter', () => {
  test('maps the canonical summary without dropping connection counts or timestamps', async () => {
    const fixture = createRepositoryFixture();
    const service = createCreativeStudioProjectsService(fixture.repository);

    expect(mapCreativeProjectSummary(PROJECT)).toEqual({
      id: PROJECT.projectId,
      title: PROJECT.title,
      createdAt: PROJECT.createdAt,
      updatedAt: PROJECT.updatedAt,
      nodeCount: PROJECT.nodeCount,
      connectionCount: PROJECT.connectionCount,
    });
    expect(await service.listProjects()).toEqual([mapCreativeProjectSummary(PROJECT)]);

    const created = await service.createProject('无限画布 2');
    expect(created.title).toBe('无限画布 2');
    expect(fixture.calls.creates).toEqual([{ title: '无限画布 2' }]);

    const renamed = await service.renameProject(PROJECT.projectId, '品牌概念');
    expect(renamed.title).toBe('品牌概念');
    expect(fixture.calls.renames).toEqual([{ id: PROJECT.projectId, title: '品牌概念' }]);

    await service.deleteProjects([PROJECT.projectId, 'project-2']);
    expect(fixture.calls.removes).toEqual([PROJECT.projectId, 'project-2']);
  });

  test('exposes archive capabilities and fails explicitly when no archive port is connected', async () => {
    const fixture = createRepositoryFixture();
    const service = createCreativeStudioProjectsService(fixture.repository);
    const file = new File(['archive'], 'canvas.zip', { type: 'application/zip' });

    expect(service.archiveCapabilities).toEqual({ canImport: false, canExport: false });
    expect(
      (await captureError(() => service.importProjectArchive(file))) instanceof
        CreativeStudioProjectArchiveUnavailableError
    ).toBe(true);
    expect(
      (await captureError(() => service.exportProjects([PROJECT.projectId]))) instanceof
        CreativeStudioProjectArchiveUnavailableError
    ).toBe(true);
    expect(fixture.calls.loads).toEqual([]);
  });

  test('passes imported summaries and loaded project details through an injected archive port', async () => {
    const fixture = createRepositoryFixture();
    const imported = { ...PROJECT, projectId: 'project-imported', connectionCount: 9 };
    const exported: CreativeProjectDetail[][] = [];
    const archivePort: CreativeStudioProjectArchivePort = {
      importProjectArchive: async () => [imported],
      exportProjectArchive: async (projects) => {
        exported.push([...projects]);
      },
    };
    const service = createCreativeStudioProjectsService(fixture.repository, archivePort);

    expect(service.archiveCapabilities).toEqual({ canImport: true, canExport: true });
    expect(await service.importProjectArchive(new File([], 'canvas.zip'))).toEqual([
      mapCreativeProjectSummary(imported),
    ]);

    await service.exportProjects([PROJECT.projectId]);
    expect(fixture.calls.loads).toEqual([PROJECT.projectId]);
    expect(exported).toHaveLength(1);
    expect(exported[0][0].document.projectId).toBe(PROJECT.projectId);
    expect(exported[0][0].project.connectionCount).toBe(3);
  });
});
