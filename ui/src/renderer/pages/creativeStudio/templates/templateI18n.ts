/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import type { TFunction } from 'i18next';

import type { CreativeTemplateValidationError } from './domain';
import type { CreativeTemplateRunRuntimeErrorCode } from './runtime';

export interface CreativeTemplateTranslationCopy {
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

type TemplateTranslate = (
  key: string,
  options?: Record<string, unknown>
) => string;

const fallbackTranslate: TemplateTranslate = (_key, options) =>
  typeof options?.defaultValue === 'string' ? options.defaultValue : '';

const translate = (
  t: TemplateTranslate,
  key: string,
  defaultValue: string,
  options: Record<string, unknown> = {}
): string => t(key, { ...options, defaultValue });

/**
 * The template domain stores display text in persisted definitions. Keep the
 * domain pure by resolving system-owned defaults at the UI boundary.
 */
export function createTemplateTranslationCopy(
  t: TemplateTranslate = fallbackTranslate
): CreativeTemplateTranslationCopy {
  return {
    planningInstruction: translate(
      t,
      'creativeStudio.templates.defaults.planningInstruction',
      'Give each image a distinct narrative role while keeping the subject, visual style, and publishing context consistent.'
    ),
    variableLabel: (ordinal) =>
      translate(t, 'creativeStudio.templates.defaults.variableLabel', 'Input {{ordinal}}', {
        ordinal,
      }),
    topicLabel: translate(t, 'creativeStudio.templates.defaults.topicLabel', 'Topic'),
    styleLabel: translate(t, 'creativeStudio.templates.defaults.styleLabel', 'Unified style'),
    platformLabel: translate(
      t,
      'creativeStudio.templates.defaults.platformLabel',
      'Publishing platform'
    ),
    productNameLabel: translate(
      t,
      'creativeStudio.templates.defaults.productNameLabel',
      'Product name'
    ),
    sellingPointsLabel: translate(
      t,
      'creativeStudio.templates.defaults.sellingPointsLabel',
      'Selling points',
    ),
    choiceOptionOne: translate(
      t,
      'creativeStudio.templates.defaults.choiceOptionOne',
      'Option one'
    ),
    choiceOptionTwo: translate(
      t,
      'creativeStudio.templates.defaults.choiceOptionTwo',
      'Option two'
    ),
    stepGenerate: translate(
      t,
      'creativeStudio.templates.defaults.stepGenerate',
      'Generate images'
    ),
    stepRecord: translate(
      t,
      'creativeStudio.templates.defaults.stepRecord',
      'Record results'
    ),
    stepPlan: translate(
      t,
      'creativeStudio.templates.defaults.stepPlan',
      'Plan multi-image prompts'
    ),
    stepBatchGenerate: translate(
      t,
      'creativeStudio.templates.defaults.stepBatchGenerate',
      'Generate images in batch'
    ),
    templateName: translate(
      t,
      'creativeStudio.templates.defaults.templateName',
      'Main prompt'
    ),
    singleName: translate(
      t,
      'creativeStudio.templates.defaults.singleName',
      'Untitled template'
    ),
    multiName: translate(
      t,
      'creativeStudio.templates.defaults.multiName',
      'Image series generator'
    ),
    multiDescription: translate(
      t,
      'creativeStudio.templates.defaults.multiDescription',
      'Generate a coherent set of image prompts from a topic, review them, then create the images in batch.'
    ),
    multiCategory: translate(
      t,
      'creativeStudio.templates.defaults.multiCategory',
      'Multi-image creation'
    ),
    singlePromptTemplate: translate(
      t,
      'creativeStudio.templates.defaults.singlePromptTemplate',
      'Create a premium e-commerce poster for {{product_name}}.\nKey selling points: {{selling_points}}'
    ),
    multiPromptTemplate: translate(
      t,
      'creativeStudio.templates.defaults.multiPromptTemplate',
      'Create a coherent image series about {{topic}}.\nUnified style: {{style}}\nPublishing platform: {{platform}}'
    ),
    duplicateSuffix: translate(
      t,
      'creativeStudio.templates.defaults.duplicateSuffix',
      'Copy'
    ),
    emptyPrompt: translate(
      t,
      'creativeStudio.templates.defaults.emptyPrompt',
      'Prompt template not filled in yet'
    ),
    outputSingle: translate(
      t,
      'creativeStudio.templates.output.single',
      'Single image'
    ),
    outputMulti: translate(
      t,
      'creativeStudio.templates.output.multi',
      'Multi-image series'
    ),
  };
}

const validationDetail = (
  error: CreativeTemplateValidationError,
  t: TemplateTranslate
): string => {
  const required = /^required input (.+) is missing$/u.exec(error.message);
  if (required) {
    return translate(
      t,
      'creativeStudio.templates.validation.requiredInput',
      'Please complete the required input “{{input}}”.',
      { input: required[1] }
    );
  }

  switch (error.code) {
    case 'invalid-json':
      return translate(
        t,
        'creativeStudio.templates.validation.invalidJson',
        'The template data is not valid JSON.'
      );
    case 'invalid-envelope':
      return translate(
        t,
        'creativeStudio.templates.validation.invalidEnvelope',
        'The template data format is invalid.'
      );
    case 'unsupported-version':
      return translate(
        t,
        'creativeStudio.templates.validation.unsupportedVersion',
        'This template version is not supported.'
      );
    case 'unknown-field':
      return translate(
        t,
        'creativeStudio.templates.validation.unknownField',
        'The template contains an unsupported field.'
      );
    case 'limit-exceeded':
      return translate(
        t,
        'creativeStudio.templates.validation.limitExceeded',
        'The template exceeds a supported limit.'
      );
    case 'duplicate-id':
      return translate(
        t,
        'creativeStudio.templates.validation.duplicateId',
        'The template contains duplicate identifiers.'
      );
    case 'broken-reference':
      return translate(
        t,
        'creativeStudio.templates.validation.brokenReference',
        'The template contains a broken reference. Reopen or edit it.'
      );
    case 'cycle-detected':
      return translate(
        t,
        'creativeStudio.templates.validation.cycleDetected',
        'The template contains a dependency cycle.'
      );
    case 'invalid-transition':
      return translate(
        t,
        'creativeStudio.templates.validation.invalidTransition',
        "This change is not valid for the template's current state."
      );
    case 'invalid-value':
    default:
      return translate(
        t,
        'creativeStudio.templates.validation.invalidValue',
        'Please check the template configuration.'
      );
  }
};

export function formatTemplateValidationError(
  error: CreativeTemplateValidationError,
  t: TemplateTranslate
): string {
  return translate(
    t,
    'creativeStudio.templates.validation.failed',
    'Template validation failed at {{path}}: {{detail}}',
    {
      path: error.path,
      detail: validationDetail(error, t),
    }
  );
}

export function formatTemplateRuntimeError(
  detail: string | null | undefined,
  t: TemplateTranslate,
  code?: CreativeTemplateRunRuntimeErrorCode | string
): string {
  const message = detail?.trim() || '';
  if (code === 'task-cancelled') {
    return translate(
      t,
      'creativeStudio.templates.runtime.taskCancelled',
      'The image-generation task was cancelled.'
    );
  }
  if (code === 'task-failed') {
    return translate(
      t,
      'creativeStudio.templates.runtime.taskFailed',
      'Image generation failed: {{detail}}',
      { detail: message }
    );
  }
  if (code === 'planner-output') {
    return translate(
      t,
      'creativeStudio.templates.runtime.plannerOutput',
      'Prompt planning returned an invalid result: {{detail}}',
      { detail: message }
    );
  }
  if (code === 'asset-response') {
    return translate(
      t,
      'creativeStudio.templates.runtime.assetResponse',
      'The planner asset could not be read: {{detail}}',
      { detail: message }
    );
  }
  if (code === 'invalid-plan') {
    return translate(
      t,
      'creativeStudio.templates.runtime.invalidPlan',
      'The template could not be prepared: {{detail}}',
      { detail: message }
    );
  }
  return translate(
    t,
    'creativeStudio.templates.runtime.generic',
    'The template run encountered an error: {{detail}}',
    { detail: message }
  );
}

export function formatTemplateLoadError(
  detail: string | null | undefined,
  t: TemplateTranslate
): string {
  return translate(
    t,
    'creativeStudio.templates.runtime.loadFailed',
    'Unable to load template runs: {{detail}}',
    { detail: detail?.trim() || '' }
  );
}

export function templateFallbackError(
  error: unknown,
  t: TemplateTranslate,
  key: string,
  defaultValue: string
): string {
  const detail = error instanceof Error ? error.message.trim() : '';
  if (!detail) return translate(t, key, defaultValue);
  return translate(
    t,
    'creativeStudio.templates.errors.generic',
    'The template operation failed. Try again.'
  );
}

export type { TFunction };
