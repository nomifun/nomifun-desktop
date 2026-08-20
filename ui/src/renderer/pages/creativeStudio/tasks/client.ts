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
} from './types';
import type {
  CreateCreativeTaskInput,
  CreativeCreationModelTask,
  CreativeTask,
  CreativeTaskCapability,
  CreativeTaskError,
  CreativeTaskIdentity,
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

function parseCreativeProjectId(
  value: unknown,
  field: string,
  code: 'invalid_request' | 'invalid_response' = 'invalid_response'
): string {
  if (typeof value !== 'string' || !CANONICAL_UUID_V7.test(value)) {
    throw new CreativeTaskContractError(
      code,
      `Invalid Creative Studio ${field}`,
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
  if (task.projectId !== expected.projectId) {
    throw new CreativeTaskContractError(
      'ownership_mismatch',
      `Creative task ${task.taskId} does not belong to project ${expected.projectId}`,
      'projectId'
    );
  }
  if (task.nodeId !== expected.nodeId) {
    throw new CreativeTaskContractError(
      'ownership_mismatch',
      `Creative task ${task.taskId} does not belong to node ${expected.nodeId}`,
      'nodeId'
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
  const capability = parseCapability(wire.capability);
  const projectId = wire.project_id === null
    ? null
    : parseCreativeProjectId(wire.project_id, 'project_id');
  const nodeId = wire.node_id === null ? null : String(parseWorkshopNodeId(wire.node_id));
  if (projectId === null || nodeId === null) {
    throw new CreativeTaskContractError(
      'ownership_mismatch',
      'Creative Studio tasks require both project_id and node_id ownership',
      projectId === null ? 'project_id' : 'node_id'
    );
  }
  if (wire.canvas_id !== null) {
    throw new CreativeTaskContractError(
      'ownership_mismatch',
      'Creative Studio tasks must not carry legacy canvas_id ownership',
      'canvas_id'
    );
  }
  const task: CreativeTask = {
    taskId: String(parseCreationTaskId(wire.creation_task_id)),
    projectId,
    nodeId,
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
    projectId: parseCreativeProjectId(identity.projectId, 'projectId', 'invalid_request'),
    nodeId: String(parseWorkshopNodeId(identity.nodeId)),
    providerId: String(parseProviderId(identity.providerId)),
    model: requireNonBlankString(identity.model, 'model'),
    task: identity.task,
    capability,
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
      project_id: identity.projectId,
      node_id: identity.nodeId,
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

/** Signal-aware HTTP adapter for the existing creation routes. */
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
    return this.request('POST', '/api/creation/tasks', body, signal, {
      'Idempotency-Key': idempotencyKey,
    });
  }

  get(taskId: string, signal?: AbortSignal): Promise<unknown> {
    return this.request(
      'GET',
      `/api/creation/tasks/${encodeURIComponent(taskId)}`,
      undefined,
      signal
    );
  }

  cancel(taskId: string, signal?: AbortSignal): Promise<unknown> {
    return this.request(
      'POST',
      `/api/creation/tasks/${encodeURIComponent(taskId)}/cancel`,
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
