/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { describe, expect, test } from 'bun:test';

import { validateWorkflowDefinition } from './domain';
import {
  createWorkflowTranslationCopy,
  formatWorkflowRuntimeError,
  formatWorkflowValidationError,
  workflowFallbackError,
} from './workflowI18n';
import {
  createBlankWorkflow,
  workflowTemplateText,
} from './page/workflowViewModel';

const translate = (
  key: string,
  options?: Record<string, unknown>
): string => {
  const defaults: Record<string, string> = {
    'creativeStudio.workflows.defaults.singleName': 'Localized template',
    'creativeStudio.workflows.defaults.productNameLabel': 'Localized product',
    'creativeStudio.workflows.defaults.sellingPointsLabel': 'Localized benefits',
    'creativeStudio.workflows.defaults.stepGenerate': 'Localized generate',
    'creativeStudio.workflows.defaults.stepRecord': 'Localized record',
    'creativeStudio.workflows.defaults.singlePromptTemplate':
      'Make {{product_name}} with {{selling_points}}',
    'creativeStudio.workflows.validation.failed':
      'Validation failed at {{path}}: {{detail}}',
    'creativeStudio.workflows.validation.invalidValue': 'Check the configuration.',
    'creativeStudio.workflows.runtime.taskFailed':
      'Image task failed: {{detail}}',
    'creativeStudio.workflows.errors.generic':
      'The template operation failed. Try again.',
  };
  let value = defaults[key] ?? String(options?.defaultValue ?? key);
  for (const [name, replacement] of Object.entries(options ?? {})) {
    if (name === 'defaultValue') continue;
    value = value.replaceAll(`{{${name}}}`, String(replacement));
  }
  return value;
};

describe('workflow translation boundary', () => {
  test('localizes system-owned template and node defaults before persistence', () => {
    const copy = createWorkflowTranslationCopy(translate);
    const workflow = createBlankWorkflow('single-image', copy);

    expect(validateWorkflowDefinition(workflow).ok).toBe(true);
    expect(workflow.metadata.name).toBe('Localized template');
    expect(workflow.variables.map((variable) => variable.label)).toEqual([
      'Localized product',
      'Localized benefits',
    ]);
    expect(workflow.steps.map((step) => step.name)).toEqual([
      'Localized generate',
      'Localized record',
    ]);
    expect(workflowTemplateText(workflow)).toBe(
      'Make {{product_name}} with {{selling_points}}'
    );
  });

  test('keeps validation and runtime details behind semantic translation keys', () => {
    expect(
      formatWorkflowValidationError(
        {
          code: 'invalid-value',
          path: '$.metadata',
          message: 'invalid',
        },
        translate
      )
    ).toBe('Validation failed at $.metadata: Check the configuration.');
    expect(
      formatWorkflowRuntimeError('provider rejected task', translate, 'task-failed')
    ).toBe('Image task failed: provider rejected task');
    expect(
      workflowFallbackError(
        new Error('fixed English protocol detail'),
        translate,
        'creativeStudio.workflows.workspace.loadError',
        'Failed to load templates'
      )
    ).toBe('The template operation failed. Try again.');
  });
});
