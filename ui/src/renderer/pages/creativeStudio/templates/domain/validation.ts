/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { CANONICAL_UUID_V7 } from '@/common/types/ids';

import { validateTemplateGraph } from './graph';
import type {
  CreativeTemplateDefinitionV1,
  CreativeTemplateInputValue,
  CreativeTemplateOutputPlan,
  CreativeTemplateRunRequest,
  CreativeTemplateValidationError,
  CreativeTemplateValidationResult,
  CreativeTemplateVariable,
  CreativeTemplateWorkspaceDocumentV1,
} from './types';

type UnknownRecord = Record<string, unknown>;

export const TEMPLATE_LIMITS = {
  jsonBytes: 8 * 1024 * 1024,
  definitions: 500,
  variables: 100,
  promptTemplates: 50,
  promptTemplateSegments: 500,
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
  code: CreativeTemplateValidationError['code'],
  path: string,
  message: string
): CreativeTemplateValidationError {
  return { code, path, message };
}

function asRecord(value: unknown, path: string, keys: readonly string[]): UnknownRecord | CreativeTemplateValidationError {
  if (!value || typeof value !== 'object' || Array.isArray(value)) {
    return issue('invalid-value', path, 'expected an object');
  }
  const record = value as UnknownRecord;
  const actual = Object.keys(record);
  const unknown = actual.find((key) => !keys.includes(key));
  if (unknown) return issue('unknown-field', `${path}.${unknown}`, 'field is not part of template v1');
  const missing = keys.find((key) => !Object.prototype.hasOwnProperty.call(record, key));
  if (missing) return issue('invalid-value', `${path}.${missing}`, 'required field is missing');
  return record;
}

function isIssue(value: UnknownRecord | CreativeTemplateValidationError): value is CreativeTemplateValidationError {
  return 'code' in value && 'path' in value && 'message' in value;
}

function text(
  value: unknown,
  path: string,
  maximum: number = TEMPLATE_LIMITS.text,
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
    stringList(record.tags, `${path}.tags`, TEMPLATE_LIMITS.tags),
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

function validateVariable(value: unknown, path: string): CreativeTemplateValidationError | null {
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
      const error = text(record.defaultValue, `${path}.defaultValue`, TEMPLATE_LIMITS.text, true);
      if (error) return error;
    }
    const placeholderError = text(record.placeholder, `${path}.placeholder`, 500, true);
    if (placeholderError) return placeholderError;
    if (
      !Number.isSafeInteger(record.minLength) ||
      !Number.isSafeInteger(record.maxLength) ||
      (record.minLength as number) < 0 ||
      (record.maxLength as number) < (record.minLength as number) ||
      (record.maxLength as number) > TEMPLATE_LIMITS.text
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
    const assetsError = stringList(record.defaultAssetIds, `${path}.defaultAssetIds`, TEMPLATE_LIMITS.seriesItems, true);
    if (assetsError) return assetsError;
    if (
      !Number.isSafeInteger(record.minItems) ||
      !Number.isSafeInteger(record.maxItems) ||
      (record.minItems as number) < 0 ||
      (record.maxItems as number) < (record.minItems as number) ||
      (record.maxItems as number) > TEMPLATE_LIMITS.seriesItems ||
      (record.defaultAssetIds as string[]).length < (record.minItems as number) ||
      (record.defaultAssetIds as string[]).length > (record.maxItems as number)
    ) {
      return issue('invalid-value', path, 'image-series bounds are invalid');
    }
  }
  return null;
}

export function validateTemplateOutput(value: unknown, path = '$.output'): CreativeTemplateValidationError | null {
  if (!value || typeof value !== 'object' || Array.isArray(value)) return issue('invalid-value', path, 'expected an output plan');
  const kind = (value as UnknownRecord).kind;
  const keys = kind === 'single-image' ? ['kind'] : kind === 'multi-image-series' ? ['kind', 'targetCount', 'concurrency', 'reviewRequired'] : null;
  if (!keys) return issue('invalid-value', `${path}.kind`, 'unsupported template output mode');
  const record = asRecord(value, path, keys);
  if (isIssue(record)) return record;
  if (kind === 'multi-image-series') {
    if (!Number.isSafeInteger(record.targetCount) || (record.targetCount as number) < 2 || (record.targetCount as number) > TEMPLATE_LIMITS.seriesItems) {
      return issue('invalid-value', `${path}.targetCount`, 'series target count must be between 2 and 20');
    }
    if (!Number.isSafeInteger(record.concurrency) || (record.concurrency as number) < 1 || (record.concurrency as number) > 6 || (record.concurrency as number) > (record.targetCount as number)) {
      return issue('invalid-value', `${path}.concurrency`, 'series concurrency is invalid');
    }
    if (typeof record.reviewRequired !== 'boolean') return issue('invalid-value', `${path}.reviewRequired`, 'expected a boolean');
  }
  return null;
}

function validatePromptTemplate(value: unknown, path: string) {
  const record = asRecord(value, path, ['id', 'name', 'segments']);
  if (isIssue(record)) return record;
  const common = id(record.id, `${path}.id`) ?? text(record.name, `${path}.name`, 120);
  if (common) return common;
  if (
    !Array.isArray(record.segments) ||
    record.segments.length === 0 ||
    record.segments.length > TEMPLATE_LIMITS.promptTemplateSegments
  ) {
    return issue(
      'limit-exceeded',
      `${path}.segments`,
      'prompt template must contain 1 to 500 segments'
    );
  }
  for (const [index, value] of record.segments.entries()) {
    if (!value || typeof value !== 'object' || Array.isArray(value)) return issue('invalid-value', `${path}.segments[${index}]`, 'expected a template segment');
    const kind = (value as UnknownRecord).kind;
    const segment = asRecord(value, `${path}.segments[${index}]`, kind === 'text' ? ['kind', 'text'] : kind === 'variable' ? ['kind', 'variableId'] : []);
    if (isIssue(segment)) return segment;
    if (kind === 'text') {
      const error = text(segment.text, `${path}.segments[${index}].text`, TEMPLATE_LIMITS.prompt, true);
      if (error) return error;
    } else if (kind === 'variable') {
      const error = id(segment.variableId, `${path}.segments[${index}].variableId`);
      if (error) return error;
    } else return issue('invalid-value', `${path}.segments[${index}].kind`, 'unsupported template segment');
  }
  return null;
}

function validateStep(value: unknown, path: string) {
  if (!value || typeof value !== 'object' || Array.isArray(value)) return issue('invalid-value', path, 'expected a template step');
  const kind = (value as UnknownRecord).kind;
  const base = ['id', 'name', 'dependsOn', 'enabled', 'kind'];
  const keys = kind === 'render-template'
    ? [...base, 'templateId']
    : kind === 'draft-prompts'
      ? [...base, 'templateId', 'planning']
    : kind === 'generate-images'
      ? [...base, 'promptSource', 'referenceVariableIds', 'generation']
      : kind === 'record-history'
        ? [...base, 'sourceStepIds']
        : null;
  if (!keys) return issue('invalid-value', `${path}.kind`, 'unsupported template step');
  const record = asRecord(value, path, keys);
  if (isIssue(record)) return record;
  const common = id(record.id, `${path}.id`) ?? text(record.name, `${path}.name`, 120) ?? stringList(record.dependsOn, `${path}.dependsOn`, TEMPLATE_LIMITS.steps, true);
  if (common) return common;
  if (typeof record.enabled !== 'boolean') return issue('invalid-value', `${path}.enabled`, 'expected a boolean');
  if (kind === 'render-template') return id(record.templateId, `${path}.templateId`);
  if (kind === 'draft-prompts') {
    const templateError = id(record.templateId, `${path}.templateId`);
    if (templateError) return templateError;
    const planning = asRecord(record.planning, `${path}.planning`, [
      'model',
      'instruction',
      'maxTokens',
    ]);
    if (isIssue(planning)) return planning;
    if (planning.model !== null) {
      const model = asRecord(planning.model, `${path}.planning.model`, [
        'providerId',
        'model',
        'task',
      ]);
      if (isIssue(model)) return model;
      const modelError = id(model.providerId, `${path}.planning.model.providerId`)
        ?? text(model.model, `${path}.planning.model.model`, 512);
      if (modelError) return modelError;
      if (model.task !== 'chat') {
        return issue('invalid-value', `${path}.planning.model.task`, 'prompt planning requires a chat model');
      }
    }
    const instructionError = text(
      planning.instruction,
      `${path}.planning.instruction`,
      2_000,
      true
    );
    if (instructionError) return instructionError;
    if (
      !Number.isSafeInteger(planning.maxTokens) ||
      (planning.maxTokens as number) < 128 ||
      (planning.maxTokens as number) > 32_768
    ) {
      return issue(
        'invalid-value',
        `${path}.planning.maxTokens`,
        'prompt planning maxTokens must be between 128 and 32768'
      );
    }
    return null;
  }
  if (kind === 'record-history') return stringList(record.sourceStepIds, `${path}.sourceStepIds`, TEMPLATE_LIMITS.steps, true);
  const refs = stringList(record.referenceVariableIds, `${path}.referenceVariableIds`, TEMPLATE_LIMITS.variables, true);
  if (refs) return refs;
  const generation = asRecord(record.generation, `${path}.generation`, [
    'model',
    'quality',
    'width',
    'height',
    'imagesPerPrompt',
  ]);
  if (isIssue(generation)) return generation;
  if (generation.model !== null) {
    const model = asRecord(generation.model, `${path}.generation.model`, [
      'providerId',
      'model',
      'task',
    ]);
    if (isIssue(model)) return model;
    const modelError = id(model.providerId, `${path}.generation.model.providerId`)
      ?? text(model.model, `${path}.generation.model.model`, 512);
    if (modelError) return modelError;
    if (model.task !== 'image_generation' && model.task !== 'image_edit') {
      return issue('invalid-value', `${path}.generation.model.task`, 'unsupported image model task');
    }
  }
  if (!['auto', 'high', 'medium', 'low'].includes(generation.quality as string)) {
    return issue('invalid-value', `${path}.generation.quality`, 'unsupported image quality');
  }
  for (const key of ['width', 'height'] as const) {
    if (
      !Number.isSafeInteger(generation[key])
      || (generation[key] as number) < 64
      || (generation[key] as number) > 8192
      || (generation[key] as number) % 16 !== 0
    ) {
      return issue('invalid-value', `${path}.generation.${key}`, 'image dimensions must be 64..8192 and aligned to 16 pixels');
    }
  }
  if (
    !Number.isSafeInteger(generation.imagesPerPrompt)
    || (generation.imagesPerPrompt as number) < 1
    || (generation.imagesPerPrompt as number) > 6
  ) {
    return issue('invalid-value', `${path}.generation.imagesPerPrompt`, 'imagesPerPrompt must be between 1 and 6');
  }
  if (!record.promptSource || typeof record.promptSource !== 'object' || Array.isArray(record.promptSource)) return issue('invalid-value', `${path}.promptSource`, 'expected a prompt source');
  const sourceKind = (record.promptSource as UnknownRecord).kind;
  const source = asRecord(record.promptSource, `${path}.promptSource`, sourceKind === 'template' ? ['kind', 'templateId'] : sourceKind === 'prompt-drafts' ? ['kind', 'stepId'] : []);
  if (isIssue(source)) return source;
  if (sourceKind === 'template') return id(source.templateId, `${path}.promptSource.templateId`);
  if (sourceKind === 'prompt-drafts') return id(source.stepId, `${path}.promptSource.stepId`);
  return issue('invalid-value', `${path}.promptSource.kind`, 'unsupported prompt source');
}

export function validateTemplateDefinition(value: unknown, path = '$'): CreativeTemplateValidationResult {
  const record = asRecord(value, path, ['id', 'revision', 'metadata', 'output', 'variables', 'templates', 'steps']);
  if (isIssue(record)) return { ok: false, error: record };
  const common = id(record.id, `${path}.id`) ?? (!Number.isSafeInteger(record.revision) || (record.revision as number) < 1 ? issue('invalid-value', `${path}.revision`, 'revision must be a positive safe integer') : null) ?? validateMetadata(record.metadata, `${path}.metadata`) ?? validateTemplateOutput(record.output, `${path}.output`);
  if (common) return { ok: false, error: common };
  for (const [key, maximum, validator] of [
    ['variables', TEMPLATE_LIMITS.variables, validateVariable],
    ['templates', TEMPLATE_LIMITS.promptTemplates, validatePromptTemplate],
    ['steps', TEMPLATE_LIMITS.steps, validateStep],
  ] as const) {
    const list = record[key];
    if (!Array.isArray(list) || list.length > maximum) return { ok: false, error: issue('limit-exceeded', `${path}.${key}`, `too many ${key}`) };
    for (const [index, item] of list.entries()) {
      const error = validator(item, `${path}.${key}[${index}]`);
      if (error) return { ok: false, error };
    }
  }
  const template = value as CreativeTemplateDefinitionV1;
  for (const [collection, ids, keys] of [
    ['variables', template.variables.map((item) => item.id), template.variables.map((item) => item.key)],
    ['templates', template.templates.map((item) => item.id), []],
    ['steps', template.steps.map((item) => item.id), []],
  ] as const) {
    const duplicate = uniqueIds(ids, `${path}.${collection}`) ?? (keys.length ? uniqueIds(keys, `${path}.${collection}`) : null);
    if (duplicate) return { ok: false, error: duplicate };
  }
  const ownedIds = [
    template.id,
    ...template.variables.map((item) => item.id),
    ...template.templates.map((item) => item.id),
    ...template.steps.map((item) => item.id),
  ];
  const globalDuplicate = uniqueIds(ownedIds, path);
  if (globalDuplicate) return { ok: false, error: globalDuplicate };
  if (template.templates.length === 0 || template.steps.length === 0 || !template.steps.some((step) => step.kind === 'generate-images')) {
    return { ok: false, error: issue('invalid-value', path, 'template requires a prompt template and image-generation step') };
  }
  const graphError = validateTemplateGraph(template);
  return graphError ? { ok: false, error: graphError } : { ok: true };
}

function validateInput(value: unknown, path: string): CreativeTemplateValidationError | null {
  if (!value || typeof value !== 'object' || Array.isArray(value)) return issue('invalid-value', path, 'expected an input value');
  const type = (value as UnknownRecord).type;
  const keys = type === 'text' || type === 'multiline-text' || type === 'choice' || type === 'number' || type === 'boolean' ? ['variableId', 'type', 'value'] : type === 'image' ? ['variableId', 'type', 'assetId'] : type === 'image-series' ? ['variableId', 'type', 'assetIds'] : null;
  if (!keys) return issue('invalid-value', `${path}.type`, 'unsupported input type');
  const record = asRecord(value, path, keys);
  if (isIssue(record)) return record;
  const idError = id(record.variableId, `${path}.variableId`);
  if (idError) return idError;
  if (type === 'text' || type === 'multiline-text' || type === 'choice') return text(record.value, `${path}.value`, TEMPLATE_LIMITS.text, true);
  if (type === 'number') return finite(record.value, `${path}.value`);
  if (type === 'boolean') return typeof record.value === 'boolean' ? null : issue('invalid-value', `${path}.value`, 'expected a boolean');
  if (type === 'image') return record.assetId === null ? null : id(record.assetId, `${path}.assetId`);
  return stringList(record.assetIds, `${path}.assetIds`, TEMPLATE_LIMITS.seriesItems, true);
}

export function validateTemplateInputValues(
  inputs: unknown,
  path = '$.inputs',
  maximum: number = TEMPLATE_LIMITS.variables
): CreativeTemplateValidationResult {
  if (!Array.isArray(inputs)) {
    return { ok: false, error: issue('invalid-value', path, 'expected an input array') };
  }
  if (inputs.length > maximum) {
    return { ok: false, error: issue('limit-exceeded', path, 'input count exceeds its limit') };
  }
  const ids: string[] = [];
  for (const [index, input] of inputs.entries()) {
    const shape = validateInput(input, `${path}[${index}]`);
    if (shape) return { ok: false, error: shape };
    ids.push((input as CreativeTemplateInputValue).variableId);
  }
  const duplicate = uniqueIds(ids, path);
  return duplicate ? { ok: false, error: duplicate } : { ok: true };
}

export function validateTemplateInputsForDefinition(template: CreativeTemplateDefinitionV1, inputs: unknown, path = '$.inputs'): CreativeTemplateValidationResult {
  if (!Array.isArray(inputs) || inputs.length > template.variables.length) return { ok: false, error: issue('limit-exceeded', path, 'input count exceeds variable count') };
  const shape = validateTemplateInputValues(inputs, path, template.variables.length);
  if (!shape.ok) return shape;
  for (const [index, input] of inputs.entries()) {
    const typed = input as CreativeTemplateInputValue;
    const variable = template.variables.find((item) => item.id === typed.variableId);
    if (!variable || variable.type !== typed.type) return { ok: false, error: issue('broken-reference', `${path}[${index}].variableId`, 'input does not match a template variable') };
    if (
      (typed.type === 'text' || typed.type === 'multiline-text') &&
      (variable.type === 'text' || variable.type === 'multiline-text') &&
      (typed.value.length < variable.minLength || typed.value.length > variable.maxLength)
    ) return { ok: false, error: issue('invalid-value', `${path}[${index}].value`, 'text input is outside its bounds') };
    if (typed.type === 'choice' && variable.type === 'choice' && !variable.options.includes(typed.value)) return { ok: false, error: issue('invalid-value', `${path}[${index}].value`, 'choice input is not an option') };
    if (typed.type === 'number' && variable.type === 'number' && ((variable.minimum !== null && typed.value < variable.minimum) || (variable.maximum !== null && typed.value > variable.maximum))) return { ok: false, error: issue('invalid-value', `${path}[${index}].value`, 'number input is outside its bounds') };
    if (typed.type === 'image-series' && variable.type === 'image-series' && (typed.assetIds.length < variable.minItems || typed.assetIds.length > variable.maxItems)) return { ok: false, error: issue('invalid-value', `${path}[${index}].assetIds`, 'image-series input is outside its bounds') };
  }
  for (const variable of template.variables) {
    if (!variable.required) continue;
    const input = (inputs as CreativeTemplateInputValue[]).find((item) => item.variableId === variable.id);
    const absent = !input || (input.type === 'image' && input.assetId === null) || (input.type === 'image-series' && input.assetIds.length === 0) || ((input.type === 'text' || input.type === 'multiline-text' || input.type === 'choice') && input.value.trim().length === 0);
    if (absent) return { ok: false, error: issue('invalid-value', path, `required input ${variable.key} is missing`) };
  }
  return { ok: true };
}

function validateRunRequest(value: unknown, path: string) {
  const record = asRecord(value, path, ['id', 'idempotencyKey', 'templateId', 'templateRevision', 'requestedAt', 'output', 'inputs', 'referenceAssetIds']);
  if (isIssue(record)) return record;
  const common = id(record.id, `${path}.id`) ?? id(record.idempotencyKey, `${path}.idempotencyKey`) ?? id(record.templateId, `${path}.templateId`) ?? (!Number.isSafeInteger(record.templateRevision) || (record.templateRevision as number) < 1 ? issue('invalid-value', `${path}.templateRevision`, 'expected a positive revision') : null) ?? timestamp(record.requestedAt, `${path}.requestedAt`) ?? validateTemplateOutput(record.output, `${path}.output`);
  if (common) return common;
  if (!Array.isArray(record.inputs) || record.inputs.length > TEMPLATE_LIMITS.variables) return issue('limit-exceeded', `${path}.inputs`, 'too many inputs');
  const ids: string[] = [];
  for (const [index, input] of record.inputs.entries()) {
    const error = validateInput(input, `${path}.inputs[${index}]`);
    if (error) return error;
    ids.push((input as CreativeTemplateInputValue).variableId);
  }
  return uniqueIds(ids, `${path}.inputs`)
    ?? stringList(record.referenceAssetIds, `${path}.referenceAssetIds`, 100, true)
    ?? (record.idempotencyKey === record.id
      ? null
      : issue('invalid-value', `${path}.idempotencyKey`, 'idempotencyKey must equal the durable run id'));
}

function validateDraft(value: unknown, path: string) {
  const record = asRecord(value, path, ['id', 'templateId', 'runRequestId', 'seriesIndex', 'title', 'prompt', 'status', 'createdAt', 'reviewedAt', 'reviewNote']);
  if (isIssue(record)) return record;
  const common = id(record.id, `${path}.id`) ?? id(record.templateId, `${path}.templateId`) ?? id(record.runRequestId, `${path}.runRequestId`) ?? (!Number.isSafeInteger(record.seriesIndex) || (record.seriesIndex as number) < 0 || (record.seriesIndex as number) >= TEMPLATE_LIMITS.seriesItems ? issue('invalid-value', `${path}.seriesIndex`, 'series index is invalid') : null) ?? text(record.title, `${path}.title`, 120) ?? text(record.prompt, `${path}.prompt`, TEMPLATE_LIMITS.prompt) ?? (!['pending-review', 'approved', 'rejected'].includes(record.status as string) ? issue('invalid-value', `${path}.status`, 'unsupported draft status') : null) ?? timestamp(record.createdAt, `${path}.createdAt`);
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
  const record = asRecord(value, path, ['requestId', 'templateId', 'status', 'promptDraftIds', 'taskIds', 'resultAssetIds', 'historyReferenceIds', 'queuedAt', 'startedAt', 'completedAt', 'failure']);
  if (isIssue(record)) return record;
  const common = id(record.requestId, `${path}.requestId`) ?? id(record.templateId, `${path}.templateId`) ?? (!['requested', 'awaiting-review', 'queued', 'running', 'succeeded', 'failed', 'cancelled'].includes(record.status as string) ? issue('invalid-value', `${path}.status`, 'unsupported run status') : null);
  if (common) return common;
  for (const key of ['promptDraftIds', 'taskIds', 'resultAssetIds', 'historyReferenceIds'] as const) {
    const error = stringList(record[key], `${path}.${key}`, TEMPLATE_LIMITS.taskReferences, true);
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

export function validateTemplateWorkspaceDocument(value: unknown): CreativeTemplateValidationResult {
  const record = asRecord(value, '$', ['kind', 'version', 'templates', 'promptDrafts', 'runRequests', 'runs']);
  if (isIssue(record)) return { ok: false, error: record };
  if (record.kind !== 'nomifun.creative-studio.templates') return { ok: false, error: issue('invalid-envelope', '$.kind', 'unexpected template document kind') };
  if (record.version !== 1) return { ok: false, error: issue('unsupported-version', '$.version', 'only template v1 is supported') };
  for (const [key, maximum, validator] of [
    ['templates', TEMPLATE_LIMITS.definitions, (item: unknown, path: string) => { const result = validateTemplateDefinition(item, path); return result.ok ? null : result.error; }],
    ['promptDrafts', TEMPLATE_LIMITS.drafts, validateDraft],
    ['runRequests', TEMPLATE_LIMITS.runs, validateRunRequest],
    ['runs', TEMPLATE_LIMITS.runs, validateRun],
  ] as const) {
    const list = record[key];
    if (!Array.isArray(list) || list.length > maximum) return { ok: false, error: issue('limit-exceeded', `$.${key}`, `too many ${key}`) };
    for (const [index, item] of list.entries()) {
      const error = validator(item, `$.${key}[${index}]`);
      if (error) return { ok: false, error };
    }
  }
  const document = value as CreativeTemplateWorkspaceDocumentV1;
  const identitySets: Array<[string, string[]]> = [
    ['$.templates', document.templates.map((item) => item.id)],
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
    if (!run || run.templateId !== request.templateId) return { ok: false, error: issue('broken-reference', `$.runRequests[${index}]`, 'run projection is missing or belongs to another template') };
    const template = document.templates.find((item) => item.id === request.templateId);
    const terminal = run.status === 'succeeded' || run.status === 'failed' || run.status === 'cancelled';
    if (!template && !terminal) return { ok: false, error: issue('broken-reference', `$.runRequests[${index}].templateId`, 'active run template does not exist') };
    if (template && request.templateRevision > template.revision) return { ok: false, error: issue('broken-reference', `$.runRequests[${index}].templateRevision`, 'run references a future template revision') };
    if (template && request.templateRevision === template.revision) {
      const inputResult = validateTemplateInputsForDefinition(template, request.inputs, `$.runRequests[${index}].inputs`);
      if (!inputResult.ok) return inputResult;
      if (JSON.stringify(request.output) !== JSON.stringify(template.output)) return { ok: false, error: issue('invalid-value', `$.runRequests[${index}].output`, 'run output snapshot does not match its template revision') };
    }
    const drafts = document.promptDrafts.filter((draft) => draft.runRequestId === request.id);
    if ((run.queuedAt !== null && run.queuedAt < request.requestedAt) || (run.completedAt !== null && run.completedAt < request.requestedAt)) return { ok: false, error: issue('invalid-value', `$.runs[${document.runs.indexOf(run)}]`, 'run timestamps cannot precede the request') };
    if (drafts.some((draft) => draft.createdAt < request.requestedAt)) return { ok: false, error: issue('invalid-value', '$.promptDrafts', 'prompt draft cannot predate its request') };
    if (drafts.some((draft) => draft.templateId !== request.templateId) || run.promptDraftIds.length !== drafts.length || run.promptDraftIds.some((draftId) => !drafts.some((draft) => draft.id === draftId))) return { ok: false, error: issue('broken-reference', `$.runs[${document.runs.indexOf(run)}].promptDraftIds`, 'prompt draft projection is inconsistent') };
    if (new Set(drafts.map((draft) => draft.seriesIndex)).size !== drafts.length) return { ok: false, error: issue('duplicate-id', '$.promptDrafts', 'series indexes must be unique per run') };
    if (request.output.kind === 'single-image' && drafts.length > 0) return { ok: false, error: issue('invalid-value', '$.promptDrafts', 'single-image run cannot contain series drafts') };
    if ((run.status === 'queued' || run.status === 'running' || terminal) && request.output.kind === 'multi-image-series') {
      if (drafts.length !== request.output.targetCount || (request.output.reviewRequired && drafts.some((draft) => draft.status !== 'approved'))) return { ok: false, error: issue('invalid-transition', `$.runs[${document.runs.indexOf(run)}].status`, 'series cannot execute before its prompt set is complete and approved') };
    }
  }
  return { ok: true };
}

export function isTemplateBusinessId(value: unknown): value is string {
  return typeof value === 'string' && CANONICAL_UUID_V7.test(value);
}

export function isTemplateTerminalStatus(status: string): boolean {
  return status === 'succeeded' || status === 'failed' || status === 'cancelled';
}

export function cloneTemplateOutput(output: CreativeTemplateOutputPlan): CreativeTemplateOutputPlan {
  return output.kind === 'single-image' ? { kind: 'single-image' } : { ...output };
}

export function cloneTemplateVariable(variable: CreativeTemplateVariable): CreativeTemplateVariable {
  if (variable.type === 'choice') return { ...variable, options: [...variable.options] };
  if (variable.type === 'image-series') return { ...variable, defaultAssetIds: [...variable.defaultAssetIds] };
  return { ...variable };
}
