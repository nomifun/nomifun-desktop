/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { httpRequest } from '@/common/adapter/httpBridge';
import {
  WORKFLOW_LIMITS,
  cloneWorkflowDefinition,
  isWorkflowBusinessId,
  validateWorkflowDefinition,
  type WorkflowDefinitionV1,
  type WorkflowValidationErrorCode,
} from '../domain';

export const CREATIVE_STUDIO_WORKFLOWS_ENDPOINT = '/api/creative-studio/workflows';

export type CreativeWorkflowHttpRequest = (
  method: string,
  path: string,
  body?: unknown
) => Promise<unknown>;

export interface SaveCreativeWorkflowRequest {
  expectedRevision: string;
  workflow: WorkflowDefinitionV1;
}

export interface CreativeWorkflowApi {
  listWorkflows(): Promise<WorkflowDefinitionV1[]>;
  createWorkflow(workflow: WorkflowDefinitionV1): Promise<WorkflowDefinitionV1>;
  getWorkflow(workflowId: string): Promise<WorkflowDefinitionV1>;
  saveWorkflow(
    workflowId: string,
    request: SaveCreativeWorkflowRequest
  ): Promise<WorkflowDefinitionV1>;
  deleteWorkflow(workflowId: string): Promise<void>;
}

export class CreativeWorkflowContractError extends Error {
  readonly code: WorkflowValidationErrorCode | 'invalid-response' | 'identity-mismatch';
  readonly path: string;

  constructor(params: {
    code: CreativeWorkflowContractError['code'];
    path: string;
    message: string;
  }) {
    super(`${params.path}: ${params.message}`);
    this.name = 'CreativeWorkflowContractError';
    this.code = params.code;
    this.path = params.path;
  }
}

export function isCreativeWorkflowContractError(
  error: unknown
): error is CreativeWorkflowContractError {
  return (
    error instanceof CreativeWorkflowContractError ||
    (!!error &&
      typeof error === 'object' &&
      'name' in error &&
      (error as { name: unknown }).name === 'CreativeWorkflowContractError')
  );
}

const defaultRequest: CreativeWorkflowHttpRequest = (method, path, body) =>
  httpRequest<unknown>(method, path, body);

function workflowPath(workflowId: string): string {
  assertWorkflowId(workflowId, '$.workflowId');
  return `${CREATIVE_STUDIO_WORKFLOWS_ENDPOINT}/${encodeURIComponent(workflowId)}`;
}

function assertWorkflowId(value: unknown, path: string): asserts value is string {
  if (!isWorkflowBusinessId(value)) {
    throw new CreativeWorkflowContractError({
      code: 'invalid-value',
      path,
      message: 'expected a canonical lowercase UUIDv7 workflow id',
    });
  }
}

function exactObject(
  value: unknown,
  keys: readonly string[],
  path: string
): Record<string, unknown> {
  if (!value || typeof value !== 'object' || Array.isArray(value)) {
    throw new CreativeWorkflowContractError({
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
    throw new CreativeWorkflowContractError({
      code: 'invalid-response',
      path: `${path}.${unknown ?? missing}`,
      message: unknown ? 'unexpected response field' : 'missing response field',
    });
  }
  return record;
}

export function parseWorkflowDefinition(
  value: unknown,
  path = '$.workflow'
): WorkflowDefinitionV1 {
  const validation = validateWorkflowDefinition(value, path);
  if (!validation.ok) {
    throw new CreativeWorkflowContractError({
      code: validation.error.code,
      path: validation.error.path,
      message: validation.error.message,
    });
  }
  return cloneWorkflowDefinition(value as WorkflowDefinitionV1);
}

function parseWorkflowResponse(value: unknown): WorkflowDefinitionV1 {
  const response = exactObject(value, ['workflow'], '$');
  return parseWorkflowDefinition(response.workflow);
}

function parseWorkflowListResponse(value: unknown): WorkflowDefinitionV1[] {
  const response = exactObject(value, ['workflows'], '$');
  if (
    !Array.isArray(response.workflows) ||
    response.workflows.length > WORKFLOW_LIMITS.workflows
  ) {
    throw new CreativeWorkflowContractError({
      code: 'invalid-response',
      path: '$.workflows',
      message: 'expected a bounded workflow list',
    });
  }
  const workflows = response.workflows.map((workflow, index) =>
    parseWorkflowDefinition(workflow, `$.workflows[${index}]`)
  );
  if (new Set(workflows.map((workflow) => workflow.id)).size !== workflows.length) {
    throw new CreativeWorkflowContractError({
      code: 'duplicate-id',
      path: '$.workflows',
      message: 'workflow ids must be unique',
    });
  }
  return workflows;
}

function assertResponseIdentity(workflow: WorkflowDefinitionV1, workflowId: string): void {
  if (workflow.id !== workflowId) {
    throw new CreativeWorkflowContractError({
      code: 'identity-mismatch',
      path: '$.workflow.id',
      message: 'response workflow id does not match the route id',
    });
  }
}

/** Build a strict workflow client over NomiFun's shared auth/CSRF HTTP bridge. */
export function createCreativeWorkflowApi(
  request: CreativeWorkflowHttpRequest = defaultRequest
): CreativeWorkflowApi {
  return {
    async listWorkflows() {
      return parseWorkflowListResponse(
        await request('GET', CREATIVE_STUDIO_WORKFLOWS_ENDPOINT)
      );
    },

    async createWorkflow(workflow) {
      const definition = parseWorkflowDefinition(workflow, '$.workflow');
      if (definition.revision !== 1) {
        throw new CreativeWorkflowContractError({
          code: 'invalid-value',
          path: '$.workflow.revision',
          message: 'a new workflow must start at revision 1',
        });
      }
      const created = parseWorkflowResponse(
        await request('POST', CREATIVE_STUDIO_WORKFLOWS_ENDPOINT, {
          workflow: definition,
        })
      );
      assertResponseIdentity(created, definition.id);
      return created;
    },

    async getWorkflow(workflowId) {
      const workflow = parseWorkflowResponse(await request('GET', workflowPath(workflowId)));
      assertResponseIdentity(workflow, workflowId);
      return workflow;
    },

    async saveWorkflow(workflowId, input) {
      const path = workflowPath(workflowId);
      if (!/^[1-9][0-9]*$/.test(input.expectedRevision)) {
        throw new CreativeWorkflowContractError({
          code: 'invalid-value',
          path: '$.expectedRevision',
          message: 'expected a positive canonical decimal revision',
        });
      }
      const expectedRevision = Number(input.expectedRevision);
      if (!Number.isSafeInteger(expectedRevision)) {
        throw new CreativeWorkflowContractError({
          code: 'invalid-value',
          path: '$.expectedRevision',
          message: 'expectedRevision exceeds the safe integer range',
        });
      }
      const workflow = parseWorkflowDefinition(input.workflow, '$.workflow');
      assertResponseIdentity(workflow, workflowId);
      if (workflow.revision !== expectedRevision + 1) {
        throw new CreativeWorkflowContractError({
          code: 'invalid-value',
          path: '$.workflow.revision',
          message: 'workflow revision must increment expectedRevision exactly once',
        });
      }
      const saved = parseWorkflowResponse(
        await request('PUT', path, {
          expectedRevision: input.expectedRevision,
          workflow,
        })
      );
      assertResponseIdentity(saved, workflowId);
      return saved;
    },

    async deleteWorkflow(workflowId) {
      await request('DELETE', workflowPath(workflowId));
    },
  };
}

export const creativeWorkflowApi = createCreativeWorkflowApi();
