/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import {
  isBackendHttpError,
  isBackendRequestError,
} from '@/common/adapter/httpBridge';
import type { WorkflowDefinitionV1 } from '../domain';
import {
  creativeWorkflowApi,
  isCreativeWorkflowContractError,
  type CreativeWorkflowApi,
} from './workflowApi';

export type CreativeWorkflowRepositoryErrorKind =
  | 'contract'
  | 'not-found'
  | 'revision-conflict'
  | 'invalid-request'
  | 'permission-denied'
  | 'transport'
  | 'server'
  | 'unknown';

export class CreativeWorkflowRepositoryError extends Error {
  readonly kind: CreativeWorkflowRepositoryErrorKind;
  readonly status: number | null;
  readonly backendCode: string | null;

  constructor(params: {
    kind: CreativeWorkflowRepositoryErrorKind;
    message: string;
    status?: number;
    backendCode?: string;
    cause?: unknown;
  }) {
    super(params.message, { cause: params.cause });
    this.name = 'CreativeWorkflowRepositoryError';
    this.kind = params.kind;
    this.status = params.status ?? null;
    this.backendCode = params.backendCode ?? null;
  }
}

export function isCreativeWorkflowRepositoryError(
  error: unknown
): error is CreativeWorkflowRepositoryError {
  return (
    error instanceof CreativeWorkflowRepositoryError ||
    (!!error &&
      typeof error === 'object' &&
      'name' in error &&
      (error as { name: unknown }).name === 'CreativeWorkflowRepositoryError')
  );
}

export function toCreativeWorkflowRepositoryError(
  error: unknown
): CreativeWorkflowRepositoryError {
  if (isCreativeWorkflowRepositoryError(error)) return error;
  if (isCreativeWorkflowContractError(error)) {
    return new CreativeWorkflowRepositoryError({
      kind: 'contract',
      message: error.message,
      cause: error,
    });
  }
  if (isBackendRequestError(error)) {
    return new CreativeWorkflowRepositoryError({
      kind: 'transport',
      message: error.message,
      cause: error,
    });
  }
  if (isBackendHttpError(error)) {
    const kind: CreativeWorkflowRepositoryErrorKind =
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
    return new CreativeWorkflowRepositoryError({
      kind,
      message: error.backendMessage.trim() || error.message,
      status: error.status,
      backendCode: error.code || undefined,
      cause: error,
    });
  }
  return new CreativeWorkflowRepositoryError({
    kind: 'unknown',
    message: error instanceof Error ? error.message : String(error),
    cause: error,
  });
}

export interface CreativeWorkflowRepository {
  list(): Promise<WorkflowDefinitionV1[]>;
  create(workflow: WorkflowDefinitionV1): Promise<WorkflowDefinitionV1>;
  load(workflowId: string): Promise<WorkflowDefinitionV1>;
  save(
    workflowId: string,
    expectedRevision: number,
    workflow: WorkflowDefinitionV1
  ): Promise<WorkflowDefinitionV1>;
  remove(workflowId: string): Promise<void>;
}

const guarded = async <Value>(operation: () => Promise<Value>): Promise<Value> => {
  try {
    return await operation();
  } catch (error) {
    throw toCreativeWorkflowRepositoryError(error);
  }
};

export function createCreativeWorkflowRepository(
  api: CreativeWorkflowApi = creativeWorkflowApi
): CreativeWorkflowRepository {
  return {
    list: () => guarded(() => api.listWorkflows()),
    create: (workflow) => guarded(() => api.createWorkflow(workflow)),
    load: (workflowId) => guarded(() => api.getWorkflow(workflowId)),
    save: (workflowId, expectedRevision, workflow) =>
      guarded(() => {
        if (!Number.isSafeInteger(expectedRevision) || expectedRevision < 1) {
          throw new CreativeWorkflowRepositoryError({
            kind: 'invalid-request',
            message: 'expectedRevision must be a positive safe integer',
          });
        }
        if (workflow.revision !== expectedRevision + 1) {
          throw new CreativeWorkflowRepositoryError({
            kind: 'invalid-request',
            message: 'workflow revision must increment expectedRevision exactly once',
          });
        }
        return api.saveWorkflow(workflowId, {
          expectedRevision: String(expectedRevision),
          workflow,
        });
      }),
    remove: (workflowId) => guarded(() => api.deleteWorkflow(workflowId)),
  };
}

export const creativeWorkflowRepository = createCreativeWorkflowRepository();
