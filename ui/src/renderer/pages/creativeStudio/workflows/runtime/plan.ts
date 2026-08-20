/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import type { CreativeJsonObject } from '../../domain/schema';
import type {
  CreateCreativeTaskInput,
  CreativeTaskInput,
  CreativeTaskReference,
} from '../../tasks';
import {
  isWorkflowBusinessId,
  renderWorkflowTemplate,
  topologicallySortWorkflowSteps,
  type WorkflowDefinitionV1,
  type WorkflowGenerateImagesStep,
  type WorkflowInputValue,
  type WorkflowPromptDraft,
  type WorkflowRunAggregateV1,
  type WorkflowStep,
} from '../domain';
import { WorkflowRunRuntimeError } from './types';

export type WorkflowTaskPlanEntry =
  | {
      kind: 'planner';
      taskId: string;
      step: Extract<WorkflowStep, { kind: 'draft-prompts' }>;
    }
  | {
      kind: 'image';
      taskId: string;
      step: WorkflowGenerateImagesStep;
      seriesIndex: number | null;
    };

type WorkflowTaskPlanDescriptor = WorkflowTaskPlanEntry extends infer Entry
  ? Entry extends WorkflowTaskPlanEntry
    ? Omit<Entry, 'taskId'>
    : never
  : never;

function fail(message: string): never {
  throw new WorkflowRunRuntimeError('invalid-plan', message);
}

function taskDescriptors(workflow: WorkflowDefinitionV1): WorkflowTaskPlanDescriptor[] {
  const sorted = topologicallySortWorkflowSteps(workflow);
  if (!sorted.ok) fail(`${sorted.error.path}: ${sorted.error.message}`);
  const entries: WorkflowTaskPlanDescriptor[] = [];
  for (const step of sorted.value) {
    if (!step.enabled) continue;
    if (step.kind === 'draft-prompts') {
      entries.push({ kind: 'planner', step });
      continue;
    }
    if (step.kind !== 'generate-images') continue;
    if (step.promptSource.kind === 'prompt-drafts') {
      if (workflow.output.kind !== 'multi-image-series') {
        fail('prompt-draft image steps require a multi-image output plan');
      }
      for (let seriesIndex = 0; seriesIndex < workflow.output.targetCount; seriesIndex += 1) {
        entries.push({ kind: 'image', step, seriesIndex });
      }
    } else {
      entries.push({ kind: 'image', step, seriesIndex: null });
    }
  }
  if (entries.length === 0) fail('workflow has no executable task plan');
  return entries;
}

export function buildWorkflowTaskPlan(
  workflow: WorkflowDefinitionV1,
  taskIds: readonly string[] | undefined,
  createId: () => string
): WorkflowTaskPlanEntry[] {
  const descriptors = taskDescriptors(workflow);
  const ids = taskIds ?? descriptors.map(() => createId());
  if (ids.length !== descriptors.length) fail('persisted task ids do not match the pinned task plan');
  for (const id of ids) {
    if (!isWorkflowBusinessId(id)) fail('task plan contains a non-canonical task id');
  }
  return descriptors.map((entry, index) => ({ ...entry, taskId: ids[index] })) as WorkflowTaskPlanEntry[];
}

function owner(run: WorkflowRunAggregateV1, stepId: string) {
  return {
    kind: 'workflow_step' as const,
    workflowId: run.request.workflowId,
    workflowRunId: run.request.id,
    workflowStepId: stepId,
  };
}

export function workflowTaskReference(
  run: WorkflowRunAggregateV1,
  entry: WorkflowTaskPlanEntry
): CreativeTaskReference {
  if (entry.kind === 'planner') {
    const model = entry.step.planning.model;
    if (!model) fail(`planner step ${entry.step.id} has no Chat model`);
    return {
      taskId: entry.taskId,
      owner: owner(run, entry.step.id),
      providerId: model.providerId,
      model: model.model,
      task: 'chat',
      capability: 'text',
    };
  }
  const model = entry.step.generation.model;
  if (!model) fail(`image step ${entry.step.id} has no model`);
  return {
    taskId: entry.taskId,
    owner: owner(run, entry.step.id),
    providerId: model.providerId,
    model: model.model,
    task: model.task,
    capability: model.task === 'image_edit' ? 'i2i' : 't2i',
  };
}

function plannerSystem(instruction: string, count: number): string {
  const productInstruction = instruction.trim();
  return [
    productInstruction || 'Keep every image in the series visually coherent.',
    `Return only one JSON object with exactly ${count} prompts.`,
    'The exact schema is {"prompts":[{"title":"...","prompt":"..."}]}.',
    'Do not add markdown fences, commentary, extra fields, or trailing text.',
  ].join('\n');
}

export function createPlannerTaskInput(
  run: WorkflowRunAggregateV1,
  entry: Extract<WorkflowTaskPlanEntry, { kind: 'planner' }>
): CreateCreativeTaskInput {
  if (run.workflowSnapshot.output.kind !== 'multi-image-series') {
    fail('planner task requires a multi-image output plan');
  }
  const reference = workflowTaskReference(run, entry);
  const { taskId, ...identity } = reference;
  const rendered = renderWorkflowTemplate(
    run.workflowSnapshot,
    entry.step.templateId,
    run.request.inputs
  );
  if (!rendered.ok) fail(`${rendered.error.path}: ${rendered.error.message}`);
  const count = run.workflowSnapshot.output.targetCount;
  const parameters: CreativeJsonObject = {
    system: plannerSystem(entry.step.planning.instruction, count),
    prompt: [
      `Create ${count} production-ready image prompts from the following brief.`,
      '<brief>',
      rendered.value,
      '</brief>',
    ].join('\n'),
    max_tokens: entry.step.planning.maxTokens,
  };
  return {
    ...identity,
    idempotencyKey: taskId,
    parameters,
    inputs: [],
  };
}

function inputAssets(inputs: readonly WorkflowInputValue[], variableIds: readonly string[]): string[] {
  const assets: string[] = [];
  for (const variableId of variableIds) {
    const input = inputs.find((candidate) => candidate.variableId === variableId);
    if (input?.type === 'image' && input.assetId) assets.push(input.assetId);
    if (input?.type === 'image-series') assets.push(...input.assetIds);
  }
  return assets;
}

export function imageReferenceAssetIds(
  run: WorkflowRunAggregateV1,
  step: WorkflowGenerateImagesStep
): string[] {
  return [...new Set([
    ...run.request.referenceAssetIds,
    ...inputAssets(run.request.inputs, step.referenceVariableIds),
  ])];
}

function imagePrompt(
  run: WorkflowRunAggregateV1,
  entry: Extract<WorkflowTaskPlanEntry, { kind: 'image' }>
): string {
  if (entry.step.promptSource.kind === 'template') {
    const rendered = renderWorkflowTemplate(
      run.workflowSnapshot,
      entry.step.promptSource.templateId,
      run.request.inputs
    );
    if (!rendered.ok) fail(`${rendered.error.path}: ${rendered.error.message}`);
    return rendered.value;
  }
  const draft = run.promptDrafts.find((candidate) => candidate.seriesIndex === entry.seriesIndex);
  if (!draft || draft.status !== 'approved') fail('image task has no approved prompt draft');
  return draft.prompt;
}

function greatestCommonDivisor(left: number, right: number): number {
  let a = Math.abs(left);
  let b = Math.abs(right);
  while (b > 0) [a, b] = [b, a % b];
  return a || 1;
}

export function createImageTaskInput(
  run: WorkflowRunAggregateV1,
  entry: Extract<WorkflowTaskPlanEntry, { kind: 'image' }>
): CreateCreativeTaskInput {
  const reference = workflowTaskReference(run, entry);
  const { taskId, ...identity } = reference;
  const assetIds = imageReferenceAssetIds(run, entry.step);
  if (reference.capability === 't2i' && assetIds.length > 0) {
    fail(`t2i step ${entry.step.id} cannot receive reference assets`);
  }
  if (reference.capability === 'i2i' && assetIds.length === 0) {
    fail(`i2i step ${entry.step.id} requires at least one reference asset`);
  }
  const { width, height } = entry.step.generation;
  const divisor = greatestCommonDivisor(width, height);
  const parameters: CreativeJsonObject = {
    prompt: imagePrompt(run, entry),
    interface_mode: 'images',
    quality: entry.step.generation.quality,
    aspect: `${width / divisor}:${height / divisor}`,
    count: entry.step.generation.imagesPerPrompt,
    width,
    height,
  };
  const inputs: CreativeTaskInput[] = assetIds.map((assetId) => ({
    assetId,
    role: 'reference',
  }));
  return {
    ...identity,
    idempotencyKey: taskId,
    parameters,
    inputs,
  };
}

function exactObject(value: unknown, keys: readonly string[], label: string): Record<string, unknown> {
  if (!value || typeof value !== 'object' || Array.isArray(value)) {
    throw new WorkflowRunRuntimeError('planner-output', `${label} must be an object`);
  }
  const record = value as Record<string, unknown>;
  const unknown = Object.keys(record).find((key) => !keys.includes(key));
  const missing = keys.find((key) => !Object.hasOwn(record, key));
  if (unknown || missing) {
    throw new WorkflowRunRuntimeError(
      'planner-output',
      `${label} ${unknown ? `contains unknown field ${unknown}` : `is missing ${missing}`}`
    );
  }
  return record;
}

function boundedText(value: unknown, maximum: number, label: string): string {
  if (
    typeof value !== 'string'
    || value.trim().length === 0
    || value.length > maximum
    || /[\u0000-\u0008\u000b\u000c\u000e-\u001f\u007f]/u.test(value)
  ) {
    throw new WorkflowRunRuntimeError('planner-output', `${label} must contain bounded text`);
  }
  return value;
}

export function parsePlannerPromptDrafts(
  text: string,
  run: WorkflowRunAggregateV1,
  createId: () => string,
  now: () => number
): WorkflowPromptDraft[] {
  if (run.workflowSnapshot.output.kind !== 'multi-image-series') {
    fail('planner output requires a multi-image workflow');
  }
  let parsed: unknown;
  try {
    parsed = JSON.parse(text);
  } catch {
    throw new WorkflowRunRuntimeError('planner-output', 'planner returned invalid JSON');
  }
  const root = exactObject(parsed, ['prompts'], 'planner output');
  const count = run.workflowSnapshot.output.targetCount;
  if (!Array.isArray(root.prompts) || root.prompts.length !== count) {
    throw new WorkflowRunRuntimeError(
      'planner-output',
      `planner must return exactly ${count} prompts`
    );
  }
  const createdAt = now();
  const reviewRequired = run.workflowSnapshot.output.reviewRequired;
  return root.prompts.map((value, seriesIndex) => {
    const prompt = exactObject(value, ['title', 'prompt'], `prompts[${seriesIndex}]`);
    return {
      id: createId(),
      workflowId: run.request.workflowId,
      runRequestId: run.request.id,
      seriesIndex,
      title: boundedText(prompt.title, 120, `prompts[${seriesIndex}].title`),
      prompt: boundedText(prompt.prompt, 200_000, `prompts[${seriesIndex}].prompt`),
      status: reviewRequired ? 'pending-review' : 'approved',
      createdAt,
      reviewedAt: reviewRequired ? null : createdAt,
      reviewNote: null,
    };
  });
}
