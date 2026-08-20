/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { httpRequest } from '@/common/adapter/httpBridge';
import {
  WORKFLOW_LIMITS,
  WORKFLOW_RUN_LIMITS,
  cloneWorkflowRunAggregate,
  isWorkflowBusinessId,
  validateWorkflowInputValues,
  validateWorkflowRunAggregate,
  type WorkflowInputValue,
  type WorkflowRunAggregateV1,
} from '../domain';
import {
  CreativeWorkflowContractError,
  type CreativeWorkflowHttpRequest,
} from './workflowApi';

export const CREATIVE_STUDIO_WORKFLOW_RUNS_ENDPOINT = '/api/creative-studio/workflow-runs';

export interface CreateCreativeWorkflowRunRequest {
  runId: string;
  workflowId: string;
  workflowRevision: number;
  inputs: WorkflowInputValue[];
  referenceAssetIds: string[];
}

export interface SaveCreativeWorkflowRunRequest {
  expectedRevision: string;
  run: WorkflowRunAggregateV1;
}

export interface CreativeWorkflowRunApi {
  listRuns(workflowId?: string): Promise<WorkflowRunAggregateV1[]>;
  createRun(request: CreateCreativeWorkflowRunRequest): Promise<WorkflowRunAggregateV1>;
  getRun(runId: string): Promise<WorkflowRunAggregateV1>;
  saveRun(runId: string, request: SaveCreativeWorkflowRunRequest): Promise<WorkflowRunAggregateV1>;
}

const defaultRequest: CreativeWorkflowHttpRequest = (method, path, body) =>
  httpRequest<unknown>(method, path, body);

function fail(
  code: CreativeWorkflowContractError['code'],
  path: string,
  message: string
): never {
  throw new CreativeWorkflowContractError({ code, path, message });
}

function assertId(value: unknown, path: string): asserts value is string {
  if (!isWorkflowBusinessId(value)) fail('invalid-value', path, 'expected a canonical lowercase UUIDv7');
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

function parseRun(value: unknown, path = '$.run'): WorkflowRunAggregateV1 {
  const validation = validateWorkflowRunAggregate(value, path);
  if (!validation.ok) {
    fail(validation.error.code, validation.error.path, validation.error.message);
  }
  return cloneWorkflowRunAggregate(value as WorkflowRunAggregateV1);
}

function parseRunResponse(value: unknown): WorkflowRunAggregateV1 {
  const response = exactObject(value, ['run'], '$');
  return parseRun(response.run);
}

function parseRunListResponse(value: unknown): WorkflowRunAggregateV1[] {
  const response = exactObject(value, ['runs'], '$');
  if (!Array.isArray(response.runs) || response.runs.length > WORKFLOW_LIMITS.runs) {
    fail('invalid-response', '$.runs', 'expected a bounded workflow run list');
  }
  const runs = response.runs.map((run, index) => parseRun(run, `$.runs[${index}]`));
  if (new Set(runs.map((run) => run.request.id)).size !== runs.length) {
    fail('duplicate-id', '$.runs', 'workflow run ids must be unique');
  }
  return runs;
}

function cloneInput(input: WorkflowInputValue): WorkflowInputValue {
  return input.type === 'image-series'
    ? { ...input, assetIds: [...input.assetIds] }
    : { ...input };
}

function inputEquals(left: WorkflowInputValue, right: WorkflowInputValue): boolean {
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
  run: WorkflowRunAggregateV1,
  request: CreateCreativeWorkflowRunRequest
): void {
  const returned = run.request;
  const inputsMatch = returned.inputs.length === request.inputs.length
    && returned.inputs.every((input, index) => inputEquals(input, request.inputs[index]));
  const referencesMatch = returned.referenceAssetIds.length === request.referenceAssetIds.length
    && returned.referenceAssetIds.every(
      (assetId, index) => assetId === request.referenceAssetIds[index]
    );
  if (
    returned.id !== request.runId
    || returned.idempotencyKey !== request.runId
    || returned.workflowId !== request.workflowId
    || returned.workflowRevision !== request.workflowRevision
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
  request: CreateCreativeWorkflowRunRequest
): CreateCreativeWorkflowRunRequest {
  assertId(request.runId, '$.request.runId');
  assertId(request.workflowId, '$.request.workflowId');
  if (!Number.isSafeInteger(request.workflowRevision) || request.workflowRevision < 1) {
    fail('invalid-value', '$.request.workflowRevision', 'expected a positive safe integer');
  }
  const inputValidation = validateWorkflowInputValues(
    request.inputs,
    '$.request.inputs',
    WORKFLOW_LIMITS.variables
  );
  if (!inputValidation.ok) {
    fail(inputValidation.error.code, inputValidation.error.path, inputValidation.error.message);
  }
  if (!Array.isArray(request.referenceAssetIds)) {
    fail('invalid-value', '$.request.referenceAssetIds', 'expected an array');
  }
  if (request.referenceAssetIds.length > WORKFLOW_RUN_LIMITS.references) {
    fail('limit-exceeded', '$.request.referenceAssetIds', 'too many reference assets');
  }
  for (const [index, assetId] of request.referenceAssetIds.entries()) {
    assertId(assetId, `$.request.referenceAssetIds[${index}]`);
  }
  if (new Set(request.referenceAssetIds).size !== request.referenceAssetIds.length) {
    fail('duplicate-id', '$.request.referenceAssetIds', 'reference asset ids must be unique');
  }
  return {
    runId: request.runId,
    workflowId: request.workflowId,
    workflowRevision: request.workflowRevision,
    inputs: request.inputs.map(cloneInput),
    referenceAssetIds: [...request.referenceAssetIds],
  };
}

function runPath(runId: string): string {
  assertId(runId, '$.runId');
  return `${CREATIVE_STUDIO_WORKFLOW_RUNS_ENDPOINT}/${encodeURIComponent(runId)}`;
}

/** Strict workflow-run client over NomiFun's shared auth and CSRF bridge. */
export function createCreativeWorkflowRunApi(
  request: CreativeWorkflowHttpRequest = defaultRequest
): CreativeWorkflowRunApi {
  return {
    async listRuns(workflowId) {
      if (workflowId !== undefined) assertId(workflowId, '$.workflowId');
      const path = workflowId === undefined
        ? CREATIVE_STUDIO_WORKFLOW_RUNS_ENDPOINT
        : `${CREATIVE_STUDIO_WORKFLOW_RUNS_ENDPOINT}?workflowId=${encodeURIComponent(workflowId)}`;
      const runs = parseRunListResponse(await request('GET', path));
      if (workflowId !== undefined && runs.some((run) => run.request.workflowId !== workflowId)) {
        fail('identity-mismatch', '$.runs', 'response contains a run for another workflow');
      }
      return runs;
    },

    async createRun(input) {
      const checked = validateCreateRequest(input);
      const run = parseRunResponse(
        await request('POST', CREATIVE_STUDIO_WORKFLOW_RUNS_ENDPOINT, { request: checked })
      );
      assertCreateResponse(run, checked);
      return run;
    },

    async getRun(runId) {
      const run = parseRunResponse(await request('GET', runPath(runId)));
      if (run.request.id !== runId) {
        fail('identity-mismatch', '$.run.request.id', 'response run id does not match the route');
      }
      return run;
    },

    async saveRun(runId, input) {
      const path = runPath(runId);
      const expectedRevision = assertRevision(input.expectedRevision, '$.expectedRevision');
      const validation = validateWorkflowRunAggregate(input.run, '$.run');
      if (!validation.ok) {
        fail(validation.error.code, validation.error.path, validation.error.message);
      }
      if (input.run.request.id !== runId) {
        fail('identity-mismatch', '$.run.request.id', 'run id does not match the route');
      }
      if (input.run.revision !== expectedRevision + 1) {
        fail('invalid-value', '$.run.revision', 'run revision must increment expectedRevision exactly once');
      }
      const run = parseRunResponse(
        await request('PUT', path, {
          expectedRevision: input.expectedRevision,
          run: cloneWorkflowRunAggregate(input.run),
        })
      );
      if (run.request.id !== runId || run.revision !== input.run.revision) {
        fail('identity-mismatch', '$.run', 'saved response does not match the submitted run revision');
      }
      return run;
    },
  };
}

export const creativeWorkflowRunApi = createCreativeWorkflowRunApi();
