/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { describe, expect, test } from 'bun:test';

import { parseProviderId } from '@/common/types/ids';

import { CreativeStudioContractError } from '../../domain';
import { validateWorkflowDefinition } from '../domain';

import {
  CREATIVE_WORKFLOW_DRAFT_ARTIFACT_KIND,
  type CreativeWorkflowDraftArtifact,
} from './artifacts';
import { convertCreativeWorkflowDraft } from './converter';

const PROVIDER_ID = parseProviderId('0190f5fe-7c00-7a00-8000-000000000901');
const chatModel = { providerId: PROVIDER_ID, model: 'nomi-chat' } as const;
const UUID_V7 =
  /^[0-9a-f]{8}-[0-9a-f]{4}-7[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/u;

const artifact = (
  mode: 'single-image' | 'multi-image-series',
  promptTemplate: string
): CreativeWorkflowDraftArtifact => ({
  kind: CREATIVE_WORKFLOW_DRAFT_ARTIFACT_KIND,
  summary: '草稿已生成',
  draft: {
    mode,
    name: mode === 'single-image' ? '商品主图' : '社媒多图',
    description: '最小可上线工作流',
    category: '营销',
    promptTemplate,
  },
});

const renderTemplateKeys = (
  workflow: ReturnType<typeof convertCreativeWorkflowDraft>
): string => {
  const keys = new Map(workflow.variables.map((variable) => [variable.id, variable.key]));
  return workflow.templates[0].segments
    .map((segment) =>
      segment.kind === 'text'
        ? segment.text
        : `{{${keys.get(segment.variableId) ?? 'missing'}}}`
    )
    .join('');
};

const expectConversionFailure = (
  value: CreativeWorkflowDraftArtifact
): CreativeStudioContractError => {
  try {
    convertCreativeWorkflowDraft(value, chatModel);
    throw new Error('Expected workflow draft conversion failure');
  } catch (error) {
    expect(error instanceof CreativeStudioContractError).toBe(true);
    return error as CreativeStudioContractError;
  }
};

describe('Creative Workflow Agent draft converter', () => {
  test('builds a valid private single-image definition from the fixed blank template', () => {
    const prompt = '为 {{product_name}} 生成主图，突出 {{selling_points}}。';
    const workflow = convertCreativeWorkflowDraft(artifact('single-image', prompt), chatModel);

    expect(validateWorkflowDefinition(workflow).ok).toBe(true);
    expect(UUID_V7.test(workflow.id)).toBe(true);
    expect(workflow.revision).toBe(1);
    expect(workflow.metadata).toEqual({
      name: '商品主图',
      description: '最小可上线工作流',
      category: '营销',
      visibility: 'private',
      tags: [],
      createdAt: 0,
      updatedAt: 0,
    });
    expect(workflow.variables.map((variable) => variable.key)).toEqual([
      'product_name',
      'selling_points',
    ]);
    expect(renderTemplateKeys(workflow)).toBe(prompt);
    expect(
      workflow.steps.find((step) => step.kind === 'generate-images')?.generation.model
    ).toBeNull();
    expect(workflow.steps.some((step) => step.kind === 'draft-prompts')).toBe(false);
    expect(JSON.stringify(workflow).includes(chatModel.model)).toBe(false);
  });

  test('injects only the exact turn Chat model into multi-image prompt planning', () => {
    const prompt =
      '围绕 {{ topic }} 规划一组图片，风格为 {{style}}，适配 {{platform}}。';
    const workflow = convertCreativeWorkflowDraft(
      artifact('multi-image-series', prompt),
      chatModel
    );

    expect(validateWorkflowDefinition(workflow).ok).toBe(true);
    expect(workflow.variables.map((variable) => variable.key)).toEqual([
      'topic',
      'style',
      'platform',
    ]);
    expect(renderTemplateKeys(workflow)).toBe(
      '围绕 {{topic}} 规划一组图片，风格为 {{style}}，适配 {{platform}}。'
    );
    const planner = workflow.steps.find((step) => step.kind === 'draft-prompts');
    expect(planner?.planning.model).toEqual({
      providerId: PROVIDER_ID,
      model: 'nomi-chat',
      task: 'chat',
    });
    expect(
      workflow.steps.find((step) => step.kind === 'generate-images')?.generation.model
    ).toBeNull();
  });

  test('rejects missing, unknown, unclosed, nested and unmatched placeholders', () => {
    expectConversionFailure(
      artifact('single-image', '生成一张没有变量的固定主图')
    );
    expectConversionFailure(
      artifact('single-image', '为 {{unknown}} 生成主图')
    );
    expectConversionFailure(
      artifact('single-image', '为 {{product_name 生成主图')
    );
    expectConversionFailure(
      artifact('single-image', '为 {{product_{{name}} 生成主图')
    );
    expectConversionFailure(
      artifact('single-image', '为 product_name}} 生成主图')
    );
  });

  test('runs final workflow validation before returning the definition', () => {
    const error = expectConversionFailure(
      artifact('single-image', '{{product_name}}'.repeat(501))
    );
    expect(error.path).toBe('$.templates[0].segments');
  });
});
