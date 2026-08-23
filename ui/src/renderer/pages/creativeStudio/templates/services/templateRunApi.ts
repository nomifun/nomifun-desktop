/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { httpRequest } from '@/common/adapter/httpBridge';
import {
  TEMPLATE_LIMITS,
  TEMPLATE_RUN_LIMITS,
  cloneTemplateRunAggregate,
  isTemplateBusinessId,
  validateTemplateInputValues,
  validateTemplateRunAggregate,
  type CreativeTemplateInputValue,
  type CreativeTemplateRunAggregateV1,
} from '../domain';
import {
  CreativeTemplateContractError,
  type CreativeTemplateHttpRequest,
} from './templateApi';

export const CREATIVE_STUDIO_TEMPLATE_RUNS_ENDPOINT = '/api/creative-studio/template-runs';

export interface CreateCreativeTemplateRunRequest {
  templateRunId: string;
  templateId: string;
  templateRevision: number;
  inputs: CreativeTemplateInputValue[];
  referenceAssetIds: string[];
}

export interface SaveCreativeTemplateRunRequest {
  expectedRevision: string;
  run: CreativeTemplateRunAggregateV1;
}

export interface CreativeTemplateRunApi {
  listRuns(templateId?: string): Promise<CreativeTemplateRunAggregateV1[]>;
  createRun(request: CreateCreativeTemplateRunRequest): Promise<CreativeTemplateRunAggregateV1>;
  getRun(templateRunId: string): Promise<CreativeTemplateRunAggregateV1>;
  saveRun(templateRunId: string, request: SaveCreativeTemplateRunRequest): Promise<CreativeTemplateRunAggregateV1>;
}

const defaultRequest: CreativeTemplateHttpRequest = (method, path, body) =>
  httpRequest<unknown>(method, path, body);

function fail(
  code: CreativeTemplateContractError['code'],
  path: string,
  message: string
): never {
  throw new CreativeTemplateContractError({ code, path, message });
}

function assertId(value: unknown, path: string): asserts value is string {
  if (!isTemplateBusinessId(value)) fail('invalid-value', path, 'expected a canonical lowercase UUIDv7');
}

function exactObject(
  value: unknown,
  keys: readonly string[],
  path: string
): Record<string, unknown> {
  if (!value || typeof value !== 'object' || Array.isArray(value)) {
    fail('invalid-response', path, 'expected an object');
  }
  const record = value as Record<string, unknown>;
  const unknown = Object.keys(record).find((key) => !keys.includes(key));
  const missing = keys.find((key) => !Object.hasOwn(record, key));
  if (unknown || missing) {
    fail(
      'invalid-response',
      `${path}.${unknown ?? missing}`,
      unknown ? 'unexpected response field' : 'missing response field'
    );
  }
  return record;
}

function parseRun(value: unknown, path = '$.run'): CreativeTemplateRunAggregateV1 {
  const validation = validateTemplateRunAggregate(value, path);
  if (!validation.ok) {
    fail(validation.error.code, validation.error.path, validation.error.message);
  }
  return cloneTemplateRunAggregate(value as CreativeTemplateRunAggregateV1);
}

function parseRunResponse(value: unknown): CreativeTemplateRunAggregateV1 {
  const response = exactObject(value, ['run'], '$');
  return parseRun(response.run);
}

function parseRunListResponse(value: unknown): CreativeTemplateRunAggregateV1[] {
  const response = exactObject(value, ['runs'], '$');
  if (!Array.isArray(response.runs) || response.runs.length > TEMPLATE_LIMITS.runs) {
    fail('invalid-response', '$.runs', 'expected a bounded template run list');
  }
  const runs = response.runs.map((run, index) => parseRun(run, `$.runs[${index}]`));
  if (new Set(runs.map((run) => run.request.id)).size !== runs.length) {
    fail('duplicate-id', '$.runs', 'template run ids must be unique');
  }
  return runs;
}

function cloneInput(input: CreativeTemplateInputValue): CreativeTemplateInputValue {
  return input.type === 'image-series'
    ? { ...input, assetIds: [...input.assetIds] }
    : { ...input };
}

function inputEquals(left: CreativeTemplateInputValue, right: CreativeTemplateInputValue): boolean {
  if (left.variableId !== right.variableId || left.type !== right.type) return false;
  if (left.type === 'image-series' && right.type === 'image-series') {
    return left.assetIds.length === right.assetIds.length
      && left.assetIds.every((assetId, index) => assetId === right.assetIds[index]);
  }
  if (left.type === 'image' && right.type === 'image') return left.assetId === right.assetId;
  if (
    (left.type === 'text' || left.type === 'multiline-text' || left.type === 'choice')
    && (right.type === 'text' || right.type === 'multiline-text' || right.type === 'choice')
  ) return left.type === right.type && left.value === right.value;
  if (left.type === 'number' && right.type === 'number') return left.value === right.value;
  return left.type === 'boolean' && right.type === 'boolean' && left.value === right.value;
}

function assertCreateResponse(
  run: CreativeTemplateRunAggregateV1,
  request: CreateCreativeTemplateRunRequest
): void {
  const returned = run.request;
  const inputsMatch = returned.inputs.length === request.inputs.length
    && returned.inputs.every((input, index) => inputEquals(input, request.inputs[index]));
  const referencesMatch = returned.referenceAssetIds.length === request.referenceAssetIds.length
    && returned.referenceAssetIds.every(
      (assetId, index) => assetId === request.referenceAssetIds[index]
    );
  if (
    returned.id !== request.templateRunId
    || returned.idempotencyKey !== request.templateRunId
    || returned.templateId !== request.templateId
    || returned.templateRevision !== request.templateRevision
    || !inputsMatch
    || !referencesMatch
  ) {
    fail('identity-mismatch', '$.run.request', 'response does not match the create request');
  }
}

function assertRevision(value: string, path: string): number {
  if (!/^[1-9][0-9]*$/.test(value)) {
    fail('invalid-value', path, 'expected a positive canonical decimal revision');
  }
  const revision = Number(value);
  if (!Number.isSafeInteger(revision)) fail('invalid-value', path, 'revision exceeds the safe integer range');
  return revision;
}

function validateCreateRequest(
  request: CreateCreativeTemplateRunRequest
): CreateCreativeTemplateRunRequest {
  assertId(request.templateRunId, '$.request.templateRunId');
  assertId(request.templateId, '$.request.templateId');
  if (!Number.isSafeInteger(request.templateRevision) || request.templateRevision < 1) {
    fail('invalid-value', '$.request.templateRevision', 'expected a positive safe integer');
  }
  const inputValidation = validateTemplateInputValues(
    request.inputs,
    '$.request.inputs',
    TEMPLATE_LIMITS.variables
  );
  if (!inputValidation.ok) {
    fail(inputValidation.error.code, inputValidation.error.path, inputValidation.error.message);
  }
  if (!Array.isArray(request.referenceAssetIds)) {
    fail('invalid-value', '$.request.referenceAssetIds', 'expected an array');
  }
  if (request.referenceAssetIds.length > TEMPLATE_RUN_LIMITS.references) {
    fail('limit-exceeded', '$.request.referenceAssetIds', 'too many reference assets');
  }
  for (const [index, assetId] of request.referenceAssetIds.entries()) {
    assertId(assetId, `$.request.referenceAssetIds[${index}]`);
  }
  if (new Set(request.referenceAssetIds).size !== request.referenceAssetIds.length) {
    fail('duplicate-id', '$.request.referenceAssetIds', 'reference asset ids must be unique');
  }
  return {
    templateRunId: request.templateRunId,
    templateId: request.templateId,
    templateRevision: request.templateRevision,
    inputs: request.inputs.map(cloneInput),
    referenceAssetIds: [...request.referenceAssetIds],
  };
}

function templateRunPath(templateRunId: string): string {
  assertId(templateRunId, '$.templateRunId');
  return `${CREATIVE_STUDIO_TEMPLATE_RUNS_ENDPOINT}/${encodeURIComponent(templateRunId)}`;
}

/** Strict template-run client over NomiFun's shared auth and CSRF bridge. */
export function createCreativeTemplateRunApi(
  request: CreativeTemplateHttpRequest = defaultRequest
): CreativeTemplateRunApi {
  return {
    async listRuns(templateId) {
      if (templateId !== undefined) assertId(templateId, '$.templateId');
      const path = templateId === undefined
        ? CREATIVE_STUDIO_TEMPLATE_RUNS_ENDPOINT
        : `${CREATIVE_STUDIO_TEMPLATE_RUNS_ENDPOINT}?templateId=${encodeURIComponent(templateId)}`;
      const runs = parseRunListResponse(await request('GET', path));
      if (templateId !== undefined && runs.some((run) => run.request.templateId !== templateId)) {
        fail('identity-mismatch', '$.runs', 'response contains a run for another template');
      }
      return runs;
    },

    async createRun(input) {
      const checked = validateCreateRequest(input);
      const run = parseRunResponse(
        await request('POST', CREATIVE_STUDIO_TEMPLATE_RUNS_ENDPOINT, { request: checked })
      );
      assertCreateResponse(run, checked);
      return run;
    },

    async getRun(templateRunId) {
      const run = parseRunResponse(await request('GET', templateRunPath(templateRunId)));
      if (run.request.id !== templateRunId) {
        fail('identity-mismatch', '$.run.request.id', 'response run id does not match the route');
      }
      return run;
    },

    async saveRun(templateRunId, input) {
      const path = templateRunPath(templateRunId);
      const expectedRevision = assertRevision(input.expectedRevision, '$.expectedRevision');
      const validation = validateTemplateRunAggregate(input.run, '$.run');
      if (!validation.ok) {
        fail(validation.error.code, validation.error.path, validation.error.message);
      }
      if (input.run.request.id !== templateRunId) {
        fail('identity-mismatch', '$.run.request.id', 'run id does not match the route');
      }
      if (input.run.revision !== expectedRevision + 1) {
        fail('invalid-value', '$.run.revision', 'run revision must increment expectedRevision exactly once');
      }
      const run = parseRunResponse(
        await request('PUT', path, {
          expectedRevision: input.expectedRevision,
          run: cloneTemplateRunAggregate(input.run),
        })
      );
      if (run.request.id !== templateRunId || run.revision !== input.run.revision) {
        fail('identity-mismatch', '$.run', 'saved response does not match the submitted run revision');
      }
      return run;
    },
  };
}

export const creativeTemplateRunApi = createCreativeTemplateRunApi();
