/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { CANONICAL_UUID_V7 } from '@/common/types/ids';

import { cloneWorkflowDefinition } from './model';
import type {
  WorkflowDefinitionV1,
  WorkflowInputValue,
  WorkflowPromptDraft,
  WorkflowRunAggregateV1,
  WorkflowRunRequest,
  WorkflowRunStatus,
  WorkflowValidationError,
  WorkflowValidationResult,
} from './types';
import {
  cloneWorkflowOutput,
  validateWorkflowDefinition,
  validateWorkflowInputsForDefinition,
} from './validation';

type UnknownRecord = Record<string, unknown>;

export const WORKFLOW_RUN_KIND = 'nomifun.creative-studio.workflow-run' as const;

export const WORKFLOW_RUN_LIMITS = {
  references: 100,
  executableSteps: 128,
  taskReferences: 1_000,
  resultReferences: 6_000,
  title: 120,
  prompt: 200_000,
  reviewNote: 2_000,
  failureMessage: 2_000,
} as const;

const CONTROL = /[\u0000-\u0008\u000b\u000c\u000e-\u001f\u007f]/u;
const FAILURE_CODE = /^[a-z][a-z0-9._-]{0,79}$/;
const RUN_STATUSES = new Set<WorkflowRunStatus>([
  'requested',
  'awaiting-review',
  'queued',
  'running',
  'succeeded',
  'failed',
  'cancelled',
]);

function issue(
  code: WorkflowValidationError['code'],
  path: string,
  message: string
): WorkflowValidationError {
  return { code, path, message };
}

function exactObject(
  value: unknown,
  keys: readonly string[],
  path: string
): UnknownRecord | WorkflowValidationError {
  if (!value || typeof value !== 'object' || Array.isArray(value)) {
    return issue('invalid-value', path, 'expected an object');
  }
  const record = value as UnknownRecord;
  const unknown = Object.keys(record).find((key) => !keys.includes(key));
  if (unknown) return issue('unknown-field', `${path}.${unknown}`, 'field is not part of workflow run v1');
  const missing = keys.find((key) => !Object.hasOwn(record, key));
  return missing
    ? issue('invalid-value', `${path}.${missing}`, 'required field is missing')
    : record;
}

function isIssue(value: UnknownRecord | WorkflowValidationError): value is WorkflowValidationError {
  return 'code' in value && 'path' in value && 'message' in value;
}

function validateId(value: unknown, path: string): WorkflowValidationError | null {
  return typeof value === 'string' && CANONICAL_UUID_V7.test(value)
    ? null
    : issue('invalid-value', path, 'expected a canonical lowercase UUIDv7');
}

function validateTimestamp(value: unknown, path: string): WorkflowValidationError | null {
  return Number.isSafeInteger(value) && (value as number) >= 0
    ? null
    : issue('invalid-value', path, 'expected a non-negative millisecond timestamp');
}

function validateText(
  value: unknown,
  path: string,
  maximum: number,
  allowEmpty = false
): WorkflowValidationError | null {
  return typeof value === 'string'
    && value.length <= maximum
    && !CONTROL.test(value)
    && (allowEmpty || value.trim().length > 0)
    ? null
    : issue('invalid-value', path, 'expected bounded text');
}

function validateIdList(
  value: unknown,
  path: string,
  maximum: number
): WorkflowValidationError | null {
  if (!Array.isArray(value)) return issue('invalid-value', path, 'expected an array');
  if (value.length > maximum) return issue('limit-exceeded', path, `expected at most ${maximum} ids`);
  for (const [index, item] of value.entries()) {
    const error = validateId(item, `${path}[${index}]`);
    if (error) return error;
  }
  return new Set(value).size === value.length
    ? null
    : issue('duplicate-id', path, 'identifiers must be unique');
}

function sameValue(left: unknown, right: unknown): boolean {
  if (Object.is(left, right)) return true;
  if (Array.isArray(left) || Array.isArray(right)) {
    return Array.isArray(left)
      && Array.isArray(right)
      && left.length === right.length
      && left.every((value, index) => sameValue(value, right[index]));
  }
  if (!left || !right || typeof left !== 'object' || typeof right !== 'object') return false;
  const leftRecord = left as UnknownRecord;
  const rightRecord = right as UnknownRecord;
  const leftKeys = Object.keys(leftRecord).sort();
  const rightKeys = Object.keys(rightRecord).sort();
  return leftKeys.length === rightKeys.length
    && leftKeys.every(
      (key, index) => key === rightKeys[index] && sameValue(leftRecord[key], rightRecord[key])
    );
}

function validateRequest(
  value: unknown,
  workflow: WorkflowDefinitionV1,
  path: string
): WorkflowValidationError | null {
  const record = exactObject(value, [
    'id',
    'idempotencyKey',
    'workflowId',
    'workflowRevision',
    'requestedAt',
    'output',
    'inputs',
    'referenceAssetIds',
  ], path);
  if (isIssue(record)) return record;
  const common = validateId(record.id, `${path}.id`)
    ?? validateId(record.idempotencyKey, `${path}.idempotencyKey`)
    ?? validateId(record.workflowId, `${path}.workflowId`)
    ?? (!Number.isSafeInteger(record.workflowRevision) || (record.workflowRevision as number) < 1
      ? issue('invalid-value', `${path}.workflowRevision`, 'expected a positive safe integer')
      : null)
    ?? validateTimestamp(record.requestedAt, `${path}.requestedAt`);
  if (common) return common;
  if (record.idempotencyKey !== record.id) {
    return issue('invalid-value', `${path}.idempotencyKey`, 'idempotencyKey must equal the durable run id');
  }
  if (
    record.workflowId !== workflow.id
    || record.workflowRevision !== workflow.revision
    || !sameValue(record.output, workflow.output)
  ) {
    return issue('broken-reference', path, 'request does not match its pinned workflow definition');
  }
  const inputs = validateWorkflowInputsForDefinition(workflow, record.inputs, `${path}.inputs`);
  if (!inputs.ok) return inputs.error;
  return validateIdList(
    record.referenceAssetIds,
    `${path}.referenceAssetIds`,
    WORKFLOW_RUN_LIMITS.references
  );
}

function expectedDraftCount(workflow: WorkflowDefinitionV1): number {
  return workflow.output.kind === 'multi-image-series' ? workflow.output.targetCount : 0;
}

export function expectedWorkflowRunTaskCount(workflow: WorkflowDefinitionV1): number {
  const seriesCount = workflow.output.kind === 'multi-image-series'
    ? workflow.output.targetCount
    : 1;
  return workflow.steps.reduce((count, step) => {
    if (!step.enabled) return count;
    if (step.kind === 'draft-prompts') return count + 1;
    if (step.kind !== 'generate-images') return count;
    return count + (step.promptSource.kind === 'prompt-drafts' ? seriesCount : 1);
  }, 0);
}

export function expectedWorkflowRunResultCount(workflow: WorkflowDefinitionV1): number {
  const seriesCount = workflow.output.kind === 'multi-image-series'
    ? workflow.output.targetCount
    : 1;
  return workflow.steps.reduce((count, step) => {
    if (!step.enabled || step.kind !== 'generate-images') return count;
    const promptCount = step.promptSource.kind === 'prompt-drafts' ? seriesCount : 1;
    return count + promptCount * step.generation.imagesPerPrompt;
  }, 0);
}

function validateExecutablePlan(
  workflow: WorkflowDefinitionV1,
  path: string
): WorkflowValidationError | null {
  const planners = workflow.steps.filter((step) => step.enabled && step.kind === 'draft-prompts');
  if (workflow.output.kind === 'single-image' && planners.length > 0) {
    return issue('invalid-value', `${path}.steps`, 'single-image workflow cannot execute a prompt planner');
  }
  if (workflow.output.kind === 'multi-image-series' && planners.length !== 1) {
    return issue('invalid-value', `${path}.steps`, 'multi-image workflow requires exactly one prompt planner');
  }
  const executable = workflow.steps.filter(
    (step) => step.enabled && (step.kind === 'draft-prompts' || step.kind === 'generate-images')
  );
  if (executable.length === 0 || executable.length > WORKFLOW_RUN_LIMITS.executableSteps) {
    return issue('limit-exceeded', `${path}.steps`, 'executable step count must be between 1 and 128');
  }
  for (const [index, step] of workflow.steps.entries()) {
    if (!step.enabled) continue;
    if (step.kind === 'draft-prompts' && step.planning.model === null) {
      return issue('invalid-value', `${path}.steps[${index}].planning.model`, 'enabled planner requires a Chat model');
    }
    if (step.kind === 'generate-images' && step.generation.model === null) {
      return issue('invalid-value', `${path}.steps[${index}].generation.model`, 'enabled image step requires a model');
    }
  }
  return workflow.steps.some((step) => step.enabled && step.kind === 'generate-images')
    ? null
    : issue('invalid-value', `${path}.steps`, 'workflow run requires an enabled image step');
}

function validateDraft(
  value: unknown,
  aggregate: WorkflowRunAggregateV1,
  expectedCount: number,
  path: string
): WorkflowValidationError | null {
  const record = exactObject(value, [
    'id',
    'workflowId',
    'runRequestId',
    'seriesIndex',
    'title',
    'prompt',
    'status',
    'createdAt',
    'reviewedAt',
    'reviewNote',
  ], path);
  if (isIssue(record)) return record;
  const common = validateId(record.id, `${path}.id`)
    ?? validateId(record.workflowId, `${path}.workflowId`)
    ?? validateId(record.runRequestId, `${path}.runRequestId`)
    ?? (!Number.isSafeInteger(record.seriesIndex)
      || (record.seriesIndex as number) < 0
      || (record.seriesIndex as number) >= expectedCount
      ? issue('invalid-value', `${path}.seriesIndex`, 'series index is outside the pinned output plan')
      : null)
    ?? validateText(record.title, `${path}.title`, WORKFLOW_RUN_LIMITS.title)
    ?? validateText(record.prompt, `${path}.prompt`, WORKFLOW_RUN_LIMITS.prompt)
    ?? (!['pending-review', 'approved', 'rejected'].includes(record.status as string)
      ? issue('invalid-value', `${path}.status`, 'unsupported prompt draft status')
      : null)
    ?? validateTimestamp(record.createdAt, `${path}.createdAt`);
  if (common) return common;
  if (
    record.workflowId !== aggregate.request.workflowId
    || record.runRequestId !== aggregate.request.id
    || (record.createdAt as number) < aggregate.request.requestedAt
  ) {
    return issue('broken-reference', path, 'prompt draft ownership or timestamp is invalid');
  }
  if (record.reviewedAt !== null) {
    const error = validateTimestamp(record.reviewedAt, `${path}.reviewedAt`);
    if (error) return error;
  }
  if (record.reviewNote !== null) {
    const error = validateText(
      record.reviewNote,
      `${path}.reviewNote`,
      WORKFLOW_RUN_LIMITS.reviewNote,
      true
    );
    if (error) return error;
  }
  if (record.status === 'pending-review') {
    return record.reviewedAt === null && record.reviewNote === null
      ? null
      : issue('invalid-value', path, 'pending prompt draft cannot contain review data');
  }
  if (record.reviewedAt === null) {
    return issue('invalid-value', `${path}.reviewedAt`, 'reviewed prompt draft requires reviewedAt');
  }
  return (record.reviewedAt as number) >= (record.createdAt as number)
    ? null
    : issue('invalid-value', `${path}.reviewedAt`, 'review cannot predate draft creation');
}

function validateFailure(value: unknown, path: string): WorkflowValidationError | null {
  if (value === null) return null;
  const record = exactObject(value, ['code', 'message'], path);
  if (isIssue(record)) return record;
  if (typeof record.code !== 'string' || !FAILURE_CODE.test(record.code)) {
    return issue('invalid-value', `${path}.code`, 'expected a stable lowercase failure code');
  }
  return validateText(record.message, `${path}.message`, WORKFLOW_RUN_LIMITS.failureMessage);
}

function validateRecord(
  value: unknown,
  aggregate: WorkflowRunAggregateV1,
  expectedDrafts: number,
  path: string
): WorkflowValidationError | null {
  const record = exactObject(value, [
    'requestId',
    'workflowId',
    'status',
    'promptDraftIds',
    'taskIds',
    'resultAssetIds',
    'historyReferenceIds',
    'queuedAt',
    'startedAt',
    'completedAt',
    'failure',
  ], path);
  if (isIssue(record)) return record;
  const common = validateId(record.requestId, `${path}.requestId`)
    ?? validateId(record.workflowId, `${path}.workflowId`)
    ?? (!RUN_STATUSES.has(record.status as WorkflowRunStatus)
      ? issue('invalid-value', `${path}.status`, 'unsupported workflow run status')
      : null)
    ?? validateIdList(record.promptDraftIds, `${path}.promptDraftIds`, expectedDrafts)
    ?? validateIdList(record.taskIds, `${path}.taskIds`, WORKFLOW_RUN_LIMITS.taskReferences)
    ?? validateIdList(record.resultAssetIds, `${path}.resultAssetIds`, WORKFLOW_RUN_LIMITS.resultReferences)
    ?? validateIdList(record.historyReferenceIds, `${path}.historyReferenceIds`, WORKFLOW_RUN_LIMITS.resultReferences);
  if (common) return common;
  if (record.requestId !== aggregate.request.id || record.workflowId !== aggregate.request.workflowId) {
    return issue('broken-reference', path, 'run record ownership does not match its request');
  }
  const projectedDraftIds = aggregate.promptDrafts.map((draft) => draft.id);
  if (!sameValue(record.promptDraftIds, projectedDraftIds)) {
    return issue('broken-reference', `${path}.promptDraftIds`, 'prompt draft projection is inconsistent');
  }
  const expectedTasks = expectedWorkflowRunTaskCount(aggregate.workflowSnapshot);
  const expectedResults = expectedWorkflowRunResultCount(aggregate.workflowSnapshot);
  if ((record.taskIds as unknown[]).length > expectedTasks || (record.resultAssetIds as unknown[]).length > expectedResults) {
    return issue('limit-exceeded', path, 'task or result projection exceeds its pinned plan');
  }
  for (const key of ['queuedAt', 'startedAt', 'completedAt'] as const) {
    if (record[key] === null) continue;
    const error = validateTimestamp(record[key], `${path}.${key}`);
    if (error) return error;
    if ((record[key] as number) < aggregate.request.requestedAt) {
      return issue('invalid-value', `${path}.${key}`, 'run timestamp predates its request');
    }
  }
  const queuedAt = record.queuedAt as number | null;
  const startedAt = record.startedAt as number | null;
  const completedAt = record.completedAt as number | null;
  if (
    (queuedAt !== null && startedAt !== null && startedAt < queuedAt)
    || (startedAt !== null && completedAt !== null && completedAt < startedAt)
  ) {
    return issue('invalid-value', path, 'run timestamps are not monotonic');
  }
  const failure = validateFailure(record.failure, `${path}.failure`);
  if (failure) return failure;
  const taskCount = (record.taskIds as unknown[]).length;
  const resultCount = (record.resultAssetIds as unknown[]).length;
  const draftsComplete = aggregate.promptDrafts.length === expectedDrafts;
  const draftsApproved = aggregate.promptDrafts.every((draft) => draft.status === 'approved');
  const status = record.status as WorkflowRunStatus;
  if (status !== 'failed' && record.failure !== null) {
    return issue('invalid-value', `${path}.failure`, 'only failed runs may carry failure data');
  }
  switch (status) {
    case 'requested':
      return queuedAt === null
        && startedAt === null
        && completedAt === null
        && taskCount === 0
        && resultCount === 0
        && aggregate.promptDrafts.length === 0
        && record.failure === null
        ? null
        : issue('invalid-value', path, 'requested run contains execution data');
    case 'queued':
      return queuedAt !== null
        && startedAt === null
        && completedAt === null
        && taskCount === expectedTasks
        && record.failure === null
        ? null
        : issue('invalid-value', path, 'queued run projection is incomplete');
    case 'running':
      if (
        queuedAt === null
        || startedAt === null
        || completedAt !== null
        || taskCount !== expectedTasks
        || record.failure !== null
      ) return issue('invalid-value', path, 'running run projection is incomplete');
      return aggregate.promptDrafts.length > 0 && (!draftsComplete || !draftsApproved)
        ? issue('invalid-transition', path, 'image phase requires a complete approved prompt set')
        : null;
    case 'awaiting-review':
      return expectedDrafts > 0
        && queuedAt !== null
        && startedAt !== null
        && completedAt === null
        && taskCount === expectedTasks
        && draftsComplete
        && record.failure === null
        ? null
        : issue('invalid-value', path, 'review projection is incomplete');
    case 'succeeded':
      return queuedAt !== null
        && startedAt !== null
        && completedAt !== null
        && taskCount === expectedTasks
        && resultCount === expectedResults
        && (expectedDrafts === 0 || (draftsComplete && draftsApproved))
        && record.failure === null
        ? null
        : issue('invalid-value', path, 'successful run projection is incomplete');
    case 'failed':
      return completedAt !== null && record.failure !== null
        ? null
        : issue('invalid-value', path, 'failed run requires terminal failure data');
    case 'cancelled':
      return completedAt !== null && record.failure === null
        ? null
        : issue('invalid-value', path, 'cancelled run projection is invalid');
  }
}

export function validateWorkflowRunAggregate(
  value: unknown,
  path = '$'
): WorkflowValidationResult {
  const record = exactObject(value, [
    'kind',
    'version',
    'revision',
    'workflowSnapshot',
    'request',
    'promptDrafts',
    'record',
  ], path);
  if (isIssue(record)) return { ok: false, error: record };
  if (record.kind !== WORKFLOW_RUN_KIND) {
    return { ok: false, error: issue('invalid-envelope', `${path}.kind`, 'unexpected workflow run kind') };
  }
  if (record.version !== 1) {
    return { ok: false, error: issue('unsupported-version', `${path}.version`, 'only workflow run v1 is supported') };
  }
  if (!Number.isSafeInteger(record.revision) || (record.revision as number) < 1) {
    return { ok: false, error: issue('invalid-value', `${path}.revision`, 'revision must be a positive safe integer') };
  }
  const workflow = validateWorkflowDefinition(record.workflowSnapshot, `${path}.workflowSnapshot`);
  if (!workflow.ok) return workflow;
  const aggregate = value as WorkflowRunAggregateV1;
  const plan = validateExecutablePlan(aggregate.workflowSnapshot, `${path}.workflowSnapshot`);
  if (plan) return { ok: false, error: plan };
  const request = validateRequest(record.request, aggregate.workflowSnapshot, `${path}.request`);
  if (request) return { ok: false, error: request };
  const draftCount = expectedDraftCount(aggregate.workflowSnapshot);
  if (!Array.isArray(record.promptDrafts)) {
    return { ok: false, error: issue('invalid-value', `${path}.promptDrafts`, 'expected an array') };
  }
  if (record.promptDrafts.length > draftCount) {
    return { ok: false, error: issue('limit-exceeded', `${path}.promptDrafts`, 'too many prompt drafts') };
  }
  const ids = new Set<string>();
  const indexes = new Set<number>();
  for (const [index, draft] of record.promptDrafts.entries()) {
    const error = validateDraft(draft, aggregate, draftCount, `${path}.promptDrafts[${index}]`);
    if (error) return { ok: false, error };
    const typed = draft as WorkflowPromptDraft;
    if (ids.has(typed.id) || indexes.has(typed.seriesIndex)) {
      return {
        ok: false,
        error: issue('duplicate-id', `${path}.promptDrafts[${index}]`, 'draft ids and series indexes must be unique'),
      };
    }
    ids.add(typed.id);
    indexes.add(typed.seriesIndex);
  }
  const runRecord = validateRecord(record.record, aggregate, draftCount, `${path}.record`);
  return runRecord ? { ok: false, error: runRecord } : { ok: true };
}

function isPrefix(current: readonly string[], next: readonly string[]): boolean {
  return current.length <= next.length && current.every((value, index) => value === next[index]);
}

export function validateWorkflowRunTransition(
  current: WorkflowRunAggregateV1,
  next: WorkflowRunAggregateV1,
  path = '$'
): WorkflowValidationResult {
  const currentValidation = validateWorkflowRunAggregate(current, `${path}.current`);
  if (!currentValidation.ok) return currentValidation;
  const nextValidation = validateWorkflowRunAggregate(next, `${path}.next`);
  if (!nextValidation.ok) return nextValidation;
  if (['succeeded', 'failed', 'cancelled'].includes(current.record.status)) {
    return { ok: false, error: issue('invalid-transition', `${path}.current.record.status`, 'terminal workflow runs are immutable') };
  }
  if (next.revision !== current.revision + 1) {
    return { ok: false, error: issue('invalid-transition', `${path}.next.revision`, 'revision must increment exactly once') };
  }
  if (!sameValue(next.workflowSnapshot, current.workflowSnapshot) || !sameValue(next.request, current.request)) {
    return { ok: false, error: issue('invalid-transition', `${path}.next`, 'definition and request snapshots are immutable') };
  }
  const allowed = new Set([
    'requested>queued', 'requested>failed', 'requested>cancelled',
    'queued>queued', 'queued>running', 'queued>failed', 'queued>cancelled',
    'running>running', 'running>awaiting-review', 'running>succeeded', 'running>failed', 'running>cancelled',
    'awaiting-review>awaiting-review', 'awaiting-review>running', 'awaiting-review>failed', 'awaiting-review>cancelled',
  ]);
  if (!allowed.has(`${current.record.status}>${next.record.status}`)) {
    return { ok: false, error: issue('invalid-transition', `${path}.next.record.status`, 'workflow run status transition is not allowed') };
  }
  for (const key of ['taskIds', 'resultAssetIds', 'historyReferenceIds'] as const) {
    if (!isPrefix(current.record[key], next.record[key])) {
      return { ok: false, error: issue('invalid-transition', `${path}.next.record.${key}`, 'persisted ids may only be appended') };
    }
  }
  for (const key of ['queuedAt', 'startedAt', 'completedAt'] as const) {
    if (current.record[key] !== null && current.record[key] !== next.record[key]) {
      return { ok: false, error: issue('invalid-transition', `${path}.next.record.${key}`, 'timestamp is immutable once set') };
    }
  }
  if (current.promptDrafts.length > 0) {
    if (current.promptDrafts.length !== next.promptDrafts.length) {
      return { ok: false, error: issue('invalid-transition', `${path}.next.promptDrafts`, 'persisted drafts cannot be removed or replaced') };
    }
    for (const [index, before] of current.promptDrafts.entries()) {
      const after = next.promptDrafts[index];
      if (
        before.id !== after.id
        || before.workflowId !== after.workflowId
        || before.runRequestId !== after.runRequestId
        || before.seriesIndex !== after.seriesIndex
        || before.createdAt !== after.createdAt
      ) {
        return { ok: false, error: issue('invalid-transition', `${path}.next.promptDrafts[${index}]`, 'draft identity is immutable') };
      }
      if (current.record.status !== 'awaiting-review' && !sameValue(before, after)) {
        return { ok: false, error: issue('invalid-transition', `${path}.next.promptDrafts[${index}]`, 'drafts can only be edited during review') };
      }
    }
  }
  return { ok: true };
}

function cloneWorkflowInput(input: WorkflowInputValue): WorkflowInputValue {
  return input.type === 'image-series'
    ? { ...input, assetIds: [...input.assetIds] }
    : { ...input };
}

function cloneWorkflowRunRequest(request: WorkflowRunRequest): WorkflowRunRequest {
  return {
    ...request,
    output: cloneWorkflowOutput(request.output),
    inputs: request.inputs.map(cloneWorkflowInput),
    referenceAssetIds: [...request.referenceAssetIds],
  };
}

export function cloneWorkflowRunAggregate(
  aggregate: WorkflowRunAggregateV1
): WorkflowRunAggregateV1 {
  return {
    kind: WORKFLOW_RUN_KIND,
    version: 1,
    revision: aggregate.revision,
    workflowSnapshot: cloneWorkflowDefinition(aggregate.workflowSnapshot),
    request: cloneWorkflowRunRequest(aggregate.request),
    promptDrafts: aggregate.promptDrafts.map((draft) => ({ ...draft })),
    record: {
      ...aggregate.record,
      promptDraftIds: [...aggregate.record.promptDraftIds],
      taskIds: [...aggregate.record.taskIds],
      resultAssetIds: [...aggregate.record.resultAssetIds],
      historyReferenceIds: [...aggregate.record.historyReferenceIds],
      failure: aggregate.record.failure ? { ...aggregate.record.failure } : null,
    },
  };
}
