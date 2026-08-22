/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import type { CreativeStudioProjectSummary, CreativeStudioProjectsService } from './types';

export const createCreativeStudioProjectFixture = (
  overrides: Partial<CreativeStudioProjectSummary> = {}
): CreativeStudioProjectSummary => ({
  id: 'project-brand-film',
  title: '品牌短片概念',
  createdAt: Date.parse('2026-08-17T03:10:00.000Z'),
  updatedAt: Date.parse('2026-08-20T01:35:00.000Z'),
  nodeCount: 18,
  connectionCount: 12,
  ...overrides,
});

export const CREATIVE_STUDIO_PROJECT_FIXTURES: readonly CreativeStudioProjectSummary[] = [
  createCreativeStudioProjectFixture(),
  createCreativeStudioProjectFixture({
    id: 'project-product-stills',
    title: '秋季产品静物',
    createdAt: Date.parse('2026-08-16T06:20:00.000Z'),
    updatedAt: Date.parse('2026-08-19T09:12:00.000Z'),
    nodeCount: 9,
    connectionCount: 6,
  }),
  createCreativeStudioProjectFixture({
    id: 'project-character-study',
    title: '角色风格探索',
    createdAt: Date.parse('2026-08-18T11:00:00.000Z'),
    updatedAt: Date.parse('2026-08-18T14:46:00.000Z'),
    nodeCount: 24,
    connectionCount: 19,
  }),
];

export interface MockCreativeStudioProjectsService extends CreativeStudioProjectsService {
  calls: {
    list: number;
    create: number;
    imports: File[];
    renames: Array<{ id: string; title: string }>;
    deletes: string[][];
    exports: string[][];
  };
  snapshot(): CreativeStudioProjectSummary[];
}

export const createMockCreativeStudioProjectsService = (
  seed: readonly CreativeStudioProjectSummary[] = CREATIVE_STUDIO_PROJECT_FIXTURES
): MockCreativeStudioProjectsService => {
  let projects = seed.map((project) => ({ ...project }));
  const calls: MockCreativeStudioProjectsService['calls'] = {
    list: 0,
    create: 0,
    imports: [],
    renames: [],
    deletes: [],
    exports: [],
  };

  return {
    archiveCapabilities: { canImport: true, canExport: true },
    calls,
    snapshot: () => projects.map((project) => ({ ...project })),
    listProjects: async () => {
      calls.list += 1;
      return projects.map((project) => ({ ...project }));
    },
    createProject: async (title) => {
      calls.create += 1;
      const created = createCreativeStudioProjectFixture({
        id: `project-created-${calls.create}`,
        title,
        createdAt: Date.parse('2026-08-20T02:00:00.000Z'),
        updatedAt: Date.parse('2026-08-20T02:00:00.000Z'),
        nodeCount: 0,
        connectionCount: 0,
      });
      projects = [created, ...projects];
      return { ...created };
    },
    importProjectArchive: async (file) => {
      calls.imports.push(file);
      const imported = createCreativeStudioProjectFixture({
        id: `project-imported-${calls.imports.length}`,
        title: file.name.replace(/\.zip$/i, '') || '导入画布',
        createdAt: Date.parse('2026-08-20T02:10:00.000Z'),
        updatedAt: Date.parse('2026-08-20T02:10:00.000Z'),
      });
      projects = [imported, ...projects];
      return [{ ...imported }];
    },
    renameProject: async (id, title) => {
      calls.renames.push({ id, title });
      const existing = projects.find((project) => project.id === id);
      if (!existing) throw new Error(`Project ${id} was not found`);
      const renamed = { ...existing, title, updatedAt: Date.parse('2026-08-20T02:20:00.000Z') };
      projects = projects.map((project) => (project.id === id ? renamed : project));
      return { ...renamed };
    },
    deleteProjects: async (ids) => {
      calls.deletes.push([...ids]);
      const removed = new Set(ids);
      projects = projects.filter((project) => !removed.has(project.id));
    },
    exportProjects: async (ids) => {
      calls.exports.push([...ids]);
    },
  };
};
