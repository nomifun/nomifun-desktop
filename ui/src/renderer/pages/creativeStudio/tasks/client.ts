/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import {
  BackendHttpError,
  BackendRequestError,
  buildBackendAuthHeaders,
  getBaseUrl,
} from '@/common/adapter/httpBridge';
import {
  CANONICAL_UUID_V7,
  parseAssetId,
  parseCreationTaskId,
  parseProviderId,
  parseWorkshopNodeId,
} from '@/common/types/ids';

import type { CreativeJsonObject, CreativeJsonValue } from '../domain/schema';
import type { CreativeTaskPort } from './port';
import {
  CreativeTaskContractError,
  assertTaskCapabilityPair,
  isCreativeTaskCapability,
  isTerminalCreativeTaskStatus,
  modelTaskForCapability,
  sameCreativeTaskOwner,
} from './types';
import type {
  CreateCreativeTaskInput,
  CreativeCreationModelTask,
  CreativeTask,
  CreativeTaskCapability,
  CreativeTaskError,
  CreativeTaskIdentity,
  CreativeTaskOwner,
  CreativeTaskInputRole,
  CreativeTaskReference,
  CreativeTaskStatus,
} from './types';

const TASK_STATUSES = new Set<CreativeTaskStatus>([
  'queued',
  'running',
  'succeeded',
  'failed',
  'canceled',
]);
const INPUT_ROLES = new Set<CreativeTaskInputRole>([
  'reference',
  'mask',
  'first_frame',
  'last_frame',
  'video',
  'audio',
]);
const CREATION_MODEL_TASKS = new Set<CreativeCreationModelTask>([
  'chat',
  'image_generation',
  'image_edit',
  'video_generation',
  'speech_synthesis',
]);

export interface CreationTaskWireApi {
  create(body: unknown, idempotencyKey: string, signal?: AbortSignal): Promise<unknown>;
  get(taskId: string, signal?: AbortSignal): Promise<unknown>;
  cancel(taskId: string, signal?: AbortSignal): Promise<unknown>;
}

function parseCreativeOwnerId(
  value: unknown,
  field: string,
  code: 'invalid_request' | 'invalid_response' = 'invalid_response'
): string {
  if (typeof value !== 'string' || !CANONICAL_UUID_V7.test(value)) {
    throw new CreativeTaskContractError(
      code,
      `Invalid Creative Studio task owner ${field}`,
      field
    );
  }
  return value;
}

export interface HttpCreationTaskApiOptions {
  fetch?: typeof fetch;
  baseUrl?: () => string;
  authHeaders?: (method: string) => Record<string, string>;
}

function abortError(message = 'Creative task request was aborted'): Error {
  const error = new Error(message);
  error.name = 'AbortError';
  return error;
}

function throwIfAborted(signal?: AbortSignal): void {
  if (signal?.aborted) throw abortError();
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return value !== null && typeof value === 'object' && !Array.isArray(value);
}

function parseJsonValue(
  value: unknown,
  path: string,
  seen: WeakSet<object>,
  depth = 0
): CreativeJsonValue {
  if (depth > 64) {
    throw new CreativeTaskContractError(
      'invalid_response',
      `Invalid JSON value at ${path}: nesting is too deep`,
      path
    );
  }
  if (value === null || typeof value === 'string' || typeof value === 'boolean') return value;
  if (typeof value === 'number' && Number.isFinite(value)) return value;
  if (typeof value !== 'object') {
    throw new CreativeTaskContractError(
      'invalid_response',
      `Invalid JSON value at ${path}`,
      path
    );
  }
  if (seen.has(value)) {
    throw new CreativeTaskContractError(
      'invalid_response',
      `Invalid cyclic JSON value at ${path}`,
      path
    );
  }
  seen.add(value);
  try {
    if (Array.isArray(value)) {
      return value.map((entry, index) => parseJsonValue(entry, `${path}[${index}]`, seen, depth + 1));
    }
    return Object.fromEntries(
      Object.entries(value).map(([key, entry]) => [
        key,
        parseJsonValue(entry, `${path}.${key}`, seen, depth + 1),
      ])
    );
  } finally {
    seen.delete(value);
  }
}

function parseJsonObject(value: unknown, path: string): CreativeJsonObject {
  if (!isRecord(value)) {
    throw new CreativeTaskContractError(
      'invalid_response',
      `Expected a JSON object at ${path}`,
      path
    );
  }
  return parseJsonValue(value, path, new WeakSet()) as CreativeJsonObject;
}

function requireRecord(value: unknown, field: string): Record<string, unknown> {
  if (!isRecord(value)) {
    throw new CreativeTaskContractError(
      'invalid_response',
      `Invalid creative task ${field}`,
      field
    );
  }
  return value;
}

function requireExactKeys(
  record: Record<string, unknown>,
  keys: readonly string[],
  field: string
): void {
  const expected = new Set(keys);
  const unknown = Object.keys(record).find((key) => !expected.has(key));
  const missing = keys.find((key) => !(key in record));
  if (unknown || missing) {
    throw new CreativeTaskContractError(
      'invalid_response',
      `Invalid creative task ${field} fields`,
      unknown ? `${field}.${unknown}` : `${field}.${missing}`
    );
  }
}

function parseOwner(value: unknown): CreativeTaskOwner {
  const owner = requireRecord(value, 'owner');
  if (owner.kind === 'canvas_node') {
    requireExactKeys(owner, ['kind', 'project_id', 'node_id'], 'owner');
    return {
      kind: 'canvas_node',
      projectId: parseCreativeOwnerId(owner.project_id, 'owner.project_id'),
      nodeId: String(parseWorkshopNodeId(owner.node_id)),
    };
  }
  if (owner.kind === 'workflow_step') {
    requireExactKeys(
      owner,
      ['kind', 'workflow_id', 'workflow_run_id', 'workflow_step_id'],
      'owner'
    );
    return {
      kind: 'workflow_step',
      workflowId: parseCreativeOwnerId(owner.workflow_id, 'owner.workflow_id'),
      workflowRunId: parseCreativeOwnerId(owner.workflow_run_id, 'owner.workflow_run_id'),
      workflowStepId: parseCreativeOwnerId(owner.workflow_step_id, 'owner.workflow_step_id'),
    };
  }
  throw new CreativeTaskContractError(
    'ownership_mismatch',
    `Unknown Creative Studio task owner: ${String(owner.kind)}`,
    'owner.kind'
  );
}

function requireString(value: unknown, field: string): string {
  if (typeof value !== 'string') {
    throw new CreativeTaskContractError(
      'invalid_response',
      `Invalid creative task ${field}`,
      field
    );
  }
  return value;
}

function requireNonBlankString(value: unknown, field: string): string {
  const string = requireString(value, field);
  if (!string || string.trim() !== string) {
    throw new CreativeTaskContractError(
      'invalid_response',
      `Creative task ${field} must be a non-blank, already-normalized string`,
      field
    );
  }
  return string;
}

function requireInteger(value: unknown, field: string, minimum = 0): number {
  if (!Number.isSafeInteger(value) || (value as number) < minimum) {
    throw new CreativeTaskContractError(
      'invalid_response',
      `Invalid creative task ${field}`,
      field
    );
  }
  return value as number;
}

function nullableInteger(value: unknown, field: string): number | null {
  return value === null ? null : requireInteger(value, field);
}

function parseStatus(value: unknown): CreativeTaskStatus {
  if (typeof value !== 'string' || !TASK_STATUSES.has(value as CreativeTaskStatus)) {
    throw new CreativeTaskContractError(
      'invalid_response',
      `Unknown creative task status: ${String(value)}`,
      'status'
    );
  }
  return value as CreativeTaskStatus;
}

function parseCapability(value: unknown): CreativeTaskCapability {
  if (!isCreativeTaskCapability(value)) {
    throw new CreativeTaskContractError(
      'invalid_response',
      `Unknown creative task capability: ${String(value)}`,
      'capability'
    );
  }
  return value;
}

function parseError(value: unknown): CreativeTaskError | null {
  if (value === null) return null;
  const error = requireRecord(value, 'error');
  const httpStatus = error.http_status === undefined ? null : requireInteger(error.http_status, 'error.http_status', 100);
  if (httpStatus !== null && httpStatus > 599) {
    throw new CreativeTaskContractError(
      'invalid_response',
      'Invalid creative task error.http_status',
      'error.http_status'
    );
  }
  return {
    kind: requireNonBlankString(error.kind, 'error.kind'),
    message: requireNonBlankString(error.message, 'error.message'),
    httpStatus,
  };
}

function parseResultAssetIds(value: unknown): string[] {
  if (!Array.isArray(value)) {
    throw new CreativeTaskContractError(
      'invalid_response',
      'Invalid creative task result_asset_ids',
      'result_asset_ids'
    );
  }
  const ids = value.map((entry) => String(parseAssetId(entry)));
  if (new Set(ids).size !== ids.length) {
    throw new CreativeTaskContractError(
      'invalid_response',
      'Creative task result_asset_ids contains duplicates',
      'result_asset_ids'
    );
  }
  return ids;
}

function assertStatusContract(task: CreativeTask): void {
  const terminal = task.status === 'succeeded' || task.status === 'failed' || task.status === 'canceled';
  if (terminal !== (task.finishedAt !== null)) {
    throw new CreativeTaskContractError(
      'invalid_response',
      `Creative task ${task.status} has an inconsistent finished_at`,
      'finished_at'
    );
  }
  if (task.status === 'queued' && task.startedAt !== null) {
    throw new CreativeTaskContractError(
      'invalid_response',
      'Queued creative task must not have started_at',
      'started_at'
    );
  }
  if (task.status === 'running' && task.startedAt === null) {
    throw new CreativeTaskContractError(
      'invalid_response',
      'Running creative task must have started_at',
      'started_at'
    );
  }
  if (task.status === 'failed' && task.error === null) {
    throw new CreativeTaskContractError(
      'invalid_response',
      'Failed creative task must include an error',
      'error'
    );
  }
  if (task.status !== 'failed' && task.error !== null) {
    throw new CreativeTaskContractError(
      'invalid_response',
      `Creative task ${task.status} must not include an error`,
      'error'
    );
  }
  if (task.status === 'succeeded' && task.resultAssetIds.length === 0) {
    throw new CreativeTaskContractError(
      'invalid_response',
      'Succeeded creative task must include at least one result asset',
      'result_asset_ids'
    );
  }
  if (task.status !== 'succeeded' && task.resultAssetIds.length > 0) {
    throw new CreativeTaskContractError(
      'invalid_response',
      `Creative task ${task.status} must not expose uncommitted result assets`,
      'result_asset_ids'
    );
  }
}

function assertExpectedTask(task: CreativeTask, expected: CreativeTaskReference | CreativeTaskIdentity): void {
  if ('taskId' in expected && task.taskId !== expected.taskId) {
    throw new CreativeTaskContractError(
      'identity_mismatch',
      `Creative task id mismatch: expected ${expected.taskId}, received ${task.taskId}`,
      'taskId'
    );
  }
  if (!sameCreativeTaskOwner(task.owner, expected.owner)) {
    throw new CreativeTaskContractError(
      'ownership_mismatch',
      `Creative task ${task.taskId} does not belong to the expected owner`,
      'owner'
    );
  }
  for (const field of ['providerId', 'model', 'task', 'capability'] as const) {
    if (task[field] !== expected[field]) {
      throw new CreativeTaskContractError(
        'identity_mismatch',
        `Creative task ${field} mismatch: expected ${expected[field]}, received ${task[field]}`,
        field
      );
    }
  }
}

/** Strict snake_case DTO -> camelCase product mapping. */
export function mapCreationTaskWire(
  value: unknown,
  expected?: CreativeTaskReference | CreativeTaskIdentity
): CreativeTask {
  const wire = requireRecord(value, 'response');
  requireExactKeys(
    wire,
    [
      'creation_task_id',
      'owner',
      'provider_id',
      'model',
      'capability',
      'params',
      'status',
      'error',
      'result_asset_ids',
      'attempt',
      'submitted_at',
      'started_at',
      'finished_at',
    ],
    'response'
  );
  const capability = parseCapability(wire.capability);
  const task: CreativeTask = {
    taskId: String(parseCreationTaskId(wire.creation_task_id)),
    owner: parseOwner(wire.owner),
    providerId: String(parseProviderId(wire.provider_id)),
    model: requireNonBlankString(wire.model, 'model'),
    task: modelTaskForCapability(capability),
    capability,
    parameters: parseJsonObject(wire.params, 'params'),
    status: parseStatus(wire.status),
    error: parseError(wire.error),
    resultAssetIds: parseResultAssetIds(wire.result_asset_ids),
    attempt: requireInteger(wire.attempt, 'attempt'),
    submittedAt: requireInteger(wire.submitted_at, 'submitted_at'),
    startedAt: nullableInteger(wire.started_at, 'started_at'),
    finishedAt: nullableInteger(wire.finished_at, 'finished_at'),
  };
  assertStatusContract(task);
  if (expected) assertExpectedTask(task, expected);
  return task;
}

function normalizeOwner(owner: CreativeTaskOwner): CreativeTaskOwner {
  if (owner.kind === 'canvas_node') {
    return {
      kind: 'canvas_node',
      projectId: parseCreativeOwnerId(owner.projectId, 'owner.projectId', 'invalid_request'),
      nodeId: String(parseWorkshopNodeId(owner.nodeId)),
    };
  }
  return {
    kind: 'workflow_step',
    workflowId: parseCreativeOwnerId(owner.workflowId, 'owner.workflowId', 'invalid_request'),
    workflowRunId: parseCreativeOwnerId(
      owner.workflowRunId,
      'owner.workflowRunId',
      'invalid_request'
    ),
    workflowStepId: parseCreativeOwnerId(
      owner.workflowStepId,
      'owner.workflowStepId',
      'invalid_request'
    ),
  };
}

function normalizeIdentity(identity: CreativeTaskIdentity): CreativeTaskIdentity {
  const capability = parseCapability(identity.capability);
  if (!CREATION_MODEL_TASKS.has(identity.task)) {
    throw new CreativeTaskContractError(
      'invalid_request',
      `Unsupported creation ModelTask: ${String(identity.task)}`,
      'task'
    );
  }
  assertTaskCapabilityPair(identity.task, capability);
  return {
    owner: normalizeOwner(identity.owner),
    providerId: String(parseProviderId(identity.providerId)),
    model: requireNonBlankString(identity.model, 'model'),
    task: identity.task,
    capability,
  };
}

function ownerWire(owner: CreativeTaskOwner): Record<string, string> {
  if (owner.kind === 'canvas_node') {
    return {
      kind: owner.kind,
      project_id: owner.projectId,
      node_id: owner.nodeId,
    };
  }
  return {
    kind: owner.kind,
    workflow_id: owner.workflowId,
    workflow_run_id: owner.workflowRunId,
    workflow_step_id: owner.workflowStepId,
  };
}

function normalizeReference(reference: CreativeTaskReference): CreativeTaskReference {
  return {
    taskId: String(parseCreationTaskId(reference.taskId)),
    ...normalizeIdentity(reference),
  };
}

function toCreateTaskBody(input: CreateCreativeTaskInput): {
  identity: CreativeTaskIdentity;
  idempotencyKey: string;
  body: Record<string, unknown>;
} {
  const identity = normalizeIdentity(input);
  const parameters = parseJsonObject(input.parameters, 'parameters');
  const inputs = input.inputs.map((entry, index) => {
    if (!INPUT_ROLES.has(entry.role)) {
      throw new CreativeTaskContractError(
        'invalid_request',
        `Unsupported creative task input role: ${String(entry.role)}`,
        `inputs[${index}].role`
      );
    }
    return {
      asset_id: String(parseAssetId(entry.assetId)),
      role: entry.role,
    };
  });
  return {
    identity,
    idempotencyKey: String(parseCreationTaskId(input.idempotencyKey)),
    body: {
      owner: ownerWire(identity.owner),
      provider_id: identity.providerId,
      model: identity.model,
      capability: identity.capability,
      params: parameters,
      inputs,
    },
  };
}

function csrfRejected(value: unknown): boolean {
  return (
    isRecord(value) &&
    typeof value.error === 'string' &&
    value.error.includes('CSRF token validation failed')
  );
}

async function responseBody(response: Response): Promise<unknown> {
  const raw = await response.text();
  if (!raw) return null;
  try {
    return JSON.parse(raw) as unknown;
  } catch {
    return raw;
  }
}

/** Signal-aware HTTP adapter for the canonical Creative Studio task routes. */
export class HttpCreationTaskApi implements CreationTaskWireApi {
  private readonly fetchImpl: typeof fetch;
  private readonly baseUrl: () => string;
  private readonly authHeaders: (method: string) => Record<string, string>;

  constructor(options: HttpCreationTaskApiOptions = {}) {
    this.fetchImpl = options.fetch ?? globalThis.fetch.bind(globalThis);
    this.baseUrl = options.baseUrl ?? getBaseUrl;
    this.authHeaders = options.authHeaders ?? buildBackendAuthHeaders;
  }

  private async request(
    method: string,
    path: string,
    body: unknown,
    signal?: AbortSignal,
    requestHeaders: Record<string, string> = {}
  ): Promise<unknown> {
    throwIfAborted(signal);
    for (let attempt = 0; attempt < 2; attempt += 1) {
      let response: Response;
      try {
        response = await this.fetchImpl(`${this.baseUrl()}${path}`, {
          method,
          headers: {
            ...(body === undefined ? {} : { 'Content-Type': 'application/json' }),
            ...this.authHeaders(method),
            ...requestHeaders,
          },
          body: body === undefined ? undefined : JSON.stringify(body),
          signal,
          cache: method === 'GET' ? 'no-store' : undefined,
        });
      } catch (error) {
        if (signal?.aborted) throw abortError();
        throw new BackendRequestError(
          'network',
          `Backend ${method} ${path} failed: backend unreachable (${error instanceof Error ? error.message : String(error)})`
        );
      }
      const parsed = await responseBody(response);
      if (!response.ok) {
        if (response.status === 403 && attempt === 0 && csrfRejected(parsed)) continue;
        throw new BackendHttpError({ method, path, status: response.status, body: parsed });
      }
      if (!isRecord(parsed) || !('data' in parsed)) {
        throw new CreativeTaskContractError(
          'invalid_response',
          `Backend ${method} ${path} returned an invalid response envelope`,
          'response'
        );
      }
      return parsed.data;
    }
    throw new CreativeTaskContractError(
      'invalid_response',
      `Backend ${method} ${path} exhausted its CSRF retry`,
      'response'
    );
  }

  create(body: unknown, idempotencyKey: string, signal?: AbortSignal): Promise<unknown> {
    return this.request('POST', '/api/creative-studio/tasks', body, signal, {
      'Idempotency-Key': idempotencyKey,
    });
  }

  get(taskId: string, signal?: AbortSignal): Promise<unknown> {
    return this.request(
      'GET',
      `/api/creative-studio/tasks/${encodeURIComponent(taskId)}`,
      undefined,
      signal
    );
  }

  cancel(taskId: string, signal?: AbortSignal): Promise<unknown> {
    return this.request(
      'POST',
      `/api/creative-studio/tasks/${encodeURIComponent(taskId)}/cancel`,
      undefined,
      signal
    );
  }
}

export class CreativeTaskClient implements CreativeTaskPort {
  constructor(private readonly api: CreationTaskWireApi = new HttpCreationTaskApi()) {}

  async create(input: CreateCreativeTaskInput, signal?: AbortSignal): Promise<CreativeTask> {
    throwIfAborted(signal);
    const { identity, idempotencyKey, body } = toCreateTaskBody(input);
    return mapCreationTaskWire(
      await this.api.create(body, idempotencyKey, signal),
      { taskId: idempotencyKey, ...identity }
    );
  }

  async get(reference: CreativeTaskReference, signal?: AbortSignal): Promise<CreativeTask> {
    throwIfAborted(signal);
    const normalized = normalizeReference(reference);
    return mapCreationTaskWire(await this.api.get(normalized.taskId, signal), normalized);
  }

  async cancel(reference: CreativeTaskReference, signal?: AbortSignal): Promise<CreativeTask> {
    throwIfAborted(signal);
    const normalized = normalizeReference(reference);
    const task = mapCreationTaskWire(await this.api.cancel(normalized.taskId, signal), normalized);
    if (!isTerminalCreativeTaskStatus(task.status)) {
      throw new CreativeTaskContractError(
        'invalid_response',
        `Cancel did not return an authoritative terminal task: ${task.status}`,
        'status'
      );
    }
    return task;
  }
}

export const creativeTaskClient = new CreativeTaskClient();
