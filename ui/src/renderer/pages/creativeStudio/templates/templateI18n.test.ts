/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { describe, expect, test } from 'bun:test';

import { validateTemplateDefinition } from './domain';
import {
  createTemplateTranslationCopy,
  formatTemplateRuntimeError,
  formatTemplateValidationError,
  templateFallbackError,
} from './templateI18n';
import {
  createBlankTemplate,
  creativePromptTemplateText,
} from './page/templateViewModel';

const translate = (
  key: string,
  options?: Record<string, unknown>
): string => {
  const defaults: Record<string, string> = {
    'creativeStudio.templates.defaults.singleName': 'Localized template',
    'creativeStudio.templates.defaults.productNameLabel': 'Localized product',
    'creativeStudio.templates.defaults.sellingPointsLabel': 'Localized benefits',
    'creativeStudio.templates.defaults.stepGenerate': 'Localized generate',
    'creativeStudio.templates.defaults.stepRecord': 'Localized record',
    'creativeStudio.templates.defaults.singlePromptTemplate':
      'Make {{product_name}} with {{selling_points}}',
    'creativeStudio.templates.validation.failed':
      'Validation failed at {{path}}: {{detail}}',
    'creativeStudio.templates.validation.invalidValue': 'Check the configuration.',
    'creativeStudio.templates.runtime.taskFailed':
      'Image task failed: {{detail}}',
    'creativeStudio.templates.errors.generic':
      'The template operation failed. Try again.',
  };
  let value = defaults[key] ?? String(options?.defaultValue ?? key);
  for (const [name, replacement] of Object.entries(options ?? {})) {
    if (name === 'defaultValue') continue;
    value = value.replaceAll(`{{${name}}}`, String(replacement));
  }
  return value;
};

describe('template translation boundary', () => {
  test('localizes system-owned template and node defaults before persistence', () => {
    const copy = createTemplateTranslationCopy(translate);
    const template = createBlankTemplate('single-image', copy);

    expect(validateTemplateDefinition(template).ok).toBe(true);
    expect(template.metadata.name).toBe('Localized template');
    expect(template.variables.map((variable) => variable.label)).toEqual([
      'Localized product',
      'Localized benefits',
    ]);
    expect(template.steps.map((step) => step.name)).toEqual([
      'Localized generate',
      'Localized record',
    ]);
    expect(creativePromptTemplateText(template)).toBe(
      'Make {{product_name}} with {{selling_points}}'
    );
  });

  test('keeps validation and runtime details behind semantic translation keys', () => {
    expect(
      formatTemplateValidationError(
        {
          code: 'invalid-value',
          path: '$.metadata',
          message: 'invalid',
        },
        translate
      )
    ).toBe('Validation failed at $.metadata: Check the configuration.');
    expect(
      formatTemplateRuntimeError('provider rejected task', translate, 'task-failed')
    ).toBe('Image task failed: provider rejected task');
    expect(
      templateFallbackError(
        new Error('fixed English protocol detail'),
        translate,
        'creativeStudio.templates.workspace.loadError',
        'Failed to load templates'
      )
    ).toBe('The template operation failed. Try again.');
  });
});
