/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { CANONICAL_UUID_V7 } from '@/common/types/ids';

import { validateWorkflowGraph } from './graph';
import type {
  WorkflowDefinitionV1,
  WorkflowInputValue,
  WorkflowOutputPlan,
  WorkflowRunRequest,
  WorkflowValidationError,
  WorkflowValidationResult,
  WorkflowVariable,
  WorkflowWorkspaceDocumentV1,
} from './types';

type UnknownRecord = Record<string, unknown>;

export const WORKFLOW_LIMITS = {
  jsonBytes: 8 * 1024 * 1024,
  workflows: 500,
  variables: 100,
  templates: 50,
  templateSegments: 500,
  steps: 200,
  drafts: 10_000,
  runs: 10_000,
  text: 20_000,
  prompt: 200_000,
  tags: 30,
  seriesItems: 20,
  taskReferences: 1_000,
} as const;

const KEY = /^[a-z][a-z0-9_]{0,63}$/;
const CODE = /^[a-z][a-z0-9._-]{0,79}$/;
const CONTROL = /[\u0000-\u0008\u000b\u000c\u000e-\u001f\u007f]/u;

function issue(
  code: WorkflowValidationError['code'],
  path: string,
  message: string
): WorkflowValidationError {
  return { code, path, message };
}

function asRecord(value: unknown, path: string, keys: readonly string[]): UnknownRecord | WorkflowValidationError {
  if (!value || typeof value !== 'object' || Array.isArray(value)) {
    return issue('invalid-value', path, 'expected an object');
  }
  const record = value as UnknownRecord;
  const actual = Object.keys(record);
  const unknown = actual.find((key) => !keys.includes(key));
  if (unknown) return issue('unknown-field', `${path}.${unknown}`, 'field is not part of workflow v1');
  const missing = keys.find((key) => !Object.prototype.hasOwnProperty.call(record, key));
  if (missing) return issue('invalid-value', `${path}.${missing}`, 'required field is missing');
  return record;
}

function isIssue(value: UnknownRecord | WorkflowValidationError): value is WorkflowValidationError {
  return 'code' in value && 'path' in value && 'message' in value;
}

function text(
  value: unknown,
  path: string,
  maximum: number = WORKFLOW_LIMITS.text,
  allowEmpty = false
) {
  if (
    typeof value !== 'string' ||
    value.length > maximum ||
    CONTROL.test(value) ||
    (!allowEmpty && value.trim().length === 0)
  ) {
    return issue('invalid-value', path, 'expected bounded display text');
  }
  return null;
}

function id(value: unknown, path: string) {
  return typeof value === 'string' && CANONICAL_UUID_V7.test(value)
    ? null
    : issue('invalid-value', path, 'expected a canonical lowercase UUIDv7');
}

function timestamp(value: unknown, path: string) {
  return Number.isSafeInteger(value) && (value as number) >= 0
    ? null
    : issue('invalid-value', path, 'expected a non-negative millisecond timestamp');
}

function finite(value: unknown, path: string) {
  return typeof value === 'number' && Number.isFinite(value)
    ? null
    : issue('invalid-value', path, 'expected a finite number');
}

function uniqueIds(values: readonly string[], path: string) {
  return new Set(values).size === values.length
    ? null
    : issue('duplicate-id', path, 'identifiers must be unique');
}

function stringList(value: unknown, path: string, maximum: number, identifiers = false) {
  if (!Array.isArray(value) || value.length > maximum) {
    return issue('limit-exceeded', path, `expected an array with at most ${maximum} entries`);
  }
  for (const [index, item] of value.entries()) {
    const error = identifiers ? id(item, `${path}[${index}]`) : text(item, `${path}[${index}]`, 120);
    if (error) return error;
  }
  return uniqueIds(value as string[], path);
}

function validateMetadata(value: unknown, path: string) {
  const record = asRecord(value, path, [
    'name',
    'description',
    'category',
    'visibility',
    'tags',
    'createdAt',
    'updatedAt',
  ]);
  if (isIssue(record)) return record;
  const checks = [
    text(record.name, `${path}.name`, 120),
    text(record.description, `${path}.description`, 2_000, true),
    text(record.category, `${path}.category`, 80, true),
    record.visibility === 'private' || record.visibility === 'public'
      ? null
      : issue('invalid-value', `${path}.visibility`, 'expected private or public'),
    stringList(record.tags, `${path}.tags`, WORKFLOW_LIMITS.tags),
    timestamp(record.createdAt, `${path}.createdAt`),
    timestamp(record.updatedAt, `${path}.updatedAt`),
  ];
  const error = checks.find(Boolean);
  if (error) return error;
  if ((record.updatedAt as number) < (record.createdAt as number)) {
    return issue('invalid-value', `${path}.updatedAt`, 'updatedAt cannot precede createdAt');
  }
  return null;
}

function variableKeys(type: string): string[] | null {
  const base = ['id', 'key', 'label', 'description', 'required', 'type'];
  switch (type) {
    case 'text':
    case 'multiline-text':
      return [...base, 'defaultValue', 'placeholder', 'minLength', 'maxLength'];
    case 'number':
      return [...base, 'defaultValue', 'minimum', 'maximum', 'step'];
    case 'boolean':
      return [...base, 'defaultValue'];
    case 'choice':
      return [...base, 'defaultValue', 'options'];
    case 'image':
      return [...base, 'defaultAssetId'];
    case 'image-series':
      return [...base, 'defaultAssetIds', 'minItems', 'maxItems'];
    default:
      return null;
  }
}

function validateVariable(value: unknown, path: string): WorkflowValidationError | null {
  if (!value || typeof value !== 'object' || Array.isArray(value)) {
    return issue('invalid-value', path, 'expected a variable object');
  }
  const type = (value as UnknownRecord).type;
  const keys = typeof type === 'string' ? variableKeys(type) : null;
  if (!keys) return issue('invalid-value', `${path}.type`, 'unsupported variable type');
  const record = asRecord(value, path, keys);
  if (isIssue(record)) return record;
  const common = [
    id(record.id, `${path}.id`),
    typeof record.key === 'string' && KEY.test(record.key)
      ? null
      : issue('invalid-value', `${path}.key`, 'expected a stable snake_case input key'),
    text(record.label, `${path}.label`, 120),
    text(record.description, `${path}.description`, 1_000, true),
    typeof record.required === 'boolean'
      ? null
      : issue('invalid-value', `${path}.required`, 'expected a boolean'),
  ].find(Boolean);
  if (common) return common;
  if (type === 'text' || type === 'multiline-text') {
    if (record.defaultValue !== null) {
      const error = text(record.defaultValue, `${path}.defaultValue`, WORKFLOW_LIMITS.text, true);
      if (error) return error;
    }
    const placeholderError = text(record.placeholder, `${path}.placeholder`, 500, true);
    if (placeholderError) return placeholderError;
    if (
      !Number.isSafeInteger(record.minLength) ||
      !Number.isSafeInteger(record.maxLength) ||
      (record.minLength as number) < 0 ||
      (record.maxLength as number) < (record.minLength as number) ||
      (record.maxLength as number) > WORKFLOW_LIMITS.text
    ) {
      return issue('invalid-value', path, 'text length bounds are invalid');
    }
    if (typeof record.defaultValue === 'string' && (record.defaultValue.length < (record.minLength as number) || record.defaultValue.length > (record.maxLength as number))) {
      return issue('invalid-value', `${path}.defaultValue`, 'default text is outside its bounds');
    }
  } else if (type === 'number') {
    for (const key of ['defaultValue', 'minimum', 'maximum', 'step'] as const) {
      if (record[key] !== null) {
        const error = finite(record[key], `${path}.${key}`);
        if (error) return error;
      }
    }
    if (record.minimum !== null && record.maximum !== null && (record.minimum as number) > (record.maximum as number)) {
      return issue('invalid-value', path, 'number minimum cannot exceed maximum');
    }
    if (record.step !== null && (record.step as number) <= 0) {
      return issue('invalid-value', `${path}.step`, 'number step must be positive');
    }
    if (
      record.defaultValue !== null &&
      ((record.minimum !== null && (record.defaultValue as number) < (record.minimum as number)) ||
        (record.maximum !== null && (record.defaultValue as number) > (record.maximum as number)))
    ) {
      return issue('invalid-value', `${path}.defaultValue`, 'default number is outside its bounds');
    }
  } else if (type === 'boolean') {
    if (typeof record.defaultValue !== 'boolean') return issue('invalid-value', `${path}.defaultValue`, 'expected a boolean');
  } else if (type === 'choice') {
    const optionsError = stringList(record.options, `${path}.options`, 100);
    if (optionsError) return optionsError;
    if ((record.options as string[]).length === 0) return issue('invalid-value', `${path}.options`, 'choice options cannot be empty');
    if (record.defaultValue !== null && (typeof record.defaultValue !== 'string' || !(record.options as string[]).includes(record.defaultValue))) {
      return issue('invalid-value', `${path}.defaultValue`, 'default choice must be an option');
    }
  } else if (type === 'image') {
    if (record.defaultAssetId !== null) return id(record.defaultAssetId, `${path}.defaultAssetId`);
  } else {
    const assetsError = stringList(record.defaultAssetIds, `${path}.defaultAssetIds`, WORKFLOW_LIMITS.seriesItems, true);
    if (assetsError) return assetsError;
    if (
      !Number.isSafeInteger(record.minItems) ||
      !Number.isSafeInteger(record.maxItems) ||
      (record.minItems as number) < 0 ||
      (record.maxItems as number) < (record.minItems as number) ||
      (record.maxItems as number) > WORKFLOW_LIMITS.seriesItems ||
      (record.defaultAssetIds as string[]).length < (record.minItems as number) ||
      (record.defaultAssetIds as string[]).length > (record.maxItems as number)
    ) {
      return issue('invalid-value', path, 'image-series bounds are invalid');
    }
  }
  return null;
}

export function validateWorkflowOutput(value: unknown, path = '$.output'): WorkflowValidationError | null {
  if (!value || typeof value !== 'object' || Array.isArray(value)) return issue('invalid-value', path, 'expected an output plan');
  const kind = (value as UnknownRecord).kind;
  const keys = kind === 'single-image' ? ['kind'] : kind === 'multi-image-series' ? ['kind', 'targetCount', 'concurrency', 'reviewRequired'] : null;
  if (!keys) return issue('invalid-value', `${path}.kind`, 'unsupported workflow output mode');
  const record = asRecord(value, path, keys);
  if (isIssue(record)) return record;
  if (kind === 'multi-image-series') {
    if (!Number.isSafeInteger(record.targetCount) || (record.targetCount as number) < 2 || (record.targetCount as number) > WORKFLOW_LIMITS.seriesItems) {
      return issue('invalid-value', `${path}.targetCount`, 'series target count must be between 2 and 20');
    }
    if (!Number.isSafeInteger(record.concurrency) || (record.concurrency as number) < 1 || (record.concurrency as number) > 6 || (record.concurrency as number) > (record.targetCount as number)) {
      return issue('invalid-value', `${path}.concurrency`, 'series concurrency is invalid');
    }
    if (typeof record.reviewRequired !== 'boolean') return issue('invalid-value', `${path}.reviewRequired`, 'expected a boolean');
  }
  return null;
}

function validateTemplate(value: unknown, path: string) {
  const record = asRecord(value, path, ['id', 'name', 'segments']);
  if (isIssue(record)) return record;
  const common = id(record.id, `${path}.id`) ?? text(record.name, `${path}.name`, 120);
  if (common) return common;
  if (!Array.isArray(record.segments) || record.segments.length === 0 || record.segments.length > WORKFLOW_LIMITS.templateSegments) {
    return issue('limit-exceeded', `${path}.segments`, 'template must contain 1 to 500 segments');
  }
  for (const [index, value] of record.segments.entries()) {
    if (!value || typeof value !== 'object' || Array.isArray(value)) return issue('invalid-value', `${path}.segments[${index}]`, 'expected a template segment');
    const kind = (value as UnknownRecord).kind;
    const segment = asRecord(value, `${path}.segments[${index}]`, kind === 'text' ? ['kind', 'text'] : kind === 'variable' ? ['kind', 'variableId'] : []);
    if (isIssue(segment)) return segment;
    if (kind === 'text') {
      const error = text(segment.text, `${path}.segments[${index}].text`, WORKFLOW_LIMITS.prompt, true);
      if (error) return error;
    } else if (kind === 'variable') {
      const error = id(segment.variableId, `${path}.segments[${index}].variableId`);
      if (error) return error;
    } else return issue('invalid-value', `${path}.segments[${index}].kind`, 'unsupported template segment');
  }
  return null;
}

function validateStep(value: unknown, path: string) {
  if (!value || typeof value !== 'object' || Array.isArray(value)) return issue('invalid-value', path, 'expected a workflow step');
  const kind = (value as UnknownRecord).kind;
  const base = ['id', 'name', 'dependsOn', 'enabled', 'kind'];
  const keys = kind === 'render-template' || kind === 'draft-prompts' ? [...base, 'templateId'] : kind === 'generate-images' ? [...base, 'promptSource', 'referenceVariableIds'] : kind === 'record-history' ? [...base, 'sourceStepIds'] : null;
  if (!keys) return issue('invalid-value', `${path}.kind`, 'unsupported workflow step');
  const record = asRecord(value, path, keys);
  if (isIssue(record)) return record;
  const common = id(record.id, `${path}.id`) ?? text(record.name, `${path}.name`, 120) ?? stringList(record.dependsOn, `${path}.dependsOn`, WORKFLOW_LIMITS.steps, true);
  if (common) return common;
  if (typeof record.enabled !== 'boolean') return issue('invalid-value', `${path}.enabled`, 'expected a boolean');
  if (kind === 'render-template' || kind === 'draft-prompts') return id(record.templateId, `${path}.templateId`);
  if (kind === 'record-history') return stringList(record.sourceStepIds, `${path}.sourceStepIds`, WORKFLOW_LIMITS.steps, true);
  const refs = stringList(record.referenceVariableIds, `${path}.referenceVariableIds`, WORKFLOW_LIMITS.variables, true);
  if (refs) return refs;
  if (!record.promptSource || typeof record.promptSource !== 'object' || Array.isArray(record.promptSource)) return issue('invalid-value', `${path}.promptSource`, 'expected a prompt source');
  const sourceKind = (record.promptSource as UnknownRecord).kind;
  const source = asRecord(record.promptSource, `${path}.promptSource`, sourceKind === 'template' ? ['kind', 'templateId'] : sourceKind === 'prompt-drafts' ? ['kind', 'stepId'] : []);
  if (isIssue(source)) return source;
  if (sourceKind === 'template') return id(source.templateId, `${path}.promptSource.templateId`);
  if (sourceKind === 'prompt-drafts') return id(source.stepId, `${path}.promptSource.stepId`);
  return issue('invalid-value', `${path}.promptSource.kind`, 'unsupported prompt source');
}

export function validateWorkflowDefinition(value: unknown, path = '$'): WorkflowValidationResult {
  const record = asRecord(value, path, ['id', 'revision', 'metadata', 'output', 'variables', 'templates', 'steps']);
  if (isIssue(record)) return { ok: false, error: record };
  const common = id(record.id, `${path}.id`) ?? (!Number.isSafeInteger(record.revision) || (record.revision as number) < 1 ? issue('invalid-value', `${path}.revision`, 'revision must be a positive safe integer') : null) ?? validateMetadata(record.metadata, `${path}.metadata`) ?? validateWorkflowOutput(record.output, `${path}.output`);
  if (common) return { ok: false, error: common };
  for (const [key, maximum, validator] of [
    ['variables', WORKFLOW_LIMITS.variables, validateVariable],
    ['templates', WORKFLOW_LIMITS.templates, validateTemplate],
    ['steps', WORKFLOW_LIMITS.steps, validateStep],
  ] as const) {
    const list = record[key];
    if (!Array.isArray(list) || list.length > maximum) return { ok: false, error: issue('limit-exceeded', `${path}.${key}`, `too many ${key}`) };
    for (const [index, item] of list.entries()) {
      const error = validator(item, `${path}.${key}[${index}]`);
      if (error) return { ok: false, error };
    }
  }
  const workflow = value as WorkflowDefinitionV1;
  for (const [collection, ids, keys] of [
    ['variables', workflow.variables.map((item) => item.id), workflow.variables.map((item) => item.key)],
    ['templates', workflow.templates.map((item) => item.id), []],
    ['steps', workflow.steps.map((item) => item.id), []],
  ] as const) {
    const duplicate = uniqueIds(ids, `${path}.${collection}`) ?? (keys.length ? uniqueIds(keys, `${path}.${collection}`) : null);
    if (duplicate) return { ok: false, error: duplicate };
  }
  const ownedIds = [
    workflow.id,
    ...workflow.variables.map((item) => item.id),
    ...workflow.templates.map((item) => item.id),
    ...workflow.steps.map((item) => item.id),
  ];
  const globalDuplicate = uniqueIds(ownedIds, path);
  if (globalDuplicate) return { ok: false, error: globalDuplicate };
  if (workflow.templates.length === 0 || workflow.steps.length === 0 || !workflow.steps.some((step) => step.kind === 'generate-images')) {
    return { ok: false, error: issue('invalid-value', path, 'workflow requires a prompt template and image-generation step') };
  }
  const graphError = validateWorkflowGraph(workflow);
  return graphError ? { ok: false, error: graphError } : { ok: true };
}

function validateInput(value: unknown, path: string): WorkflowValidationError | null {
  if (!value || typeof value !== 'object' || Array.isArray(value)) return issue('invalid-value', path, 'expected an input value');
  const type = (value as UnknownRecord).type;
  const keys = type === 'text' || type === 'multiline-text' || type === 'choice' || type === 'number' || type === 'boolean' ? ['variableId', 'type', 'value'] : type === 'image' ? ['variableId', 'type', 'assetId'] : type === 'image-series' ? ['variableId', 'type', 'assetIds'] : null;
  if (!keys) return issue('invalid-value', `${path}.type`, 'unsupported input type');
  const record = asRecord(value, path, keys);
  if (isIssue(record)) return record;
  const idError = id(record.variableId, `${path}.variableId`);
  if (idError) return idError;
  if (type === 'text' || type === 'multiline-text' || type === 'choice') return text(record.value, `${path}.value`, WORKFLOW_LIMITS.text, true);
  if (type === 'number') return finite(record.value, `${path}.value`);
  if (type === 'boolean') return typeof record.value === 'boolean' ? null : issue('invalid-value', `${path}.value`, 'expected a boolean');
  if (type === 'image') return record.assetId === null ? null : id(record.assetId, `${path}.assetId`);
  return stringList(record.assetIds, `${path}.assetIds`, WORKFLOW_LIMITS.seriesItems, true);
}

export function validateWorkflowInputsForDefinition(workflow: WorkflowDefinitionV1, inputs: unknown, path = '$.inputs'): WorkflowValidationResult {
  if (!Array.isArray(inputs) || inputs.length > workflow.variables.length) return { ok: false, error: issue('limit-exceeded', path, 'input count exceeds variable count') };
  const ids: string[] = [];
  for (const [index, input] of inputs.entries()) {
    const shape = validateInput(input, `${path}[${index}]`);
    if (shape) return { ok: false, error: shape };
    const typed = input as WorkflowInputValue;
    ids.push(typed.variableId);
    const variable = workflow.variables.find((item) => item.id === typed.variableId);
    if (!variable || variable.type !== typed.type) return { ok: false, error: issue('broken-reference', `${path}[${index}].variableId`, 'input does not match a workflow variable') };
    if (
      (typed.type === 'text' || typed.type === 'multiline-text') &&
      (variable.type === 'text' || variable.type === 'multiline-text') &&
      (typed.value.length < variable.minLength || typed.value.length > variable.maxLength)
    ) return { ok: false, error: issue('invalid-value', `${path}[${index}].value`, 'text input is outside its bounds') };
    if (typed.type === 'choice' && variable.type === 'choice' && !variable.options.includes(typed.value)) return { ok: false, error: issue('invalid-value', `${path}[${index}].value`, 'choice input is not an option') };
    if (typed.type === 'number' && variable.type === 'number' && ((variable.minimum !== null && typed.value < variable.minimum) || (variable.maximum !== null && typed.value > variable.maximum))) return { ok: false, error: issue('invalid-value', `${path}[${index}].value`, 'number input is outside its bounds') };
    if (typed.type === 'image-series' && variable.type === 'image-series' && (typed.assetIds.length < variable.minItems || typed.assetIds.length > variable.maxItems)) return { ok: false, error: issue('invalid-value', `${path}[${index}].assetIds`, 'image-series input is outside its bounds') };
  }
  const duplicate = uniqueIds(ids, path);
  if (duplicate) return { ok: false, error: duplicate };
  for (const variable of workflow.variables) {
    if (!variable.required) continue;
    const input = (inputs as WorkflowInputValue[]).find((item) => item.variableId === variable.id);
    const absent = !input || (input.type === 'image' && input.assetId === null) || (input.type === 'image-series' && input.assetIds.length === 0) || ((input.type === 'text' || input.type === 'multiline-text' || input.type === 'choice') && input.value.trim().length === 0);
    if (absent) return { ok: false, error: issue('invalid-value', path, `required input ${variable.key} is missing`) };
  }
  return { ok: true };
}

function validateRunRequest(value: unknown, path: string) {
  const record = asRecord(value, path, ['id', 'idempotencyKey', 'workflowId', 'workflowRevision', 'requestedAt', 'output', 'inputs']);
  if (isIssue(record)) return record;
  const common = id(record.id, `${path}.id`) ?? id(record.idempotencyKey, `${path}.idempotencyKey`) ?? id(record.workflowId, `${path}.workflowId`) ?? (!Number.isSafeInteger(record.workflowRevision) || (record.workflowRevision as number) < 1 ? issue('invalid-value', `${path}.workflowRevision`, 'expected a positive revision') : null) ?? timestamp(record.requestedAt, `${path}.requestedAt`) ?? validateWorkflowOutput(record.output, `${path}.output`);
  if (common) return common;
  if (!Array.isArray(record.inputs) || record.inputs.length > WORKFLOW_LIMITS.variables) return issue('limit-exceeded', `${path}.inputs`, 'too many inputs');
  const ids: string[] = [];
  for (const [index, input] of record.inputs.entries()) {
    const error = validateInput(input, `${path}.inputs[${index}]`);
    if (error) return error;
    ids.push((input as WorkflowInputValue).variableId);
  }
  return uniqueIds(ids, `${path}.inputs`);
}

function validateDraft(value: unknown, path: string) {
  const record = asRecord(value, path, ['id', 'workflowId', 'runRequestId', 'seriesIndex', 'title', 'prompt', 'status', 'createdAt', 'reviewedAt', 'reviewNote']);
  if (isIssue(record)) return record;
  const common = id(record.id, `${path}.id`) ?? id(record.workflowId, `${path}.workflowId`) ?? id(record.runRequestId, `${path}.runRequestId`) ?? (!Number.isSafeInteger(record.seriesIndex) || (record.seriesIndex as number) < 0 || (record.seriesIndex as number) >= WORKFLOW_LIMITS.seriesItems ? issue('invalid-value', `${path}.seriesIndex`, 'series index is invalid') : null) ?? text(record.title, `${path}.title`, 120) ?? text(record.prompt, `${path}.prompt`, WORKFLOW_LIMITS.prompt) ?? (!['pending-review', 'approved', 'rejected'].includes(record.status as string) ? issue('invalid-value', `${path}.status`, 'unsupported draft status') : null) ?? timestamp(record.createdAt, `${path}.createdAt`);
  if (common) return common;
  if (record.reviewedAt !== null) {
    const error = timestamp(record.reviewedAt, `${path}.reviewedAt`);
    if (error) return error;
  }
  if (record.reviewNote !== null) {
    const error = text(record.reviewNote, `${path}.reviewNote`, 2_000, true);
    if (error) return error;
  }
  if (record.status === 'pending-review' && (record.reviewedAt !== null || record.reviewNote !== null)) return issue('invalid-value', path, 'pending draft cannot contain review data');
  if (record.status !== 'pending-review' && record.reviewedAt === null) return issue('invalid-value', `${path}.reviewedAt`, 'reviewed draft requires a timestamp');
  if (record.reviewedAt !== null && (record.reviewedAt as number) < (record.createdAt as number)) return issue('invalid-value', `${path}.reviewedAt`, 'reviewedAt cannot precede createdAt');
  return null;
}

function validateFailure(value: unknown, path: string) {
  if (value === null) return null;
  const record = asRecord(value, path, ['code', 'message']);
  if (isIssue(record)) return record;
  if (typeof record.code !== 'string' || !CODE.test(record.code)) return issue('invalid-value', `${path}.code`, 'invalid failure code');
  return text(record.message, `${path}.message`, 2_000);
}

function validateRun(value: unknown, path: string) {
  const record = asRecord(value, path, ['requestId', 'workflowId', 'status', 'promptDraftIds', 'taskIds', 'resultAssetIds', 'historyReferenceIds', 'queuedAt', 'startedAt', 'completedAt', 'failure']);
  if (isIssue(record)) return record;
  const common = id(record.requestId, `${path}.requestId`) ?? id(record.workflowId, `${path}.workflowId`) ?? (!['requested', 'awaiting-review', 'queued', 'running', 'succeeded', 'failed', 'cancelled'].includes(record.status as string) ? issue('invalid-value', `${path}.status`, 'unsupported run status') : null);
  if (common) return common;
  for (const key of ['promptDraftIds', 'taskIds', 'resultAssetIds', 'historyReferenceIds'] as const) {
    const error = stringList(record[key], `${path}.${key}`, WORKFLOW_LIMITS.taskReferences, true);
    if (error) return error;
  }
  for (const key of ['queuedAt', 'startedAt', 'completedAt'] as const) {
    if (record[key] !== null) {
      const error = timestamp(record[key], `${path}.${key}`);
      if (error) return error;
    }
  }
  const failureError = validateFailure(record.failure, `${path}.failure`);
  if (failureError) return failureError;
  const status = record.status as string;
  const early = status === 'requested' || status === 'awaiting-review';
  if (early && (record.queuedAt !== null || record.startedAt !== null || record.completedAt !== null || (record.taskIds as string[]).length || (record.resultAssetIds as string[]).length || (record.historyReferenceIds as string[]).length || record.failure !== null)) return issue('invalid-value', path, 'unstarted run contains execution data');
  if (status === 'queued' && (record.queuedAt === null || record.startedAt !== null || record.completedAt !== null || record.failure !== null)) return issue('invalid-value', path, 'queued run timestamps are invalid');
  if (status === 'running' && (record.queuedAt === null || record.startedAt === null || record.completedAt !== null || (record.taskIds as string[]).length === 0 || record.failure !== null)) return issue('invalid-value', path, 'running projection is incomplete');
  if (status === 'succeeded' && (record.queuedAt === null || record.startedAt === null || record.completedAt === null || (record.resultAssetIds as string[]).length === 0 || record.failure !== null)) return issue('invalid-value', path, 'successful projection is incomplete');
  if (status === 'failed' && (record.completedAt === null || record.failure === null)) return issue('invalid-value', path, 'failed projection requires failure data');
  if (status === 'cancelled' && (record.completedAt === null || record.failure !== null)) return issue('invalid-value', path, 'cancelled projection is invalid');
  if (record.queuedAt !== null && (record.queuedAt as number) < 0) return issue('invalid-value', path, 'run timestamps are invalid');
  if (record.startedAt !== null && record.queuedAt !== null && (record.startedAt as number) < (record.queuedAt as number)) return issue('invalid-value', path, 'startedAt precedes queuedAt');
  if (record.completedAt !== null && record.startedAt !== null && (record.completedAt as number) < (record.startedAt as number)) return issue('invalid-value', path, 'completedAt precedes startedAt');
  return null;
}

export function validateWorkflowWorkspaceDocument(value: unknown): WorkflowValidationResult {
  const record = asRecord(value, '$', ['kind', 'version', 'workflows', 'promptDrafts', 'runRequests', 'runs']);
  if (isIssue(record)) return { ok: false, error: record };
  if (record.kind !== 'nomifun.creative-studio.workflows') return { ok: false, error: issue('invalid-envelope', '$.kind', 'unexpected workflow document kind') };
  if (record.version !== 1) return { ok: false, error: issue('unsupported-version', '$.version', 'only workflow v1 is supported') };
  for (const [key, maximum, validator] of [
    ['workflows', WORKFLOW_LIMITS.workflows, (item: unknown, path: string) => { const result = validateWorkflowDefinition(item, path); return result.ok ? null : result.error; }],
    ['promptDrafts', WORKFLOW_LIMITS.drafts, validateDraft],
    ['runRequests', WORKFLOW_LIMITS.runs, validateRunRequest],
    ['runs', WORKFLOW_LIMITS.runs, validateRun],
  ] as const) {
    const list = record[key];
    if (!Array.isArray(list) || list.length > maximum) return { ok: false, error: issue('limit-exceeded', `$.${key}`, `too many ${key}`) };
    for (const [index, item] of list.entries()) {
      const error = validator(item, `$.${key}[${index}]`);
      if (error) return { ok: false, error };
    }
  }
  const document = value as WorkflowWorkspaceDocumentV1;
  const identitySets: Array<[string, string[]]> = [
    ['$.workflows', document.workflows.map((item) => item.id)],
    ['$.promptDrafts', document.promptDrafts.map((item) => item.id)],
    ['$.runRequests', document.runRequests.map((item) => item.id)],
    ['$.runs', document.runs.map((item) => item.requestId)],
    ['$.runRequests.idempotencyKey', document.runRequests.map((item) => item.idempotencyKey)],
  ];
  for (const [path, values] of identitySets) {
    const error = uniqueIds(values, path);
    if (error) return { ok: false, error };
  }
  if (document.runRequests.length !== document.runs.length) return { ok: false, error: issue('broken-reference', '$.runs', 'every run request needs exactly one status projection') };
  for (const [index, request] of document.runRequests.entries()) {
    const run = document.runs.find((item) => item.requestId === request.id);
    if (!run || run.workflowId !== request.workflowId) return { ok: false, error: issue('broken-reference', `$.runRequests[${index}]`, 'run projection is missing or belongs to another workflow') };
    const workflow = document.workflows.find((item) => item.id === request.workflowId);
    const terminal = run.status === 'succeeded' || run.status === 'failed' || run.status === 'cancelled';
    if (!workflow && !terminal) return { ok: false, error: issue('broken-reference', `$.runRequests[${index}].workflowId`, 'active run workflow does not exist') };
    if (workflow && request.workflowRevision > workflow.revision) return { ok: false, error: issue('broken-reference', `$.runRequests[${index}].workflowRevision`, 'run references a future workflow revision') };
    if (workflow && request.workflowRevision === workflow.revision) {
      const inputResult = validateWorkflowInputsForDefinition(workflow, request.inputs, `$.runRequests[${index}].inputs`);
      if (!inputResult.ok) return inputResult;
      if (JSON.stringify(request.output) !== JSON.stringify(workflow.output)) return { ok: false, error: issue('invalid-value', `$.runRequests[${index}].output`, 'run output snapshot does not match its workflow revision') };
    }
    const drafts = document.promptDrafts.filter((draft) => draft.runRequestId === request.id);
    if ((run.queuedAt !== null && run.queuedAt < request.requestedAt) || (run.completedAt !== null && run.completedAt < request.requestedAt)) return { ok: false, error: issue('invalid-value', `$.runs[${document.runs.indexOf(run)}]`, 'run timestamps cannot precede the request') };
    if (drafts.some((draft) => draft.createdAt < request.requestedAt)) return { ok: false, error: issue('invalid-value', '$.promptDrafts', 'prompt draft cannot predate its request') };
    if (drafts.some((draft) => draft.workflowId !== request.workflowId) || run.promptDraftIds.length !== drafts.length || run.promptDraftIds.some((draftId) => !drafts.some((draft) => draft.id === draftId))) return { ok: false, error: issue('broken-reference', `$.runs[${document.runs.indexOf(run)}].promptDraftIds`, 'prompt draft projection is inconsistent') };
    if (new Set(drafts.map((draft) => draft.seriesIndex)).size !== drafts.length) return { ok: false, error: issue('duplicate-id', '$.promptDrafts', 'series indexes must be unique per run') };
    if (request.output.kind === 'single-image' && drafts.length > 0) return { ok: false, error: issue('invalid-value', '$.promptDrafts', 'single-image run cannot contain series drafts') };
    if ((run.status === 'queued' || run.status === 'running' || terminal) && request.output.kind === 'multi-image-series') {
      if (drafts.length !== request.output.targetCount || (request.output.reviewRequired && drafts.some((draft) => draft.status !== 'approved'))) return { ok: false, error: issue('invalid-transition', `$.runs[${document.runs.indexOf(run)}].status`, 'series cannot execute before its prompt set is complete and approved') };
    }
  }
  return { ok: true };
}

export function isWorkflowBusinessId(value: unknown): value is string {
  return typeof value === 'string' && CANONICAL_UUID_V7.test(value);
}

export function isWorkflowTerminalStatus(status: string): boolean {
  return status === 'succeeded' || status === 'failed' || status === 'cancelled';
}

export function cloneWorkflowOutput(output: WorkflowOutputPlan): WorkflowOutputPlan {
  return output.kind === 'single-image' ? { kind: 'single-image' } : { ...output };
}

export function cloneWorkflowVariable(variable: WorkflowVariable): WorkflowVariable {
  if (variable.type === 'choice') return { ...variable, options: [...variable.options] };
  if (variable.type === 'image-series') return { ...variable, defaultAssetIds: [...variable.defaultAssetIds] };
  return { ...variable };
}
