/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import type { CreativeProjectDetail, CreativeProjectSummary } from '../domain';
import {
  creativeProjectRepository,
  type CreativeProjectRepository,
} from '../services/projectRepository';
import type {
  CreativeStudioProjectSummary,
  CreativeStudioProjectsService,
} from './types';

/**
 * Archive IO is intentionally separate from canonical project persistence.
 * A future HTTP/IPC implementation can be injected without teaching the page
 * about zip files, blobs, or document persistence.
 */
export interface CreativeStudioProjectArchivePort {
  importProjectArchive?(file: File): Promise<readonly CreativeProjectSummary[]>;
  exportProjectArchive?(projects: readonly CreativeProjectDetail[]): Promise<void>;
}

export type CreativeStudioProjectArchiveOperation = 'import' | 'export';

/** Raised instead of pretending an archive operation completed successfully. */
export class CreativeStudioProjectArchiveUnavailableError extends Error {
  readonly operation: CreativeStudioProjectArchiveOperation;

  constructor(operation: CreativeStudioProjectArchiveOperation) {
    super(`Creative Studio project archive ${operation} is not connected`);
    this.name = 'CreativeStudioProjectArchiveUnavailableError';
    this.operation = operation;
  }
}

export const mapCreativeProjectSummary = (
  project: CreativeProjectSummary
): CreativeStudioProjectSummary => ({
  id: project.projectId,
  title: project.title,
  createdAt: project.createdAt,
  updatedAt: project.updatedAt,
  nodeCount: project.nodeCount,
  connectionCount: project.connectionCount,
});

const assertRequestActive = (signal?: AbortSignal) => {
  if (!signal?.aborted) return;
  if (signal.reason instanceof Error) throw signal.reason;
  throw new DOMException('Creative Studio project request was aborted', 'AbortError');
};

/** Adapt the canonical repository and optional archive boundary to the project-center port. */
export const createCreativeStudioProjectsService = (
  repository: CreativeProjectRepository = creativeProjectRepository,
  archivePort?: CreativeStudioProjectArchivePort
): CreativeStudioProjectsService => ({
  archiveCapabilities: {
    canImport: typeof archivePort?.importProjectArchive === 'function',
    canExport: typeof archivePort?.exportProjectArchive === 'function',
  },

  listProjects: async (signal) => {
    assertRequestActive(signal);
    const projects = await repository.list();
    assertRequestActive(signal);
    return projects.map(mapCreativeProjectSummary);
  },

  createProject: async (title) =>
    mapCreativeProjectSummary(await repository.create({ title })),

  importProjectArchive: async (file) => {
    if (!archivePort?.importProjectArchive) {
      throw new CreativeStudioProjectArchiveUnavailableError('import');
    }
    const imported = await archivePort.importProjectArchive(file);
    return imported.map(mapCreativeProjectSummary);
  },

  renameProject: async (id, title) =>
    mapCreativeProjectSummary(await repository.rename(id, title)),

  deleteProjects: async (ids) => {
    await Promise.all(ids.map((id) => repository.remove(id)));
  },

  exportProjects: async (ids) => {
    if (!archivePort?.exportProjectArchive) {
      throw new CreativeStudioProjectArchiveUnavailableError('export');
    }
    const projects = await Promise.all(ids.map((id) => repository.load(id)));
    await archivePort.exportProjectArchive(projects);
  },
});

/** Production project persistence. Archive capabilities stay disabled until injected. */
export const creativeStudioProjectsService = createCreativeStudioProjectsService();
