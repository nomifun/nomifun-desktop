/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { describe, expect, test } from 'bun:test';
import { validateTemplateDefinition } from '../domain';
import { createTemplateFixture } from '../domain/testFixtures';
import {
  createBlankTemplate,
  duplicateTemplate,
  parseCreativePromptTemplateText,
  switchTemplateMode,
  withPrivateTemplateVisibility,
  creativePromptTemplateText,
} from './templateViewModel';

describe('template page view model', () => {
  test('creates valid single and multi-image definitions', () => {
    expect(validateTemplateDefinition(createBlankTemplate('single-image')).ok).toBe(true);
    expect(validateTemplateDefinition(createBlankTemplate('multi-image-series')).ok).toBe(true);
  });

  test('round trips known structured template variables without interpolating strings', () => {
    const template = createBlankTemplate('single-image');
    const text = creativePromptTemplateText(template);
    expect(text.includes('{{product_name}}')).toBe(true);
    expect(parseCreativePromptTemplateText(text, template.variables)).toEqual(
      template.templates[0].segments
    );
  });

  test('switches the DAG and duplicates every owned UUIDv7 reference', () => {
    const single = createBlankTemplate('single-image');
    const multi = switchTemplateMode(single, 'multi-image-series');
    expect(validateTemplateDefinition(multi).ok).toBe(true);
    expect(multi.steps.map((step) => step.kind)).toEqual([
      'draft-prompts',
      'generate-images',
      'record-history',
    ]);

    const copy = duplicateTemplate(multi);
    expect(validateTemplateDefinition(copy).ok).toBe(true);
    expect(copy.id).not.toBe(multi.id);
    expect(copy.variables.map((variable) => variable.id)).not.toEqual(
      multi.variables.map((variable) => variable.id)
    );
    expect(copy.templates.map((promptTemplate) => promptTemplate.id)).not.toEqual(
      multi.templates.map((promptTemplate) => promptTemplate.id)
    );
    expect(copy.steps.map((step) => step.id)).not.toEqual(multi.steps.map((step) => step.id));
  });

  test('normalizes legacy and copied templates to private without mutating the source', () => {
    const legacyPublic = createTemplateFixture();
    legacyPublic.metadata.visibility = 'public';

    const normalized = withPrivateTemplateVisibility(legacyPublic);
    const copy = duplicateTemplate(legacyPublic);

    expect(normalized.metadata.visibility).toBe('private');
    expect(copy.metadata.visibility).toBe('private');
    expect(legacyPublic.metadata.visibility).toBe('public');
    expect(normalized.metadata).not.toBe(legacyPublic.metadata);
  });
});
