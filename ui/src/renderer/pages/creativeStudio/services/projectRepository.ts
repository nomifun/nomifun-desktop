/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import {
  isBackendHttpError,
  isBackendRequestError,
} from '@/common/adapter/httpBridge';
import {
  isCreativeStudioContractError,
  type CreateCreativeProjectRequest,
  type CreativeProjectDetail,
  type CreativeProjectDocument,
  type CreativeProjectSummary,
} from '../domain';
import {
  creativeStudioProjectApi,
  type CreativeStudioProjectApi,
} from './projectApi';

export type CreativeProjectRepositoryErrorKind =
  | 'contract'
  | 'not-found'
  | 'revision-conflict'
  | 'invalid-request'
  | 'permission-denied'
  | 'transport'
  | 'server'
  | 'unknown';

/** Product-facing error vocabulary independent of the HTTP implementation. */
export class CreativeProjectRepositoryError extends Error {
  readonly kind: CreativeProjectRepositoryErrorKind;
  readonly status: number | null;
  readonly backendCode: string | null;

  constructor(params: {
    kind: CreativeProjectRepositoryErrorKind;
    message: string;
    status?: number;
    backendCode?: string;
    cause?: unknown;
  }) {
    super(params.message, { cause: params.cause });
    this.name = 'CreativeProjectRepositoryError';
    this.kind = params.kind;
    this.status = params.status ?? null;
    this.backendCode = params.backendCode ?? null;
  }
}

export function isCreativeProjectRepositoryError(
  error: unknown
): error is CreativeProjectRepositoryError {
  return (
    error instanceof CreativeProjectRepositoryError ||
    (!!error &&
      typeof error === 'object' &&
      'name' in error &&
      (error as { name: unknown }).name === 'CreativeProjectRepositoryError')
  );
}

export function toCreativeProjectRepositoryError(
  error: unknown
): CreativeProjectRepositoryError {
  if (isCreativeProjectRepositoryError(error)) return error;
  if (isCreativeStudioContractError(error)) {
    return new CreativeProjectRepositoryError({
      kind: 'contract',
      message: error.message,
      cause: error,
    });
  }
  if (isBackendRequestError(error)) {
    return new CreativeProjectRepositoryError({
      kind: 'transport',
      message: error.message,
      cause: error,
    });
  }
  if (isBackendHttpError(error)) {
    const kind: CreativeProjectRepositoryErrorKind =
      error.status === 404
        ? 'not-found'
        : error.status === 409
          ? 'revision-conflict'
          : error.status === 400 || error.status === 422
            ? 'invalid-request'
            : error.status === 401 || error.status === 403
              ? 'permission-denied'
              : error.status >= 500
                ? 'server'
                : 'unknown';
    return new CreativeProjectRepositoryError({
      kind,
      message: error.backendMessage.trim() || error.message,
      status: error.status,
      backendCode: error.code || undefined,
      cause: error,
    });
  }
  return new CreativeProjectRepositoryError({
    kind: 'unknown',
    message: error instanceof Error ? error.message : String(error),
    cause: error,
  });
}

export interface CreativeProjectRepository {
  list(): Promise<CreativeProjectSummary[]>;
  create(request?: CreateCreativeProjectRequest): Promise<CreativeProjectSummary>;
  load(projectId: string): Promise<CreativeProjectDetail>;
  save(
    projectId: string,
    expectedRevision: string,
    document: CreativeProjectDocument
  ): Promise<CreativeProjectSummary>;
  rename(projectId: string, title: string): Promise<CreativeProjectSummary>;
  remove(projectId: string): Promise<void>;
}

const guarded = async <T>(operation: () => Promise<T>): Promise<T> => {
  try {
    return await operation();
  } catch (error) {
    throw toCreativeProjectRepositoryError(error);
  }
};

export function createCreativeProjectRepository(
  api: CreativeStudioProjectApi = creativeStudioProjectApi
): CreativeProjectRepository {
  return {
    list: () => guarded(() => api.listProjects()),
    create: (request = {}) => guarded(() => api.createProject(request)),
    load: (projectId) => guarded(() => api.getProject(projectId)),
    save: (projectId, expectedRevision, document) =>
      guarded(() => api.saveProject(projectId, { expectedRevision, document })),
    rename: (projectId, title) => guarded(() => api.renameProject(projectId, { title })),
    remove: (projectId) => guarded(() => api.deleteProject(projectId)),
  };
}

export const creativeProjectRepository = createCreativeProjectRepository();
