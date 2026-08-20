/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { describe, expect, test } from 'bun:test';

import { topologicallySortWorkflowSteps } from './graph';
import {
  createWorkflowDefaultInputs,
  createWorkflowWorkspaceDocumentV1,
  renderWorkflowTemplate,
} from './model';
import { exportWorkflowWorkspaceV1, parseWorkflowWorkspaceV1 } from './serialization';
import { IDS, createWorkflowFixture } from './testFixtures';
import {
  isWorkflowBusinessId,
  validateWorkflowDefinition,
  validateWorkflowInputsForDefinition,
  validateWorkflowWorkspaceDocument,
} from './validation';

describe('workflow v1 validation and parser', () => {
  test('accepts canonical single-image and multi-image DAGs', () => {
    expect(validateWorkflowDefinition(createWorkflowFixture()).ok).toBe(true);
    expect(validateWorkflowDefinition(createWorkflowFixture(true)).ok).toBe(true);
    const sorted = topologicallySortWorkflowSteps(createWorkflowFixture(true));
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
    expect(isWorkflowBusinessId(IDS.workflow)).toBe(true);
    expect(isWorkflowBusinessId(IDS.workflow.toUpperCase())).toBe(false);
    expect(isWorkflowBusinessId(`workflow_${IDS.workflow}`)).toBe(false);
    const workflow = createWorkflowFixture();
    workflow.id = 'legacy-workflow';
    const result = validateWorkflowDefinition(workflow);
    expect(result.ok).toBe(false);
    if (!result.ok) expect(result.error.path).toBe('$.id');
  });

  test('rejects cycles, broken dependencies, and image interpolation', () => {
    const cyclic = createWorkflowFixture(true);
    cyclic.steps[0].dependsOn = [IDS.generateStep];
    const cycleResult = validateWorkflowDefinition(cyclic);
    expect(cycleResult.ok).toBe(false);
    if (!cycleResult.ok) expect(cycleResult.error.code).toBe('cycle-detected');

    const broken = createWorkflowFixture();
    broken.steps[0].dependsOn = ['018f0000-0000-7000-8000-000000000099'];
    expect(validateWorkflowDefinition(broken).ok).toBe(false);

    const unsafeTemplate = createWorkflowFixture();
    unsafeTemplate.templates[0].segments.push({
      kind: 'variable',
      variableId: IDS.imageVariable,
    });
    const unsafeResult = validateWorkflowDefinition(unsafeTemplate);
    expect(unsafeResult.ok).toBe(false);
    if (!unsafeResult.ok) expect(unsafeResult.error.message.includes('image inputs')).toBe(true);
  });

  test('validates typed inputs and renders structured templates without string substitution', () => {
    const workflow = createWorkflowFixture();
    const missing = validateWorkflowInputsForDefinition(workflow, []);
    expect(missing.ok).toBe(false);
    const inputs = [
      { variableId: IDS.variable, type: 'text' as const, value: 'NomiFun' },
      { variableId: IDS.imageVariable, type: 'image' as const, assetId: IDS.asset },
    ];
    expect(validateWorkflowInputsForDefinition(workflow, inputs).ok).toBe(true);
    const rendered = renderWorkflowTemplate(workflow, IDS.template, inputs);
    expect(rendered).toEqual({ ok: true, value: 'Create a poster for NomiFun' });
    expect(createWorkflowDefaultInputs(workflow)).toEqual([]);
  });

  test('fails closed on malformed JSON, versions, and unknown nested fields', () => {
    expect(parseWorkflowWorkspaceV1('{')).toMatchObject({
      ok: false,
      error: { code: 'invalid-json' },
    });
    const empty = createWorkflowWorkspaceDocumentV1();
    expect(parseWorkflowWorkspaceV1(JSON.stringify({ ...empty, version: 2 }))).toMatchObject({
      ok: false,
      error: { code: 'unsupported-version' },
    });
    const workflow = createWorkflowFixture();
    const document = { ...empty, workflows: [{ ...workflow, unexpectedExtension: {} }] };
    const parsed = parseWorkflowWorkspaceV1(JSON.stringify(document));
    expect(parsed.ok).toBe(false);
    if (!parsed.ok) {
      expect(parsed.error.code).toBe('unknown-field');
      expect(parsed.error.path.includes('unexpectedExtension')).toBe(true);
    }
  });

  test('round trips only validated v1 JSON', () => {
    const document = {
      ...createWorkflowWorkspaceDocumentV1(),
      workflows: [createWorkflowFixture()],
    };
    expect(validateWorkflowWorkspaceDocument(document).ok).toBe(true);
    const exported = exportWorkflowWorkspaceV1(document);
    expect(exported.ok).toBe(true);
    if (!exported.ok) return;
    const parsed = parseWorkflowWorkspaceV1(exported.json);
    expect(parsed).toEqual({ ok: true, document });
  });
});
