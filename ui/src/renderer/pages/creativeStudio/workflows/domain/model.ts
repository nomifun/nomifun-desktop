/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import type {
  WorkflowDefinitionV1,
  WorkflowInputValue,
  WorkflowTemplate,
  WorkflowValidationError,
  WorkflowValueResult,
  WorkflowWorkspaceDocumentV1,
} from './types';
import {
  cloneWorkflowOutput,
  cloneWorkflowVariable,
  validateWorkflowDefinition,
  validateWorkflowInputsForDefinition,
} from './validation';

export function createWorkflowWorkspaceDocumentV1(): WorkflowWorkspaceDocumentV1 {
  return {
    kind: 'nomifun.creative-studio.workflows',
    version: 1,
    workflows: [],
    promptDrafts: [],
    runRequests: [],
    runs: [],
  };
}

export function cloneWorkflowDefinition(workflow: WorkflowDefinitionV1): WorkflowDefinitionV1 {
  return {
    id: workflow.id,
    revision: workflow.revision,
    metadata: { ...workflow.metadata, tags: [...workflow.metadata.tags] },
    output: cloneWorkflowOutput(workflow.output),
    variables: workflow.variables.map(cloneWorkflowVariable),
    templates: workflow.templates.map((template) => ({
      ...template,
      segments: template.segments.map((segment) => ({ ...segment })),
    })),
    steps: workflow.steps.map((step) => {
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

export function createWorkflowDefinitionV1(
  input: WorkflowDefinitionV1
): WorkflowValueResult<WorkflowDefinitionV1> {
  const result = validateWorkflowDefinition(input);
  return result.ok ? { ok: true, value: cloneWorkflowDefinition(input) } : result;
}

export function createWorkflowDefaultInputs(workflow: WorkflowDefinitionV1): WorkflowInputValue[] {
  const inputs: WorkflowInputValue[] = [];
  for (const variable of workflow.variables) {
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

function valueText(input: WorkflowInputValue): string | null {
  if (input.type === 'text' || input.type === 'multiline-text' || input.type === 'choice') {
    return input.value;
  }
  if (input.type === 'number') return String(input.value);
  if (input.type === 'boolean') return input.value ? 'true' : 'false';
  return null;
}

export function renderWorkflowTemplate(
  workflow: WorkflowDefinitionV1,
  templateId: string,
  inputs: WorkflowInputValue[]
): WorkflowValueResult<string> {
  const validation = validateWorkflowInputsForDefinition(workflow, inputs);
  if (!validation.ok) return validation;
  const template: WorkflowTemplate | undefined = workflow.templates.find(
    (item) => item.id === templateId
  );
  if (!template) {
    const error: WorkflowValidationError = {
      code: 'broken-reference',
      path: '$.templateId',
      message: 'template does not exist',
    };
    return { ok: false, error };
  }
  let rendered = '';
  for (const segment of template.segments) {
    if (segment.kind === 'text') {
      rendered += segment.text;
      continue;
    }
    const input = inputs.find((item) => item.variableId === segment.variableId);
    if (!input) {
      const variable = workflow.variables.find((item) => item.id === segment.variableId);
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

export function findWorkflow(
  document: WorkflowWorkspaceDocumentV1,
  workflowId: string
): WorkflowDefinitionV1 | undefined {
  return document.workflows.find((workflow) => workflow.id === workflowId);
}
