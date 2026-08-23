/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import type {
  CreativeTemplateDefinitionV1,
  CreativeTemplateInputValue,
  CreativePromptTemplate,
  CreativeTemplateValidationError,
  CreativeTemplateValueResult,
  CreativeTemplateWorkspaceDocumentV1,
} from './types';
import {
  cloneTemplateOutput,
  cloneTemplateVariable,
  validateTemplateDefinition,
  validateTemplateInputsForDefinition,
} from './validation';

export function createTemplateWorkspaceDocumentV1(): CreativeTemplateWorkspaceDocumentV1 {
  return {
    kind: 'nomifun.creative-studio.templates',
    version: 1,
    templates: [],
    promptDrafts: [],
    runRequests: [],
    runs: [],
  };
}

export function cloneTemplateDefinition(template: CreativeTemplateDefinitionV1): CreativeTemplateDefinitionV1 {
  return {
    id: template.id,
    revision: template.revision,
    metadata: { ...template.metadata, tags: [...template.metadata.tags] },
    output: cloneTemplateOutput(template.output),
    variables: template.variables.map(cloneTemplateVariable),
    templates: template.templates.map((promptTemplate) => ({
      ...promptTemplate,
      segments: promptTemplate.segments.map((segment) => ({ ...segment })),
    })),
    steps: template.steps.map((step) => {
      if (step.kind === 'generate-images') {
        return {
          ...step,
          dependsOn: [...step.dependsOn],
          promptSource: { ...step.promptSource },
          referenceVariableIds: [...step.referenceVariableIds],
          generation: {
            ...step.generation,
            model: step.generation.model ? { ...step.generation.model } : null,
          },
        };
      }
      if (step.kind === 'record-history') {
        return { ...step, dependsOn: [...step.dependsOn], sourceStepIds: [...step.sourceStepIds] };
      }
      if (step.kind === 'draft-prompts') {
        return {
          ...step,
          dependsOn: [...step.dependsOn],
          planning: {
            ...step.planning,
            model: step.planning.model ? { ...step.planning.model } : null,
          },
        };
      }
      return { ...step, dependsOn: [...step.dependsOn] };
    }),
  };
}

export function createTemplateDefinitionV1(
  input: CreativeTemplateDefinitionV1
): CreativeTemplateValueResult<CreativeTemplateDefinitionV1> {
  const result = validateTemplateDefinition(input);
  return result.ok ? { ok: true, value: cloneTemplateDefinition(input) } : result;
}

export function createTemplateDefaultInputs(template: CreativeTemplateDefinitionV1): CreativeTemplateInputValue[] {
  const inputs: CreativeTemplateInputValue[] = [];
  for (const variable of template.variables) {
    if (variable.type === 'text' || variable.type === 'multiline-text') {
      if (variable.defaultValue !== null) {
        inputs.push({ variableId: variable.id, type: variable.type, value: variable.defaultValue });
      }
    } else if (variable.type === 'number') {
      if (variable.defaultValue !== null) {
        inputs.push({ variableId: variable.id, type: 'number', value: variable.defaultValue });
      }
    } else if (variable.type === 'boolean') {
      inputs.push({ variableId: variable.id, type: 'boolean', value: variable.defaultValue });
    } else if (variable.type === 'choice') {
      if (variable.defaultValue !== null) {
        inputs.push({ variableId: variable.id, type: 'choice', value: variable.defaultValue });
      }
    } else if (variable.type === 'image') {
      if (variable.defaultAssetId !== null) {
        inputs.push({ variableId: variable.id, type: 'image', assetId: variable.defaultAssetId });
      }
    } else if (variable.type === 'image-series' && variable.defaultAssetIds.length > 0) {
      inputs.push({
        variableId: variable.id,
        type: 'image-series',
        assetIds: [...variable.defaultAssetIds],
      });
    }
  }
  return inputs;
}

function valueText(input: CreativeTemplateInputValue): string | null {
  if (input.type === 'text' || input.type === 'multiline-text' || input.type === 'choice') {
    return input.value;
  }
  if (input.type === 'number') return String(input.value);
  if (input.type === 'boolean') return input.value ? 'true' : 'false';
  return null;
}

export function renderCreativePromptTemplate(
  definition: CreativeTemplateDefinitionV1,
  promptTemplateId: string,
  inputs: CreativeTemplateInputValue[]
): CreativeTemplateValueResult<string> {
  const validation = validateTemplateInputsForDefinition(definition, inputs);
  if (!validation.ok) return validation;
  const promptTemplate: CreativePromptTemplate | undefined = definition.templates.find(
    (item) => item.id === promptTemplateId
  );
  if (!promptTemplate) {
    const error: CreativeTemplateValidationError = {
      code: 'broken-reference',
      path: '$.promptTemplateId',
      message: 'prompt template does not exist',
    };
    return { ok: false, error };
  }
  let rendered = '';
  for (const segment of promptTemplate.segments) {
    if (segment.kind === 'text') {
      rendered += segment.text;
      continue;
    }
    const input = inputs.find((item) => item.variableId === segment.variableId);
    if (!input) {
      const variable = definition.variables.find((item) => item.id === segment.variableId);
      if (variable && !variable.required) continue;
      return {
        ok: false,
        error: {
          code: 'invalid-value',
          path: '$.inputs',
          message: 'template input is missing',
        },
      };
    }
    const formatted = valueText(input);
    if (formatted === null) {
      return {
        ok: false,
        error: {
          code: 'invalid-value',
          path: '$.inputs',
          message: 'image references cannot be rendered as prompt text',
        },
      };
    }
    rendered += formatted;
  }
  return { ok: true, value: rendered };
}

export function findTemplate(
  document: CreativeTemplateWorkspaceDocumentV1,
  templateId: string
): CreativeTemplateDefinitionV1 | undefined {
  return document.templates.find((template) => template.id === templateId);
}
