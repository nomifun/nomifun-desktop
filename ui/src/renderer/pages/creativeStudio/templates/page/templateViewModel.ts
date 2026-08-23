/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { uuidv7 } from '@/common/utils/uuidv7';
import {
  cloneTemplateDefinition,
  type CreativeTemplateDefinitionV1,
  type CreativeTemplateDraftPromptsStep,
  type CreativeTemplateGenerateImagesStep,
  type CreativeTemplateImageGenerationSettings,
  type CreativeTemplateOutputPlan,
  type CreativePromptTemplateSegment,
  type CreativeTemplateVariable,
} from '../domain';
import {
  createTemplateTranslationCopy,
  type CreativeTemplateTranslationCopy,
} from '../templateI18n';

export type TemplateEditorMode = 'single-image' | 'multi-image-series';
export type TemplateVariableType = CreativeTemplateVariable['type'];

const DEFAULT_GENERATION: CreativeTemplateImageGenerationSettings = {
  model: null,
  quality: 'auto',
  width: 1024,
  height: 1024,
  imagesPerPrompt: 1,
};

const defaultPromptPlanning = (copy: CreativeTemplateTranslationCopy) => ({
  model: null,
  instruction: copy.planningInstruction,
  maxTokens: 4096,
});

const textVariable = (
  key: string,
  label: string,
  type: 'text' | 'multiline-text' = 'text'
): CreativeTemplateVariable => ({
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

export function createTemplateVariable(
  type: TemplateVariableType = 'text',
  ordinal = 1,
  copy: CreativeTemplateTranslationCopy = createTemplateTranslationCopy()
): CreativeTemplateVariable {
  const common = {
    id: uuidv7(),
    key: `input_${ordinal}`,
    label: copy.variableLabel(ordinal),
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
      return {
        ...common,
        type,
        defaultValue: null,
        options: [copy.choiceOptionOne, copy.choiceOptionTwo],
      };
    case 'image':
      return { ...common, type, defaultAssetId: null };
    case 'image-series':
      return { ...common, type, defaultAssetIds: [], minItems: 0, maxItems: 20 };
  }
}

function buildSteps(
  mode: TemplateEditorMode,
  templateId: string,
  generation: CreativeTemplateImageGenerationSettings,
  referenceVariableIds: string[] = [],
  copy: CreativeTemplateTranslationCopy = createTemplateTranslationCopy()
): CreativeTemplateDefinitionV1['steps'] {
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
        name: copy.stepGenerate,
        dependsOn: [],
        enabled: true,
        promptSource: { kind: 'template', templateId },
        referenceVariableIds: [...referenceVariableIds],
        generation: normalizedGeneration,
      },
      {
        id: historyId,
        kind: 'record-history',
        name: copy.stepRecord,
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
      name: copy.stepPlan,
      dependsOn: [],
      enabled: true,
      templateId,
      planning: { ...defaultPromptPlanning(copy) },
    },
    {
      id: generateId,
      kind: 'generate-images',
      name: copy.stepBatchGenerate,
      dependsOn: [draftId],
      enabled: true,
      promptSource: { kind: 'prompt-drafts', stepId: draftId },
      referenceVariableIds: [...referenceVariableIds],
      generation: normalizedGeneration,
    },
    {
      id: historyId,
      kind: 'record-history',
      name: copy.stepRecord,
      dependsOn: [generateId],
      enabled: true,
      sourceStepIds: [generateId],
    },
  ];
}

export function createBlankTemplate(
  mode: TemplateEditorMode,
  copy: CreativeTemplateTranslationCopy = createTemplateTranslationCopy()
): CreativeTemplateDefinitionV1 {
  const variables =
    mode === 'multi-image-series'
      ? [
          textVariable('topic', copy.topicLabel, 'multiline-text'),
          textVariable('style', copy.styleLabel),
          textVariable('platform', copy.platformLabel),
        ]
      : [
          textVariable('product_name', copy.productNameLabel),
          textVariable('selling_points', copy.sellingPointsLabel, 'multiline-text'),
        ];
  const templateId = uuidv7();
  return {
    id: uuidv7(),
    revision: 1,
    metadata: {
      name: mode === 'multi-image-series' ? copy.multiName : copy.singleName,
      description:
        mode === 'multi-image-series'
          ? copy.multiDescription
          : '',
      category: mode === 'multi-image-series' ? copy.multiCategory : '',
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
        name: copy.templateName,
        segments: parseCreativePromptTemplateText(
          mode === 'multi-image-series'
            ? copy.multiPromptTemplate
            : copy.singlePromptTemplate,
          variables
        ),
      },
    ],
    steps: buildSteps(mode, templateId, DEFAULT_GENERATION, [], copy),
  };
}

export function withPrivateTemplateVisibility(
  template: CreativeTemplateDefinitionV1
): CreativeTemplateDefinitionV1 {
  return {
    ...template,
    metadata: {
      ...template.metadata,
      visibility: 'private',
    },
  };
}

export function templateMode(template: CreativeTemplateDefinitionV1): TemplateEditorMode {
  return template.output.kind;
}

export function generationStep(
  template: CreativeTemplateDefinitionV1
): CreativeTemplateGenerateImagesStep {
  const step = template.steps.find(
    (candidate): candidate is CreativeTemplateGenerateImagesStep => candidate.kind === 'generate-images'
  );
  if (!step) throw new Error('template is missing its image-generation step');
  return step;
}

export function draftPromptsStep(
  template: CreativeTemplateDefinitionV1
): CreativeTemplateDraftPromptsStep | null {
  return template.steps.find(
    (candidate): candidate is CreativeTemplateDraftPromptsStep => candidate.kind === 'draft-prompts'
  ) ?? null;
}

export function creativePromptTemplateText(template: CreativeTemplateDefinitionV1): string {
  const variables = new Map(template.variables.map((variable) => [variable.id, variable.key]));
  return (template.templates[0]?.segments ?? [])
    .map((segment) =>
      segment.kind === 'text'
        ? segment.text
        : `{{${variables.get(segment.variableId) ?? 'missing_variable'}}}`
    )
    .join('');
}

export function parseCreativePromptTemplateText(
  text: string,
  variables: readonly CreativeTemplateVariable[]
): CreativePromptTemplateSegment[] {
  const variablesByKey = new Map(
    variables
      .filter((variable) => variable.type !== 'image' && variable.type !== 'image-series')
      .map((variable) => [variable.key, variable])
  );
  const segments: CreativePromptTemplateSegment[] = [];
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

export function replaceCreativePromptTemplateText(
  template: CreativeTemplateDefinitionV1,
  text: string
): CreativeTemplateDefinitionV1 {
  const next = cloneTemplateDefinition(template);
  next.templates[0] = {
    ...next.templates[0],
    segments: parseCreativePromptTemplateText(text, next.variables),
  };
  return next;
}

export function switchTemplateMode(
  template: CreativeTemplateDefinitionV1,
  mode: TemplateEditorMode,
  copy: CreativeTemplateTranslationCopy = createTemplateTranslationCopy()
): CreativeTemplateDefinitionV1 {
  const next = cloneTemplateDefinition(template);
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
    currentGeneration.referenceVariableIds,
    copy
  );
  return next;
}

export function convertTemplateVariable(
  variable: CreativeTemplateVariable,
  type: TemplateVariableType,
  copy: CreativeTemplateTranslationCopy = createTemplateTranslationCopy()
): CreativeTemplateVariable {
  const converted = createTemplateVariable(type, 1, copy);
  return {
    ...converted,
    id: variable.id,
    key: variable.key,
    label: variable.label,
    description: variable.description,
    required: variable.required,
  } as CreativeTemplateVariable;
}

export function replaceTemplateVariable(
  template: CreativeTemplateDefinitionV1,
  variable: CreativeTemplateVariable
): CreativeTemplateDefinitionV1 {
  const next = cloneTemplateDefinition(template);
  next.variables = next.variables.map((candidate) =>
    candidate.id === variable.id ? structuredClone(variable) : candidate
  );
  const imageVariable = variable.type === 'image' || variable.type === 'image-series';
  if (imageVariable) {
    for (const promptTemplate of next.templates) {
      promptTemplate.segments = promptTemplate.segments.map((segment) =>
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

export function removeTemplateVariable(
  template: CreativeTemplateDefinitionV1,
  variableId: string
): CreativeTemplateDefinitionV1 {
  const variable = template.variables.find((candidate) => candidate.id === variableId);
  if (!variable) return cloneTemplateDefinition(template);
  const next = cloneTemplateDefinition(template);
  next.variables = next.variables.filter((candidate) => candidate.id !== variableId);
  for (const promptTemplate of next.templates) {
    promptTemplate.segments = promptTemplate.segments.map((segment) =>
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

export function setTemplateReferenceVariable(
  template: CreativeTemplateDefinitionV1,
  variableId: string,
  enabled: boolean
): CreativeTemplateDefinitionV1 {
  const next = cloneTemplateDefinition(template);
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

export function duplicateTemplate(
  template: CreativeTemplateDefinitionV1,
  copy: CreativeTemplateTranslationCopy = createTemplateTranslationCopy()
): CreativeTemplateDefinitionV1 {
  const next = cloneTemplateDefinition(template);
  const variableIds = new Map(next.variables.map((variable) => [variable.id, uuidv7()]));
  const promptTemplateIds = new Map(
    next.templates.map((promptTemplate) => [promptTemplate.id, uuidv7()])
  );
  const stepIds = new Map(next.steps.map((step) => [step.id, uuidv7()]));
  next.id = uuidv7();
  next.revision = 1;
  next.metadata = {
    ...next.metadata,
    name: `${next.metadata.name} (${copy.duplicateSuffix})`,
    visibility: 'private',
    createdAt: 0,
    updatedAt: 0,
  };
  next.variables = next.variables.map((variable) => ({
    ...variable,
    id: variableIds.get(variable.id)!,
  })) as CreativeTemplateVariable[];
  next.templates = next.templates.map((promptTemplate) => ({
    ...promptTemplate,
    id: promptTemplateIds.get(promptTemplate.id)!,
    segments: promptTemplate.segments.map((segment) =>
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
            templateId: promptTemplateIds.get(step.templateId)!,
            planning: {
              ...step.planning,
              model: step.planning.model ? { ...step.planning.model } : null,
            },
          }
        : { ...common, templateId: promptTemplateIds.get(step.templateId)! };
    }
    if (step.kind === 'generate-images') {
      return {
        ...common,
        promptSource:
          step.promptSource.kind === 'template'
            ? {
                kind: 'template' as const,
                templateId: promptTemplateIds.get(step.promptSource.templateId)!,
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

export function templatePromptPreview(
  template: CreativeTemplateDefinitionV1,
  copy: CreativeTemplateTranslationCopy = createTemplateTranslationCopy()
): string {
  return creativePromptTemplateText(template).trim() || copy.emptyPrompt;
}

export function templateOutputLabel(
  output: CreativeTemplateOutputPlan,
  copy: CreativeTemplateTranslationCopy = createTemplateTranslationCopy()
): string {
  return output.kind === 'single-image' ? copy.outputSingle : copy.outputMulti;
}
