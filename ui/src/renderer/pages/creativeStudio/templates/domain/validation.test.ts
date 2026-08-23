/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { describe, expect, test } from 'bun:test';

import { topologicallySortTemplateSteps } from './graph';
import {
  createTemplateDefaultInputs,
  createTemplateWorkspaceDocumentV1,
  renderCreativePromptTemplate,
} from './model';
import { exportTemplateWorkspaceV1, parseTemplateWorkspaceV1 } from './serialization';
import { IDS, createTemplateFixture } from './testFixtures';
import {
  isTemplateBusinessId,
  validateTemplateDefinition,
  validateTemplateInputsForDefinition,
  validateTemplateWorkspaceDocument,
} from './validation';

describe('template v1 validation and parser', () => {
  test('accepts canonical single-image and multi-image DAGs', () => {
    expect(validateTemplateDefinition(createTemplateFixture()).ok).toBe(true);
    expect(validateTemplateDefinition(createTemplateFixture(true)).ok).toBe(true);
    const sorted = topologicallySortTemplateSteps(createTemplateFixture(true));
    expect(sorted.ok).toBe(true);
    if (sorted.ok) {
      expect(sorted.value.map((step) => step.id)).toEqual([
        IDS.draftStep,
        IDS.generateStep,
        IDS.historyStep,
      ]);
    }
  });

  test('requires bare lowercase UUIDv7 business identities', () => {
    expect(isTemplateBusinessId(IDS.template)).toBe(true);
    expect(isTemplateBusinessId(IDS.template.toUpperCase())).toBe(false);
    expect(isTemplateBusinessId(`template_${IDS.template}`)).toBe(false);
    const template = createTemplateFixture();
    template.id = 'legacy-template';
    const result = validateTemplateDefinition(template);
    expect(result.ok).toBe(false);
    if (!result.ok) expect(result.error.path).toBe('$.id');
  });

  test('rejects cycles, broken dependencies, and image interpolation', () => {
    const cyclic = createTemplateFixture(true);
    cyclic.steps[0].dependsOn = [IDS.generateStep];
    const cycleResult = validateTemplateDefinition(cyclic);
    expect(cycleResult.ok).toBe(false);
    if (!cycleResult.ok) expect(cycleResult.error.code).toBe('cycle-detected');

    const broken = createTemplateFixture();
    broken.steps[0].dependsOn = ['018f0000-0000-7000-8000-000000000099'];
    expect(validateTemplateDefinition(broken).ok).toBe(false);

    const unsafeTemplate = createTemplateFixture();
    unsafeTemplate.templates[0].segments.push({
      kind: 'variable',
      variableId: IDS.imageVariable,
    });
    const unsafeResult = validateTemplateDefinition(unsafeTemplate);
    expect(unsafeResult.ok).toBe(false);
    if (!unsafeResult.ok) expect(unsafeResult.error.message.includes('image inputs')).toBe(true);
  });

  test('keeps image generation settings provider-scoped and task-exact', () => {
    const template = createTemplateFixture();
    const generate = template.steps.find((step) => step.kind === 'generate-images');
    if (!generate || generate.kind !== 'generate-images') throw new Error('missing generation step');
    generate.generation.model = {
      providerId: IDS.history,
      model: 'gpt-image-test',
      task: 'image_generation',
    };
    const mismatch = validateTemplateDefinition(template);
    expect(mismatch.ok).toBe(false);
    if (!mismatch.ok) expect(mismatch.error.path.includes('generation.model.task')).toBe(true);

    generate.generation.model.task = 'image_edit';
    generate.generation.width = 1025;
    const dimensions = validateTemplateDefinition(template);
    expect(dimensions.ok).toBe(false);
    if (!dimensions.ok) expect(dimensions.error.path.includes('generation.width')).toBe(true);
  });

  test('requires an exact chat binding for prompt planning settings', () => {
    const template = createTemplateFixture(true);
    const planner = template.steps.find((step) => step.kind === 'draft-prompts');
    if (!planner || planner.kind !== 'draft-prompts') throw new Error('missing prompt planner');
    planner.planning.model = {
      providerId: IDS.history,
      model: 'chat-test',
      task: 'chat',
    };
    expect(validateTemplateDefinition(template).ok).toBe(true);

    planner.planning.maxTokens = 0;
    const invalidTokens = validateTemplateDefinition(template);
    expect(invalidTokens.ok).toBe(false);
    if (!invalidTokens.ok) {
      expect(invalidTokens.error.path.includes('planning.maxTokens')).toBe(true);
    }
  });

  test('validates typed inputs and renders structured templates without string substitution', () => {
    const template = createTemplateFixture();
    const missing = validateTemplateInputsForDefinition(template, []);
    expect(missing.ok).toBe(false);
    const inputs = [
      { variableId: IDS.variable, type: 'text' as const, value: 'NomiFun' },
      { variableId: IDS.imageVariable, type: 'image' as const, assetId: IDS.asset },
    ];
    expect(validateTemplateInputsForDefinition(template, inputs).ok).toBe(true);
    const rendered = renderCreativePromptTemplate(
      template,
      IDS.promptTemplate,
      inputs
    );
    expect(rendered).toEqual({ ok: true, value: 'Create a poster for NomiFun' });
    expect(createTemplateDefaultInputs(template)).toEqual([]);
  });

  test('fails closed on malformed JSON, versions, and unknown nested fields', () => {
    expect(parseTemplateWorkspaceV1('{')).toMatchObject({
      ok: false,
      error: { code: 'invalid-json' },
    });
    const empty = createTemplateWorkspaceDocumentV1();
    expect(parseTemplateWorkspaceV1(JSON.stringify({ ...empty, version: 2 }))).toMatchObject({
      ok: false,
      error: { code: 'unsupported-version' },
    });
    const template = createTemplateFixture();
    const document = { ...empty, templates: [{ ...template, unexpectedExtension: {} }] };
    const parsed = parseTemplateWorkspaceV1(JSON.stringify(document));
    expect(parsed.ok).toBe(false);
    if (!parsed.ok) {
      expect(parsed.error.code).toBe('unknown-field');
      expect(parsed.error.path.includes('unexpectedExtension')).toBe(true);
    }
  });

  test('round trips only validated v1 JSON', () => {
    const document = {
      ...createTemplateWorkspaceDocumentV1(),
      templates: [createTemplateFixture()],
    };
    expect(validateTemplateWorkspaceDocument(document).ok).toBe(true);
    const exported = exportTemplateWorkspaceV1(document);
    expect(exported.ok).toBe(true);
    if (!exported.ok) return;
    const parsed = parseTemplateWorkspaceV1(exported.json);
    expect(parsed).toEqual({ ok: true, document });
  });
});
