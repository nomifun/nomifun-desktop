/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import {
  isBackendHttpError,
  isBackendRequestError,
} from '@/common/adapter/httpBridge';
import type { CreativeTemplateDefinitionV1 } from '../domain';
import {
  creativeTemplateApi,
  isCreativeTemplateContractError,
  type CreativeTemplateApi,
} from './templateApi';

export type CreativeTemplateRepositoryErrorKind =
  | 'contract'
  | 'not-found'
  | 'revision-conflict'
  | 'invalid-request'
  | 'permission-denied'
  | 'transport'
  | 'server'
  | 'unknown';

export class CreativeTemplateRepositoryError extends Error {
  readonly kind: CreativeTemplateRepositoryErrorKind;
  readonly status: number | null;
  readonly backendCode: string | null;

  constructor(params: {
    kind: CreativeTemplateRepositoryErrorKind;
    message: string;
    status?: number;
    backendCode?: string;
    cause?: unknown;
  }) {
    super(params.message, { cause: params.cause });
    this.name = 'CreativeTemplateRepositoryError';
    this.kind = params.kind;
    this.status = params.status ?? null;
    this.backendCode = params.backendCode ?? null;
  }
}

export function isCreativeTemplateRepositoryError(
  error: unknown
): error is CreativeTemplateRepositoryError {
  return (
    error instanceof CreativeTemplateRepositoryError ||
    (!!error &&
      typeof error === 'object' &&
      'name' in error &&
      (error as { name: unknown }).name === 'CreativeTemplateRepositoryError')
  );
}

export function toCreativeTemplateRepositoryError(
  error: unknown
): CreativeTemplateRepositoryError {
  if (isCreativeTemplateRepositoryError(error)) return error;
  if (isCreativeTemplateContractError(error)) {
    return new CreativeTemplateRepositoryError({
      kind: 'contract',
      message: error.message,
      cause: error,
    });
  }
  if (isBackendRequestError(error)) {
    return new CreativeTemplateRepositoryError({
      kind: 'transport',
      message: error.message,
      cause: error,
    });
  }
  if (isBackendHttpError(error)) {
    const kind: CreativeTemplateRepositoryErrorKind =
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
    return new CreativeTemplateRepositoryError({
      kind,
      message: error.backendMessage.trim() || error.message,
      status: error.status,
      backendCode: error.code || undefined,
      cause: error,
    });
  }
  return new CreativeTemplateRepositoryError({
    kind: 'unknown',
    message: error instanceof Error ? error.message : String(error),
    cause: error,
  });
}

export interface CreativeTemplateRepository {
  list(): Promise<CreativeTemplateDefinitionV1[]>;
  create(template: CreativeTemplateDefinitionV1): Promise<CreativeTemplateDefinitionV1>;
  load(templateId: string): Promise<CreativeTemplateDefinitionV1>;
  save(
    templateId: string,
    expectedRevision: number,
    template: CreativeTemplateDefinitionV1
  ): Promise<CreativeTemplateDefinitionV1>;
  remove(templateId: string): Promise<void>;
}

const guarded = async <Value>(operation: () => Promise<Value>): Promise<Value> => {
  try {
    return await operation();
  } catch (error) {
    throw toCreativeTemplateRepositoryError(error);
  }
};

export function createCreativeTemplateRepository(
  api: CreativeTemplateApi = creativeTemplateApi
): CreativeTemplateRepository {
  return {
    list: () => guarded(() => api.listTemplates()),
    create: (template) => guarded(() => api.createTemplate(template)),
    load: (templateId) => guarded(() => api.getTemplate(templateId)),
    save: (templateId, expectedRevision, template) =>
      guarded(() => {
        if (!Number.isSafeInteger(expectedRevision) || expectedRevision < 1) {
          throw new CreativeTemplateRepositoryError({
            kind: 'invalid-request',
            message: 'expectedRevision must be a positive safe integer',
          });
        }
        if (template.revision !== expectedRevision + 1) {
          throw new CreativeTemplateRepositoryError({
            kind: 'invalid-request',
            message: 'template revision must increment expectedRevision exactly once',
          });
        }
        return api.saveTemplate(templateId, {
          expectedRevision: String(expectedRevision),
          template,
        });
      }),
    remove: (templateId) => guarded(() => api.deleteTemplate(templateId)),
  };
}

export const creativeTemplateRepository = createCreativeTemplateRepository();
