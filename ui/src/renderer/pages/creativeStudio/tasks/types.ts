/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import type { ModelTask } from '@/common/protocolBindings/ModelTask';

import type { CreativeJsonObject } from '../domain/schema';

/** Model-invoke tasks that the existing creation service can execute. */
export type CreativeCreationModelTask = Extract<
  ModelTask,
  'chat' | 'image_generation' | 'image_edit' | 'video_generation' | 'speech_synthesis'
>;

/** Exact `/api/creation/tasks` capability codes. */
export type CreativeTaskCapability =
  | 't2i'
  | 'i2i'
  | 'inpaint'
  | 't2v'
  | 'i2v'
  | 'v2v'
  | 'tts'
  | 'text';

export type CreativeTaskStatus = 'queued' | 'running' | 'succeeded' | 'failed' | 'canceled';

export type CreativeTaskInputRole =
  | 'reference'
  | 'mask'
  | 'first_frame'
  | 'last_frame'
  | 'video'
  | 'audio';

export interface CreativeTaskIdentity {
  projectId: string;
  nodeId: string;
  providerId: string;
  model: string;
  task: CreativeCreationModelTask;
  capability: CreativeTaskCapability;
}
export interface CreativeTaskInput {
  assetId: string;
  role: CreativeTaskInputRole;
}

export interface CreateCreativeTaskInput extends CreativeTaskIdentity {
  parameters: CreativeJsonObject;
  inputs: readonly CreativeTaskInput[];
}

export interface CreativeTaskError {
  kind: string;
  message: string;
  httpStatus: number | null;
}

/** Camel-case product contract. The backend's snake-case DTO never escapes the client. */
export interface CreativeTask extends CreativeTaskIdentity {
  taskId: string;
  parameters: CreativeJsonObject;
  status: CreativeTaskStatus;
  error: CreativeTaskError | null;
  resultAssetIds: string[];
  attempt: number;
  submittedAt: number;
  startedAt: number | null;
  finishedAt: number | null;
}

/** A task id is never fetched without the identity and ownership it must match. */
export interface CreativeTaskReference extends CreativeTaskIdentity {
  taskId: string;
}

export interface CreativeTaskOutput {
  taskId: string;
  projectId: string;
  nodeId: string;
  assetIds: string[];
}

export type CreativeTaskContractErrorCode =
  | 'invalid_request'
  | 'invalid_response'
  | 'task_capability_mismatch'
  | 'identity_mismatch'
  | 'ownership_mismatch';

/** Stable fail-closed error for request, response, and ownership contract violations. */
export class CreativeTaskContractError extends TypeError {
  readonly code: CreativeTaskContractErrorCode;
  readonly field: string | null;

  constructor(code: CreativeTaskContractErrorCode, message: string, field: string | null = null) {
    super(message);
    this.name = 'CreativeTaskContractError';
    this.code = code;
    this.field = field;
  }
}

const TASK_BY_CAPABILITY: Readonly<Record<CreativeTaskCapability, CreativeCreationModelTask>> = {
  t2i: 'image_generation',
  i2i: 'image_edit',
  inpaint: 'image_edit',
  t2v: 'video_generation',
  i2v: 'video_generation',
  v2v: 'video_generation',
  tts: 'speech_synthesis',
  text: 'chat',
};

const TASK_CAPABILITIES = new Set<CreativeTaskCapability>(
  Object.keys(TASK_BY_CAPABILITY) as CreativeTaskCapability[]
);

export function isCreativeTaskCapability(value: unknown): value is CreativeTaskCapability {
  return typeof value === 'string' && TASK_CAPABILITIES.has(value as CreativeTaskCapability);
}

/** Explicit backend capability mapping; this function never examines model or task names. */
export function modelTaskForCapability(
  capability: CreativeTaskCapability
): CreativeCreationModelTask {
  return TASK_BY_CAPABILITY[capability];
}

export function assertTaskCapabilityPair(
  task: CreativeCreationModelTask,
  capability: CreativeTaskCapability
): void {
  const expectedTask = modelTaskForCapability(capability);
  if (task !== expectedTask) {
    throw new CreativeTaskContractError(
      'task_capability_mismatch',
      `Capability ${capability} requires ModelTask ${expectedTask}; received ${task}`,
      'task'
    );
  }
}

export function isTerminalCreativeTaskStatus(status: CreativeTaskStatus): boolean {
  return status === 'succeeded' || status === 'failed' || status === 'canceled';
}

export function creativeTaskReference(task: CreativeTask): CreativeTaskReference {
  return {
    taskId: task.taskId,
    projectId: task.projectId,
    nodeId: task.nodeId,
    providerId: task.providerId,
    model: task.model,
    task: task.task,
    capability: task.capability,
  };
}
