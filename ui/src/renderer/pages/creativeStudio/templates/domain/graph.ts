/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import type {
  CreativeTemplateDefinitionV1,
  CreativeTemplateStep,
  CreativeTemplateValidationError,
  CreativeTemplateValueResult,
} from './types';

function broken(path: string, message: string): CreativeTemplateValueResult<CreativeTemplateStep[]> {
  return { ok: false, error: { code: 'broken-reference', path, message } };
}

export function topologicallySortTemplateSteps(
  template: CreativeTemplateDefinitionV1
): CreativeTemplateValueResult<CreativeTemplateStep[]> {
  const byId = new Map(template.steps.map((step) => [step.id, step]));
  if (byId.size !== template.steps.length) {
    return {
      ok: false,
      error: { code: 'duplicate-id', path: '$.steps', message: 'step IDs must be unique' },
    };
  }
  const followers = new Map<string, string[]>();
  const indegree = new Map<string, number>();
  for (const step of template.steps) {
    indegree.set(step.id, step.dependsOn.length);
    for (const dependencyId of step.dependsOn) {
      if (!byId.has(dependencyId)) {
        return broken(`$.steps[${template.steps.indexOf(step)}].dependsOn`, 'dependency does not exist');
      }
      followers.set(dependencyId, [...(followers.get(dependencyId) ?? []), step.id]);
    }
  }
  const queue = template.steps.filter((step) => indegree.get(step.id) === 0).map((step) => step.id);
  const ordered: CreativeTemplateStep[] = [];
  while (queue.length > 0) {
    const id = queue.shift();
    if (!id) break;
    const step = byId.get(id);
    if (!step) continue;
    ordered.push(step);
    for (const followerId of followers.get(id) ?? []) {
      const remaining = (indegree.get(followerId) ?? 0) - 1;
      indegree.set(followerId, remaining);
      if (remaining === 0) queue.push(followerId);
    }
  }
  if (ordered.length !== template.steps.length) {
    return {
      ok: false,
      error: { code: 'cycle-detected', path: '$.steps', message: 'template step graph must be acyclic' },
    };
  }
  return { ok: true, value: ordered };
}

export function validateTemplateGraph(template: CreativeTemplateDefinitionV1): CreativeTemplateValidationError | null {
  const variables = new Map(template.variables.map((variable) => [variable.id, variable]));
  const promptTemplates = new Map(
    template.templates.map((promptTemplate) => [promptTemplate.id, promptTemplate])
  );
  const steps = new Map(template.steps.map((step) => [step.id, step]));

  for (const [templateIndex, promptTemplate] of template.templates.entries()) {
    for (const [segmentIndex, segment] of promptTemplate.segments.entries()) {
      if (segment.kind !== 'variable') continue;
      const variable = variables.get(segment.variableId);
      if (!variable) {
        return {
          code: 'broken-reference',
          path: `$.templates[${templateIndex}].segments[${segmentIndex}].variableId`,
          message: 'template variable does not exist',
        };
      }
      if (variable.type === 'image' || variable.type === 'image-series') {
        return {
          code: 'invalid-value',
          path: `$.templates[${templateIndex}].segments[${segmentIndex}].variableId`,
          message: 'image inputs are task references and cannot be interpolated into prompt text',
        };
      }
    }
  }

  for (const [stepIndex, step] of template.steps.entries()) {
    if (new Set(step.dependsOn).size !== step.dependsOn.length || step.dependsOn.includes(step.id)) {
      return {
        code: 'invalid-value',
        path: `$.steps[${stepIndex}].dependsOn`,
        message: 'dependencies must be unique and cannot reference the step itself',
      };
    }
    if (step.kind === 'render-template' || step.kind === 'draft-prompts') {
      if (!promptTemplates.has(step.templateId)) {
        return {
          code: 'broken-reference',
          path: `$.steps[${stepIndex}].templateId`,
          message: 'template does not exist',
        };
      }
      if (step.kind === 'draft-prompts' && template.output.kind !== 'multi-image-series') {
        return {
          code: 'invalid-value',
          path: `$.steps[${stepIndex}].kind`,
          message: 'draft-prompts is only valid for a multi-image series',
        };
      }
    } else if (step.kind === 'generate-images') {
      if (new Set(step.referenceVariableIds).size !== step.referenceVariableIds.length) {
        return {
          code: 'duplicate-id',
          path: `$.steps[${stepIndex}].referenceVariableIds`,
          message: 'reference variables must be unique',
        };
      }
      for (const variableId of step.referenceVariableIds) {
        const variable = variables.get(variableId);
        if (!variable || (variable.type !== 'image' && variable.type !== 'image-series')) {
          return {
            code: 'broken-reference',
            path: `$.steps[${stepIndex}].referenceVariableIds`,
            message: 'reference variable must exist and contain image assets',
          };
        }
      }
      const expectedTask = step.referenceVariableIds.length > 0 ? 'image_edit' : 'image_generation';
      if (step.generation.model && step.generation.model.task !== expectedTask) {
        return {
          code: 'invalid-value',
          path: `$.steps[${stepIndex}].generation.model.task`,
          message: `generation model must use ${expectedTask}`,
        };
      }
      if (step.promptSource.kind === 'template') {
        if (!promptTemplates.has(step.promptSource.templateId)) {
          return {
            code: 'broken-reference',
            path: `$.steps[${stepIndex}].promptSource.templateId`,
            message: 'prompt template does not exist',
          };
        }
        if (template.output.kind === 'multi-image-series') {
          return {
            code: 'invalid-value',
            path: `$.steps[${stepIndex}].promptSource`,
            message: 'multi-image generation must consume reviewed prompt drafts',
          };
        }
      } else {
        const source = steps.get(step.promptSource.stepId);
        if (!source || source.kind !== 'draft-prompts' || !step.dependsOn.includes(source.id)) {
          return {
            code: 'broken-reference',
            path: `$.steps[${stepIndex}].promptSource.stepId`,
            message: 'prompt draft source must be a direct draft-prompts dependency',
          };
        }
        if (template.output.kind !== 'multi-image-series') {
          return {
            code: 'invalid-value',
            path: `$.steps[${stepIndex}].promptSource`,
            message: 'single-image generation must consume a template',
          };
        }
      }
    } else {
      if (step.sourceStepIds.length === 0 || !step.sourceStepIds.every((id) => step.dependsOn.includes(id))) {
        return {
          code: 'broken-reference',
          path: `$.steps[${stepIndex}].sourceStepIds`,
          message: 'history sources must be direct dependencies',
        };
      }
      if (
        step.sourceStepIds.some((id) => {
          const source = steps.get(id);
          return !source || source.kind !== 'generate-images';
        })
      ) {
        return {
          code: 'broken-reference',
          path: `$.steps[${stepIndex}].sourceStepIds`,
          message: 'history sources must be image-generation steps',
        };
      }
    }
  }

  const sorted = topologicallySortTemplateSteps(template);
  return sorted.ok ? null : sorted.error;
}
