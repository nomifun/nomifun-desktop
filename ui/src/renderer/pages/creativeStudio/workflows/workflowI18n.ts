/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import type { TFunction } from 'i18next';

import type { WorkflowValidationError } from './domain';
import type { WorkflowRunRuntimeErrorCode } from './runtime';

export interface WorkflowTranslationCopy {
  planningInstruction: string;
  variableLabel(ordinal: number): string;
  topicLabel: string;
  styleLabel: string;
  platformLabel: string;
  productNameLabel: string;
  sellingPointsLabel: string;
  choiceOptionOne: string;
  choiceOptionTwo: string;
  stepGenerate: string;
  stepRecord: string;
  stepPlan: string;
  stepBatchGenerate: string;
  templateName: string;
  singleName: string;
  multiName: string;
  multiDescription: string;
  multiCategory: string;
  singlePromptTemplate: string;
  multiPromptTemplate: string;
  duplicateSuffix: string;
  emptyPrompt: string;
  outputSingle: string;
  outputMulti: string;
}

type WorkflowTranslate = (
  key: string,
  options?: Record<string, unknown>
) => string;

const fallbackTranslate: WorkflowTranslate = (_key, options) =>
  typeof options?.defaultValue === 'string' ? options.defaultValue : '';

const translate = (
  t: WorkflowTranslate,
  key: string,
  defaultValue: string,
  options: Record<string, unknown> = {}
): string => t(key, { ...options, defaultValue });

/**
 * The workflow domain stores display text in persisted definitions. Keep the
 * domain pure by resolving system-owned defaults at the UI boundary.
 */
export function createWorkflowTranslationCopy(
  t: WorkflowTranslate = fallbackTranslate
): WorkflowTranslationCopy {
  return {
    planningInstruction: translate(
      t,
      'creativeStudio.workflows.defaults.planningInstruction',
      'Give each image a distinct narrative role while keeping the subject, visual style, and publishing context consistent.'
    ),
    variableLabel: (ordinal) =>
      translate(t, 'creativeStudio.workflows.defaults.variableLabel', 'Input {{ordinal}}', {
        ordinal,
      }),
    topicLabel: translate(t, 'creativeStudio.workflows.defaults.topicLabel', 'Topic'),
    styleLabel: translate(t, 'creativeStudio.workflows.defaults.styleLabel', 'Unified style'),
    platformLabel: translate(
      t,
      'creativeStudio.workflows.defaults.platformLabel',
      'Publishing platform'
    ),
    productNameLabel: translate(
      t,
      'creativeStudio.workflows.defaults.productNameLabel',
      'Product name'
    ),
    sellingPointsLabel: translate(
      t,
      'creativeStudio.workflows.defaults.sellingPointsLabel',
      'Selling points',
    ),
    choiceOptionOne: translate(
      t,
      'creativeStudio.workflows.defaults.choiceOptionOne',
      'Option one'
    ),
    choiceOptionTwo: translate(
      t,
      'creativeStudio.workflows.defaults.choiceOptionTwo',
      'Option two'
    ),
    stepGenerate: translate(
      t,
      'creativeStudio.workflows.defaults.stepGenerate',
      'Generate images'
    ),
    stepRecord: translate(
      t,
      'creativeStudio.workflows.defaults.stepRecord',
      'Record results'
    ),
    stepPlan: translate(
      t,
      'creativeStudio.workflows.defaults.stepPlan',
      'Plan multi-image prompts'
    ),
    stepBatchGenerate: translate(
      t,
      'creativeStudio.workflows.defaults.stepBatchGenerate',
      'Generate images in batch'
    ),
    templateName: translate(
      t,
      'creativeStudio.workflows.defaults.templateName',
      'Main prompt'
    ),
    singleName: translate(
      t,
      'creativeStudio.workflows.defaults.singleName',
      'Untitled template'
    ),
    multiName: translate(
      t,
      'creativeStudio.workflows.defaults.multiName',
      'Image series generator'
    ),
    multiDescription: translate(
      t,
      'creativeStudio.workflows.defaults.multiDescription',
      'Generate a coherent set of image prompts from a topic, review them, then create the images in batch.'
    ),
    multiCategory: translate(
      t,
      'creativeStudio.workflows.defaults.multiCategory',
      'Multi-image creation'
    ),
    singlePromptTemplate: translate(
      t,
      'creativeStudio.workflows.defaults.singlePromptTemplate',
      'Create a premium e-commerce poster for {{product_name}}.\nKey selling points: {{selling_points}}'
    ),
    multiPromptTemplate: translate(
      t,
      'creativeStudio.workflows.defaults.multiPromptTemplate',
      'Create a coherent image series about {{topic}}.\nUnified style: {{style}}\nPublishing platform: {{platform}}'
    ),
    duplicateSuffix: translate(
      t,
      'creativeStudio.workflows.defaults.duplicateSuffix',
      'Copy'
    ),
    emptyPrompt: translate(
      t,
      'creativeStudio.workflows.defaults.emptyPrompt',
      'Prompt template not filled in yet'
    ),
    outputSingle: translate(
      t,
      'creativeStudio.workflows.output.single',
      'Single image'
    ),
    outputMulti: translate(
      t,
      'creativeStudio.workflows.output.multi',
      'Multi-image series'
    ),
  };
}

const validationDetail = (
  error: WorkflowValidationError,
  t: WorkflowTranslate
): string => {
  const required = /^required input (.+) is missing$/u.exec(error.message);
  if (required) {
    return translate(
      t,
      'creativeStudio.workflows.validation.requiredInput',
      'Please complete the required input “{{input}}”.',
      { input: required[1] }
    );
  }

  switch (error.code) {
    case 'invalid-json':
      return translate(
        t,
        'creativeStudio.workflows.validation.invalidJson',
        'The template data is not valid JSON.'
      );
    case 'invalid-envelope':
      return translate(
        t,
        'creativeStudio.workflows.validation.invalidEnvelope',
        'The template data format is invalid.'
      );
    case 'unsupported-version':
      return translate(
        t,
        'creativeStudio.workflows.validation.unsupportedVersion',
        'This template version is not supported.'
      );
    case 'unknown-field':
      return translate(
        t,
        'creativeStudio.workflows.validation.unknownField',
        'The template contains an unsupported field.'
      );
    case 'limit-exceeded':
      return translate(
        t,
        'creativeStudio.workflows.validation.limitExceeded',
        'The template exceeds a supported limit.'
      );
    case 'duplicate-id':
      return translate(
        t,
        'creativeStudio.workflows.validation.duplicateId',
        'The template contains duplicate identifiers.'
      );
    case 'broken-reference':
      return translate(
        t,
        'creativeStudio.workflows.validation.brokenReference',
        'The template contains a broken reference. Reopen or edit it.'
      );
    case 'cycle-detected':
      return translate(
        t,
        'creativeStudio.workflows.validation.cycleDetected',
        'The template contains a dependency cycle.'
      );
    case 'invalid-transition':
      return translate(
        t,
        'creativeStudio.workflows.validation.invalidTransition',
        "This change is not valid for the template's current state."
      );
    case 'invalid-value':
    default:
      return translate(
        t,
        'creativeStudio.workflows.validation.invalidValue',
        'Please check the template configuration.'
      );
  }
};

export function formatWorkflowValidationError(
  error: WorkflowValidationError,
  t: WorkflowTranslate
): string {
  return translate(
    t,
    'creativeStudio.workflows.validation.failed',
    'Template validation failed at {{path}}: {{detail}}',
    {
      path: error.path,
      detail: validationDetail(error, t),
    }
  );
}

export function formatWorkflowRuntimeError(
  detail: string | null | undefined,
  t: WorkflowTranslate,
  code?: WorkflowRunRuntimeErrorCode | string
): string {
  const message = detail?.trim() || '';
  if (code === 'task-cancelled') {
    return translate(
      t,
      'creativeStudio.workflows.runtime.taskCancelled',
      'The image-generation task was cancelled.'
    );
  }
  if (code === 'task-failed') {
    return translate(
      t,
      'creativeStudio.workflows.runtime.taskFailed',
      'Image generation failed: {{detail}}',
      { detail: message }
    );
  }
  if (code === 'planner-output') {
    return translate(
      t,
      'creativeStudio.workflows.runtime.plannerOutput',
      'Prompt planning returned an invalid result: {{detail}}',
      { detail: message }
    );
  }
  if (code === 'asset-response') {
    return translate(
      t,
      'creativeStudio.workflows.runtime.assetResponse',
      'The planner asset could not be read: {{detail}}',
      { detail: message }
    );
  }
  if (code === 'invalid-plan') {
    return translate(
      t,
      'creativeStudio.workflows.runtime.invalidPlan',
      'The template could not be prepared: {{detail}}',
      { detail: message }
    );
  }
  return translate(
    t,
    'creativeStudio.workflows.runtime.generic',
    'The template run encountered an error: {{detail}}',
    { detail: message }
  );
}

export function formatWorkflowLoadError(
  detail: string | null | undefined,
  t: WorkflowTranslate
): string {
  return translate(
    t,
    'creativeStudio.workflows.runtime.loadFailed',
    'Unable to load template runs: {{detail}}',
    { detail: detail?.trim() || '' }
  );
}

export function workflowFallbackError(
  error: unknown,
  t: WorkflowTranslate,
  key: string,
  defaultValue: string
): string {
  const detail = error instanceof Error ? error.message.trim() : '';
  if (!detail) return translate(t, key, defaultValue);
  return translate(
    t,
    'creativeStudio.workflows.errors.generic',
    'The template operation failed. Try again.'
  );
}

export type { TFunction };
