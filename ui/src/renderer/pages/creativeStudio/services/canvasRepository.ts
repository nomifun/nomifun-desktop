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
  type CreateCreativeCanvasRequest,
  type CreativeCanvasDetail,
  type CreativeCanvasDocument,
  type CreativeCanvasSummary,
} from '../domain';
import {
  creativeStudioCanvasApi,
  type CreativeStudioCanvasApi,
} from './canvasApi';

export type CreativeCanvasRepositoryErrorKind =
  | 'contract'
  | 'not-found'
  | 'revision-conflict'
  | 'invalid-request'
  | 'permission-denied'
  | 'transport'
  | 'server'
  | 'unknown';

/** Product-facing Canvas error vocabulary independent of HTTP transport. */
export class CreativeCanvasRepositoryError extends Error {
  readonly kind: CreativeCanvasRepositoryErrorKind;
  readonly status: number | null;
  readonly backendCode: string | null;

  constructor(params: {
    kind: CreativeCanvasRepositoryErrorKind;
    message: string;
    status?: number;
    backendCode?: string;
    cause?: unknown;
  }) {
    super(params.message, { cause: params.cause });
    this.name = 'CreativeCanvasRepositoryError';
    this.kind = params.kind;
    this.status = params.status ?? null;
    this.backendCode = params.backendCode ?? null;
  }
}

export function isCreativeCanvasRepositoryError(
  error: unknown
): error is CreativeCanvasRepositoryError {
  return (
    error instanceof CreativeCanvasRepositoryError ||
    (!!error &&
      typeof error === 'object' &&
      'name' in error &&
      (error as { name: unknown }).name === 'CreativeCanvasRepositoryError')
  );
}

export function toCreativeCanvasRepositoryError(
  error: unknown
): CreativeCanvasRepositoryError {
  if (isCreativeCanvasRepositoryError(error)) return error;
  if (isCreativeStudioContractError(error)) {
    return new CreativeCanvasRepositoryError({
      kind: 'contract',
      message: error.message,
      cause: error,
    });
  }
  if (isBackendRequestError(error)) {
    return new CreativeCanvasRepositoryError({
      kind: 'transport',
      message: error.message,
      cause: error,
    });
  }
  if (isBackendHttpError(error)) {
    const kind: CreativeCanvasRepositoryErrorKind =
      error.status === 404
        ? 'not-found'
        : error.status === 409 && error.code === 'REVISION_CONFLICT'
          ? 'revision-conflict'
          : error.status === 400 || error.status === 409 || error.status === 422
            ? 'invalid-request'
            : error.status === 401 || error.status === 403
              ? 'permission-denied'
              : error.status >= 500
                ? 'server'
                : 'unknown';
    return new CreativeCanvasRepositoryError({
      kind,
      message: error.backendMessage.trim() || error.message,
      status: error.status,
      backendCode: error.code || undefined,
      cause: error,
    });
  }
  return new CreativeCanvasRepositoryError({
    kind: 'unknown',
    message: error instanceof Error ? error.message : String(error),
    cause: error,
  });
}

export interface CreativeCanvasRepository {
  list(): Promise<CreativeCanvasSummary[]>;
  create(request?: CreateCreativeCanvasRequest): Promise<CreativeCanvasSummary>;
  load(canvasId: string): Promise<CreativeCanvasDetail>;
  save(
    canvasId: string,
    expectedRevision: string,
    document: CreativeCanvasDocument
  ): Promise<CreativeCanvasSummary>;
  rename(canvasId: string, title: string): Promise<CreativeCanvasSummary>;
  remove(canvasId: string): Promise<void>;
}

const guarded = async <T>(operation: () => Promise<T>): Promise<T> => {
  try {
    return await operation();
  } catch (error) {
    throw toCreativeCanvasRepositoryError(error);
  }
};

export function createCreativeCanvasRepository(
  api: CreativeStudioCanvasApi = creativeStudioCanvasApi
): CreativeCanvasRepository {
  return {
    list: () => guarded(() => api.listCanvases()),
    create: (request = {}) => guarded(() => api.createCanvas(request)),
    load: (canvasId) => guarded(() => api.getCanvas(canvasId)),
    save: (canvasId, expectedRevision, document) =>
      guarded(() =>
        api.saveCanvas(canvasId, { expectedRevision, document })
      ),
    rename: (canvasId, title) =>
      guarded(() => api.renameCanvas(canvasId, { title })),
    remove: (canvasId) => guarded(() => api.deleteCanvas(canvasId)),
  };
}

export const creativeCanvasRepository = createCreativeCanvasRepository();
