/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { httpRequest } from '@/common/adapter/httpBridge';
import {
  TEMPLATE_LIMITS,
  cloneTemplateDefinition,
  isTemplateBusinessId,
  validateTemplateDefinition,
  type CreativeTemplateDefinitionV1,
  type CreativeTemplateValidationErrorCode,
} from '../domain';

export const CREATIVE_STUDIO_TEMPLATES_ENDPOINT = '/api/creative-studio/templates';

export type CreativeTemplateHttpRequest = (
  method: string,
  path: string,
  body?: unknown
) => Promise<unknown>;

export interface SaveCreativeTemplateRequest {
  expectedRevision: string;
  template: CreativeTemplateDefinitionV1;
}

export interface CreativeTemplateApi {
  listTemplates(): Promise<CreativeTemplateDefinitionV1[]>;
  createTemplate(template: CreativeTemplateDefinitionV1): Promise<CreativeTemplateDefinitionV1>;
  getTemplate(templateId: string): Promise<CreativeTemplateDefinitionV1>;
  saveTemplate(
    templateId: string,
    request: SaveCreativeTemplateRequest
  ): Promise<CreativeTemplateDefinitionV1>;
  deleteTemplate(templateId: string): Promise<void>;
}

export class CreativeTemplateContractError extends Error {
  readonly code: CreativeTemplateValidationErrorCode | 'invalid-response' | 'identity-mismatch';
  readonly path: string;

  constructor(params: {
    code: CreativeTemplateContractError['code'];
    path: string;
    message: string;
  }) {
    super(`${params.path}: ${params.message}`);
    this.name = 'CreativeTemplateContractError';
    this.code = params.code;
    this.path = params.path;
  }
}

export function isCreativeTemplateContractError(
  error: unknown
): error is CreativeTemplateContractError {
  return (
    error instanceof CreativeTemplateContractError ||
    (!!error &&
      typeof error === 'object' &&
      'name' in error &&
      (error as { name: unknown }).name === 'CreativeTemplateContractError')
  );
}

const defaultRequest: CreativeTemplateHttpRequest = (method, path, body) =>
  httpRequest<unknown>(method, path, body);

function templatePath(templateId: string): string {
  assertTemplateId(templateId, '$.templateId');
  return `${CREATIVE_STUDIO_TEMPLATES_ENDPOINT}/${encodeURIComponent(templateId)}`;
}

function assertTemplateId(value: unknown, path: string): asserts value is string {
  if (!isTemplateBusinessId(value)) {
    throw new CreativeTemplateContractError({
      code: 'invalid-value',
      path,
      message: 'expected a canonical lowercase UUIDv7 template id',
    });
  }
}

function exactObject(
  value: unknown,
  keys: readonly string[],
  path: string
): Record<string, unknown> {
  if (!value || typeof value !== 'object' || Array.isArray(value)) {
    throw new CreativeTemplateContractError({
      code: 'invalid-response',
      path,
      message: 'expected an object',
    });
  }
  const record = value as Record<string, unknown>;
  const actualKeys = Object.keys(record);
  const unknown = actualKeys.find((key) => !keys.includes(key));
  const missing = keys.find((key) => !Object.hasOwn(record, key));
  if (unknown || missing) {
    throw new CreativeTemplateContractError({
      code: 'invalid-response',
      path: `${path}.${unknown ?? missing}`,
      message: unknown ? 'unexpected response field' : 'missing response field',
    });
  }
  return record;
}

export function parseTemplateDefinition(
  value: unknown,
  path = '$.template'
): CreativeTemplateDefinitionV1 {
  const validation = validateTemplateDefinition(value, path);
  if (!validation.ok) {
    throw new CreativeTemplateContractError({
      code: validation.error.code,
      path: validation.error.path,
      message: validation.error.message,
    });
  }
  return cloneTemplateDefinition(value as CreativeTemplateDefinitionV1);
}

function parseTemplateResponse(value: unknown): CreativeTemplateDefinitionV1 {
  const response = exactObject(value, ['template'], '$');
  return parseTemplateDefinition(response.template);
}

function parseTemplateListResponse(value: unknown): CreativeTemplateDefinitionV1[] {
  const response = exactObject(value, ['templates'], '$');
  if (
    !Array.isArray(response.templates) ||
    response.templates.length > TEMPLATE_LIMITS.definitions
  ) {
    throw new CreativeTemplateContractError({
      code: 'invalid-response',
      path: '$.templates',
      message: 'expected a bounded template list',
    });
  }
  const templates = response.templates.map((template, index) =>
    parseTemplateDefinition(template, `$.templates[${index}]`)
  );
  if (new Set(templates.map((template) => template.id)).size !== templates.length) {
    throw new CreativeTemplateContractError({
      code: 'duplicate-id',
      path: '$.templates',
      message: 'template ids must be unique',
    });
  }
  return templates;
}

function assertResponseIdentity(template: CreativeTemplateDefinitionV1, templateId: string): void {
  if (template.id !== templateId) {
    throw new CreativeTemplateContractError({
      code: 'identity-mismatch',
      path: '$.template.id',
      message: 'response template id does not match the route id',
    });
  }
}

/** Build a strict template client over NomiFun's shared auth/CSRF HTTP bridge. */
export function createCreativeTemplateApi(
  request: CreativeTemplateHttpRequest = defaultRequest
): CreativeTemplateApi {
  return {
    async listTemplates() {
      return parseTemplateListResponse(
        await request('GET', CREATIVE_STUDIO_TEMPLATES_ENDPOINT)
      );
    },

    async createTemplate(template) {
      const definition = parseTemplateDefinition(template, '$.template');
      if (definition.revision !== 1) {
        throw new CreativeTemplateContractError({
          code: 'invalid-value',
          path: '$.template.revision',
          message: 'a new template must start at revision 1',
        });
      }
      const created = parseTemplateResponse(
        await request('POST', CREATIVE_STUDIO_TEMPLATES_ENDPOINT, {
          template: definition,
        })
      );
      assertResponseIdentity(created, definition.id);
      return created;
    },

    async getTemplate(templateId) {
      const template = parseTemplateResponse(await request('GET', templatePath(templateId)));
      assertResponseIdentity(template, templateId);
      return template;
    },

    async saveTemplate(templateId, input) {
      const path = templatePath(templateId);
      if (!/^[1-9][0-9]*$/.test(input.expectedRevision)) {
        throw new CreativeTemplateContractError({
          code: 'invalid-value',
          path: '$.expectedRevision',
          message: 'expected a positive canonical decimal revision',
        });
      }
      const expectedRevision = Number(input.expectedRevision);
      if (!Number.isSafeInteger(expectedRevision)) {
        throw new CreativeTemplateContractError({
          code: 'invalid-value',
          path: '$.expectedRevision',
          message: 'expectedRevision exceeds the safe integer range',
        });
      }
      const template = parseTemplateDefinition(input.template, '$.template');
      assertResponseIdentity(template, templateId);
      if (template.revision !== expectedRevision + 1) {
        throw new CreativeTemplateContractError({
          code: 'invalid-value',
          path: '$.template.revision',
          message: 'template revision must increment expectedRevision exactly once',
        });
      }
      const saved = parseTemplateResponse(
        await request('PUT', path, {
          expectedRevision: input.expectedRevision,
          template,
        })
      );
      assertResponseIdentity(saved, templateId);
      return saved;
    },

    async deleteTemplate(templateId) {
      await request('DELETE', templatePath(templateId));
    },
  };
}

export const creativeTemplateApi = createCreativeTemplateApi();
