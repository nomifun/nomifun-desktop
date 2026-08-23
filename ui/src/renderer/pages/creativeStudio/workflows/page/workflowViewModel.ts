/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { uuidv7 } from '@/common/utils/uuidv7';
import {
  cloneWorkflowDefinition,
  type WorkflowDefinitionV1,
  type WorkflowDraftPromptsStep,
  type WorkflowGenerateImagesStep,
  type WorkflowImageGenerationSettings,
  type WorkflowOutputPlan,
  type WorkflowTemplateSegment,
  type WorkflowVariable,
} from '../domain';

export type WorkflowEditorMode = 'single-image' | 'multi-image-series';
export type WorkflowVariableType = WorkflowVariable['type'];

const DEFAULT_GENERATION: WorkflowImageGenerationSettings = {
  model: null,
  quality: 'auto',
  width: 1024,
  height: 1024,
  imagesPerPrompt: 1,
};

const DEFAULT_PROMPT_PLANNING = {
  model: null,
  instruction: '让每张图片承担不同的叙事作用，同时保持统一的主体、视觉风格与发布语境。',
  maxTokens: 4096,
} as const;

const textVariable = (
  key: string,
  label: string,
  type: 'text' | 'multiline-text' = 'text'
): WorkflowVariable => ({
  id: uuidv7(),
  key,
  label,
  description: '',
  required: true,
  type,
  defaultValue: null,
  placeholder: '',
  minLength: 0,
  maxLength: 20_000,
});

export function createWorkflowVariable(
  type: WorkflowVariableType = 'text',
  ordinal = 1
): WorkflowVariable {
  const common = {
    id: uuidv7(),
    key: `input_${ordinal}`,
    label: `输入 ${ordinal}`,
    description: '',
    required: false,
  };
  switch (type) {
    case 'text':
    case 'multiline-text':
      return {
        ...common,
        type,
        defaultValue: null,
        placeholder: '',
        minLength: 0,
        maxLength: 20_000,
      };
    case 'number':
      return {
        ...common,
        type,
        defaultValue: null,
        minimum: null,
        maximum: null,
        step: null,
      };
    case 'boolean':
      return { ...common, type, defaultValue: false };
    case 'choice':
      return { ...common, type, defaultValue: null, options: ['选项一', '选项二'] };
    case 'image':
      return { ...common, type, defaultAssetId: null };
    case 'image-series':
      return { ...common, type, defaultAssetIds: [], minItems: 0, maxItems: 20 };
  }
}

function buildSteps(
  mode: WorkflowEditorMode,
  templateId: string,
  generation: WorkflowImageGenerationSettings,
  referenceVariableIds: string[] = []
): WorkflowDefinitionV1['steps'] {
  const expectedTask = referenceVariableIds.length > 0 ? 'image_edit' : 'image_generation';
  const normalizedGeneration = {
    ...generation,
    model:
      generation.model?.task === expectedTask
        ? { ...generation.model }
        : null,
  };
  const generateId = uuidv7();
  const historyId = uuidv7();
  if (mode === 'single-image') {
    return [
      {
        id: generateId,
        kind: 'generate-images',
        name: '生成图片',
        dependsOn: [],
        enabled: true,
        promptSource: { kind: 'template', templateId },
        referenceVariableIds: [...referenceVariableIds],
        generation: normalizedGeneration,
      },
      {
        id: historyId,
        kind: 'record-history',
        name: '记录结果',
        dependsOn: [generateId],
        enabled: true,
        sourceStepIds: [generateId],
      },
    ];
  }
  const draftId = uuidv7();
  return [
    {
      id: draftId,
      kind: 'draft-prompts',
      name: '规划多图提示词',
      dependsOn: [],
      enabled: true,
      templateId,
      planning: { ...DEFAULT_PROMPT_PLANNING },
    },
    {
      id: generateId,
      kind: 'generate-images',
      name: '批量生成图片',
      dependsOn: [draftId],
      enabled: true,
      promptSource: { kind: 'prompt-drafts', stepId: draftId },
      referenceVariableIds: [...referenceVariableIds],
      generation: normalizedGeneration,
    },
    {
      id: historyId,
      kind: 'record-history',
      name: '记录结果',
      dependsOn: [generateId],
      enabled: true,
      sourceStepIds: [generateId],
    },
  ];
}

const starterPrompt = (mode: WorkflowEditorMode, variables: WorkflowVariable[]) => {
  if (mode === 'multi-image-series') {
    return [
      { kind: 'text' as const, text: '围绕 ' },
      { kind: 'variable' as const, variableId: variables[0].id },
      { kind: 'text' as const, text: ' 生成一组连贯配图。\n统一风格：' },
      { kind: 'variable' as const, variableId: variables[1].id },
      { kind: 'text' as const, text: '\n发布平台：' },
      { kind: 'variable' as const, variableId: variables[2].id },
    ];
  }
  return [
    { kind: 'text' as const, text: '为 ' },
    { kind: 'variable' as const, variableId: variables[0].id },
    { kind: 'text' as const, text: ' 生成一张高端电商海报。\n核心卖点：' },
    { kind: 'variable' as const, variableId: variables[1].id },
  ];
};

export function createBlankWorkflow(mode: WorkflowEditorMode): WorkflowDefinitionV1 {
  const variables =
    mode === 'multi-image-series'
      ? [
          textVariable('topic', '主题', 'multiline-text'),
          textVariable('style', '统一风格'),
          textVariable('platform', '发布平台'),
        ]
      : [
          textVariable('product_name', '产品名称'),
          textVariable('selling_points', '产品卖点', 'multiline-text'),
        ];
  const templateId = uuidv7();
  return {
    id: uuidv7(),
    revision: 1,
    metadata: {
      name: mode === 'multi-image-series' ? '多图系列生成' : '未命名模板',
      description:
        mode === 'multi-image-series'
          ? '根据主题生成一组连贯图片提示词，审核后批量生成图片。'
          : '',
      category: mode === 'multi-image-series' ? '多图创作' : '',
      visibility: 'private',
      tags: [],
      createdAt: 0,
      updatedAt: 0,
    },
    output:
      mode === 'single-image'
        ? { kind: 'single-image' }
        : {
            kind: 'multi-image-series',
            targetCount: 6,
            concurrency: 3,
            reviewRequired: true,
          },
    variables,
    templates: [
      {
        id: templateId,
        name: '主提示词',
        segments: starterPrompt(mode, variables),
      },
    ],
    steps: buildSteps(mode, templateId, DEFAULT_GENERATION),
  };
}

export function withPrivateWorkflowVisibility(
  workflow: WorkflowDefinitionV1
): WorkflowDefinitionV1 {
  return {
    ...workflow,
    metadata: {
      ...workflow.metadata,
      visibility: 'private',
    },
  };
}

export function workflowMode(workflow: WorkflowDefinitionV1): WorkflowEditorMode {
  return workflow.output.kind;
}

export function generationStep(
  workflow: WorkflowDefinitionV1
): WorkflowGenerateImagesStep {
  const step = workflow.steps.find(
    (candidate): candidate is WorkflowGenerateImagesStep => candidate.kind === 'generate-images'
  );
  if (!step) throw new Error('workflow is missing its image-generation step');
  return step;
}

export function draftPromptsStep(
  workflow: WorkflowDefinitionV1
): WorkflowDraftPromptsStep | null {
  return workflow.steps.find(
    (candidate): candidate is WorkflowDraftPromptsStep => candidate.kind === 'draft-prompts'
  ) ?? null;
}

export function workflowTemplateText(workflow: WorkflowDefinitionV1): string {
  const variables = new Map(workflow.variables.map((variable) => [variable.id, variable.key]));
  return (workflow.templates[0]?.segments ?? [])
    .map((segment) =>
      segment.kind === 'text'
        ? segment.text
        : `{{${variables.get(segment.variableId) ?? 'missing_variable'}}}`
    )
    .join('');
}

export function parseWorkflowTemplateText(
  text: string,
  variables: readonly WorkflowVariable[]
): WorkflowTemplateSegment[] {
  const variablesByKey = new Map(
    variables
      .filter((variable) => variable.type !== 'image' && variable.type !== 'image-series')
      .map((variable) => [variable.key, variable])
  );
  const segments: WorkflowTemplateSegment[] = [];
  const expression = /\{\{\s*([a-z][a-z0-9_]{0,63})\s*\}\}/gu;
  let cursor = 0;
  for (const match of text.matchAll(expression)) {
    const index = match.index ?? cursor;
    if (index > cursor) segments.push({ kind: 'text', text: text.slice(cursor, index) });
    const variable = variablesByKey.get(match[1]);
    segments.push(
      variable
        ? { kind: 'variable', variableId: variable.id }
        : { kind: 'text', text: match[0] }
    );
    cursor = index + match[0].length;
  }
  if (cursor < text.length) segments.push({ kind: 'text', text: text.slice(cursor) });
  return segments.length > 0 ? segments : [{ kind: 'text', text: '' }];
}

export function replaceWorkflowTemplateText(
  workflow: WorkflowDefinitionV1,
  text: string
): WorkflowDefinitionV1 {
  const next = cloneWorkflowDefinition(workflow);
  next.templates[0] = {
    ...next.templates[0],
    segments: parseWorkflowTemplateText(text, next.variables),
  };
  return next;
}

export function switchWorkflowMode(
  workflow: WorkflowDefinitionV1,
  mode: WorkflowEditorMode
): WorkflowDefinitionV1 {
  const next = cloneWorkflowDefinition(workflow);
  const currentGeneration = generationStep(next);
  next.output =
    mode === 'single-image'
      ? { kind: 'single-image' }
      : {
          kind: 'multi-image-series',
          targetCount: 6,
          concurrency: 3,
          reviewRequired: true,
        };
  next.steps = buildSteps(
    mode,
    next.templates[0].id,
    currentGeneration.generation,
    currentGeneration.referenceVariableIds
  );
  return next;
}

export function convertWorkflowVariable(
  variable: WorkflowVariable,
  type: WorkflowVariableType
): WorkflowVariable {
  const converted = createWorkflowVariable(type);
  return {
    ...converted,
    id: variable.id,
    key: variable.key,
    label: variable.label,
    description: variable.description,
    required: variable.required,
  } as WorkflowVariable;
}

export function replaceWorkflowVariable(
  workflow: WorkflowDefinitionV1,
  variable: WorkflowVariable
): WorkflowDefinitionV1 {
  const next = cloneWorkflowDefinition(workflow);
  next.variables = next.variables.map((candidate) =>
    candidate.id === variable.id ? structuredClone(variable) : candidate
  );
  const imageVariable = variable.type === 'image' || variable.type === 'image-series';
  if (imageVariable) {
    for (const template of next.templates) {
      template.segments = template.segments.map((segment) =>
        segment.kind === 'variable' && segment.variableId === variable.id
          ? { kind: 'text', text: `{{${variable.key}}}` }
          : segment
      );
    }
  }
  const step = generationStep(next);
  if (!imageVariable && step.referenceVariableIds.includes(variable.id)) {
    step.referenceVariableIds = step.referenceVariableIds.filter((id) => id !== variable.id);
    if (step.generation.model?.task !== 'image_generation') step.generation.model = null;
  }
  return next;
}

export function removeWorkflowVariable(
  workflow: WorkflowDefinitionV1,
  variableId: string
): WorkflowDefinitionV1 {
  const variable = workflow.variables.find((candidate) => candidate.id === variableId);
  if (!variable) return cloneWorkflowDefinition(workflow);
  const next = cloneWorkflowDefinition(workflow);
  next.variables = next.variables.filter((candidate) => candidate.id !== variableId);
  for (const template of next.templates) {
    template.segments = template.segments.map((segment) =>
      segment.kind === 'variable' && segment.variableId === variableId
        ? { kind: 'text', text: `{{${variable.key}}}` }
        : segment
    );
  }
  const step = generationStep(next);
  step.referenceVariableIds = step.referenceVariableIds.filter((id) => id !== variableId);
  const expectedTask = step.referenceVariableIds.length > 0 ? 'image_edit' : 'image_generation';
  if (step.generation.model?.task !== expectedTask) step.generation.model = null;
  return next;
}

export function setWorkflowReferenceVariable(
  workflow: WorkflowDefinitionV1,
  variableId: string,
  enabled: boolean
): WorkflowDefinitionV1 {
  const next = cloneWorkflowDefinition(workflow);
  const variable = next.variables.find((candidate) => candidate.id === variableId);
  if (!variable || (variable.type !== 'image' && variable.type !== 'image-series')) return next;
  const step = generationStep(next);
  const ids = new Set(step.referenceVariableIds);
  if (enabled) ids.add(variableId);
  else ids.delete(variableId);
  step.referenceVariableIds = [...ids];
  const expectedTask = step.referenceVariableIds.length > 0 ? 'image_edit' : 'image_generation';
  if (step.generation.model?.task !== expectedTask) step.generation.model = null;
  return next;
}

export function duplicateWorkflow(workflow: WorkflowDefinitionV1): WorkflowDefinitionV1 {
  const next = cloneWorkflowDefinition(workflow);
  const variableIds = new Map(next.variables.map((variable) => [variable.id, uuidv7()]));
  const templateIds = new Map(next.templates.map((template) => [template.id, uuidv7()]));
  const stepIds = new Map(next.steps.map((step) => [step.id, uuidv7()]));
  next.id = uuidv7();
  next.revision = 1;
  next.metadata = {
    ...next.metadata,
    name: `${next.metadata.name} 副本`,
    visibility: 'private',
    createdAt: 0,
    updatedAt: 0,
  };
  next.variables = next.variables.map((variable) => ({
    ...variable,
    id: variableIds.get(variable.id)!,
  })) as WorkflowVariable[];
  next.templates = next.templates.map((template) => ({
    ...template,
    id: templateIds.get(template.id)!,
    segments: template.segments.map((segment) =>
      segment.kind === 'text'
        ? { ...segment }
        : { kind: 'variable', variableId: variableIds.get(segment.variableId)! }
    ),
  }));
  next.steps = next.steps.map((step) => {
    const common = {
      ...step,
      id: stepIds.get(step.id)!,
      dependsOn: step.dependsOn.map((id) => stepIds.get(id)!),
    };
    if (step.kind === 'render-template' || step.kind === 'draft-prompts') {
      return step.kind === 'draft-prompts'
        ? {
            ...common,
            templateId: templateIds.get(step.templateId)!,
            planning: {
              ...step.planning,
              model: step.planning.model ? { ...step.planning.model } : null,
            },
          }
        : { ...common, templateId: templateIds.get(step.templateId)! };
    }
    if (step.kind === 'generate-images') {
      return {
        ...common,
        promptSource:
          step.promptSource.kind === 'template'
            ? {
                kind: 'template' as const,
                templateId: templateIds.get(step.promptSource.templateId)!,
              }
            : {
                kind: 'prompt-drafts' as const,
                stepId: stepIds.get(step.promptSource.stepId)!,
              },
        referenceVariableIds: step.referenceVariableIds.map((id) => variableIds.get(id)!),
        generation: {
          ...step.generation,
          model: step.generation.model ? { ...step.generation.model } : null,
        },
      };
    }
    return {
      ...common,
      sourceStepIds: step.sourceStepIds.map((id) => stepIds.get(id)!),
    };
  });
  return next;
}

export function workflowPromptPreview(workflow: WorkflowDefinitionV1): string {
  return workflowTemplateText(workflow).trim() || '尚未填写提示词模板';
}

export function workflowOutputLabel(output: WorkflowOutputPlan): string {
  return output.kind === 'single-image' ? '单图' : '多图';
}
