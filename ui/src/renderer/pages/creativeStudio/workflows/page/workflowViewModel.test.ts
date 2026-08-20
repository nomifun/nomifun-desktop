/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { describe, expect, test } from 'bun:test';
import { validateWorkflowDefinition } from '../domain';
import {
  createBlankWorkflow,
  duplicateWorkflow,
  parseWorkflowTemplateText,
  switchWorkflowMode,
  workflowTemplateText,
} from './workflowViewModel';

describe('workflow page view model', () => {
  test('creates valid single and multi-image definitions', () => {
    expect(validateWorkflowDefinition(createBlankWorkflow('single-image')).ok).toBe(true);
    expect(validateWorkflowDefinition(createBlankWorkflow('multi-image-series')).ok).toBe(true);
  });

  test('round trips known structured template variables without interpolating strings', () => {
    const workflow = createBlankWorkflow('single-image');
    const text = workflowTemplateText(workflow);
    expect(text.includes('{{product_name}}')).toBe(true);
    expect(parseWorkflowTemplateText(text, workflow.variables)).toEqual(
      workflow.templates[0].segments
    );
  });

  test('switches the DAG and duplicates every owned UUIDv7 reference', () => {
    const single = createBlankWorkflow('single-image');
    const multi = switchWorkflowMode(single, 'multi-image-series');
    expect(validateWorkflowDefinition(multi).ok).toBe(true);
    expect(multi.steps.map((step) => step.kind)).toEqual([
      'draft-prompts',
      'generate-images',
      'record-history',
    ]);

    const copy = duplicateWorkflow(multi);
    expect(validateWorkflowDefinition(copy).ok).toBe(true);
    expect(copy.id).not.toBe(multi.id);
    expect(copy.variables.map((variable) => variable.id)).not.toEqual(
      multi.variables.map((variable) => variable.id)
    );
    expect(copy.templates.map((template) => template.id)).not.toEqual(
      multi.templates.map((template) => template.id)
    );
    expect(copy.steps.map((step) => step.id)).not.toEqual(multi.steps.map((step) => step.id));
  });
});
